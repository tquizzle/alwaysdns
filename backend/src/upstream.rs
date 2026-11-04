//! DNS-over-HTTPS and DNS-over-QUIC forwarding. Each upstream URL gets its own
//! resolver with a single name server, so per-server health and latency can be tracked.
//! Hostnames in upstream URLs are resolved through the bootstrap servers.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hickory_proto::op::ResponseCode;
use hickory_proto::rr::{Name, Record, RecordType};
use hickory_resolver::TokioResolver;
use hickory_resolver::config::{
    NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts,
};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError, NetError};
use tracing::{debug, warn};
use url::Url;

use crate::models::UpstreamStat;

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// How often every configured upstream is probed. Probes keep latency and health
/// current for servers that get no client traffic.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(60);

/// Names probed in turn with A queries. Popular names keep each probe on a
/// realistic answer path.
pub const PROBE_NAMES: [&str; 5] = [
    "google.com",
    "apple.com",
    "yahoo.com",
    "cloudflare.com",
    "wikipedia.org",
];

/// Successful probe round trips kept per upstream: one hour at one per minute.
const PROBE_WINDOW: usize = 60;

/// Consecutive failures, live or probe, before an upstream shows as down.
pub const DOWN_AFTER: u32 = 3;

/// A successful forwarded answer.
pub struct ForwardOk {
    pub answers: Vec<Record>,
    pub rcode: ResponseCode,
    pub upstream: String,
}

#[derive(Default, Clone)]
struct ServerStats {
    /// Live client queries only. Probes are not counted here.
    successes: u64,
    /// Live and probe failures together.
    errors: u64,
    /// Live client queries only.
    sum_ms: f64,
    /// Reset by any success, live or probe.
    failures_in_row: u32,
    /// Round trips of recent successful probes, in ms, oldest first.
    probe_ms: VecDeque<f64>,
}

#[derive(Default)]
struct State {
    bootstrap: Vec<IpAddr>,
    bootstrap_resolver: Option<Arc<TokioResolver>>,
    /// Per upstream URL. Dropped on failure so the next query re-resolves the host.
    doh: HashMap<String, Arc<TokioResolver>>,
    stats: BTreeMap<String, ServerStats>,
}

#[derive(Default)]
pub struct UpstreamManager {
    state: Mutex<State>,
}

impl UpstreamManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates bootstrap servers. A change clears resolver state, but keeps stats.
    pub fn set_bootstrap(&self, ips: Vec<IpAddr>) {
        if let Ok(mut st) = self.state.lock() {
            if st.bootstrap != ips {
                st.bootstrap = ips;
                st.bootstrap_resolver = None;
                st.doh.clear();
            }
        }
    }

    /// Tries each server in order and returns the first that answers. NXDOMAIN
    /// and NODATA count as answers. Errors and timeouts move on to the next server.
    pub async fn forward(
        &self,
        servers: &[String],
        name: &str,
        qtype: RecordType,
    ) -> Result<ForwardOk, String> {
        let mut last_error = String::from("no upstream servers configured");
        for url in servers {
            let started = Instant::now();
            match self.query_server(url, name, qtype).await {
                Ok((answers, rcode)) => {
                    self.note(url, true, started.elapsed());
                    return Ok(ForwardOk {
                        answers,
                        rcode,
                        upstream: url.clone(),
                    });
                }
                Err(err) => {
                    warn!(upstream = %url, %name, error = %err, "upstream query failed");
                    self.note(url, false, started.elapsed());
                    last_error = format!("{url}: {err}");
                }
            }
        }
        Err(last_error)
    }

    /// Snapshot for the stats API. `urls` are all configured servers, so
    /// servers that have not been used yet still appear.
    pub fn snapshot(&self, urls: &[String]) -> Vec<UpstreamStat> {
        let st = match self.state.lock() {
            Ok(st) => st,
            Err(_) => return Vec::new(),
        };
        urls.iter()
            .map(|url| {
                let s = st.stats.get(url).cloned().unwrap_or_default();
                UpstreamStat {
                    name: url.clone(),
                    ok: s.failures_in_row < DOWN_AFTER,
                    avg_ms: if s.successes > 0 {
                        ((s.sum_ms / s.successes as f64) * 1000.0).round() / 1000.0
                    } else {
                        0.0
                    },
                    errors: s.errors,
                    probe_p50_ms: percentile(&s.probe_ms, 50.0),
                    probe_p95_ms: percentile(&s.probe_ms, 95.0),
                    probe_samples: s.probe_ms.len() as u32,
                }
            })
            .collect()
    }

    async fn query_server(
        &self,
        url: &str,
        name: &str,
        qtype: RecordType,
    ) -> Result<(Vec<Record>, ResponseCode), String> {
        let resolver = self.resolver_for(url).await?;
        let fqdn = Name::from_ascii(format!("{name}.")).map_err(|e| e.to_string())?;
        match resolver.lookup(fqdn, qtype).await {
            Ok(lookup) => Ok((lookup.answers().to_vec(), ResponseCode::NoError)),
            Err(NetError::Dns(DnsError::NoRecordsFound(nr)))
                if nr.response_code == ResponseCode::NoError
                    || nr.response_code == ResponseCode::NXDomain =>
            {
                Ok((Vec::new(), nr.response_code))
            }
            Err(NetError::Dns(DnsError::ResponseCode(rc)))
                if rc == ResponseCode::NXDomain =>
            {
                Ok((Vec::new(), rc))
            }
            Err(other) => {
                self.forget(url);
                Err(other.to_string())
            }
        }
    }

    async fn resolver_for(&self, url: &str) -> Result<Arc<TokioResolver>, String> {
        if let Some(existing) = self.cached_resolver(url) {
            return Ok(existing);
        }
        let parsed = Url::parse(url).map_err(|e| e.to_string())?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "upstream URL has no host".to_string())?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let addr = match host.parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) => self.bootstrap_lookup(&host).await?,
        };
        let resolver = Arc::new(match parsed.scheme() {
            "quic" => build_quic_resolver(addr, &host, parsed.port().unwrap_or(853))?,
            _ => {
                let path = if parsed.path().is_empty() {
                    "/dns-query".to_string()
                } else {
                    parsed.path().to_string()
                };
                build_doh_resolver(addr, &host, parsed.port().unwrap_or(443), &path)?
            }
        });

        let mut st = self.state.lock().map_err(|_| "upstream state poisoned".to_string())?;
        Ok(st
            .doh
            .entry(url.to_string())
            .or_insert(resolver)
            .clone())
    }

    fn cached_resolver(&self, url: &str) -> Option<Arc<TokioResolver>> {
        self.state.lock().ok()?.doh.get(url).cloned()
    }

    fn forget(&self, url: &str) {
        if let Ok(mut st) = self.state.lock() {
            st.doh.remove(url);
        }
    }

    async fn bootstrap_lookup(&self, host: &str) -> Result<IpAddr, String> {
        let existing = {
            let st = self.state.lock().map_err(|_| "upstream state poisoned".to_string())?;
            if st.bootstrap.is_empty() {
                return Err(format!("'{host}' needs bootstrap servers, none configured"));
            }
            st.bootstrap_resolver.clone()
        };
        let resolver = match existing {
            Some(r) => r,
            None => {
                let ips = self
                    .state
                    .lock()
                    .map_err(|_| "upstream state poisoned".to_string())?
                    .bootstrap
                    .clone();
                let config = ResolverConfig::from_parts(
                    None,
                    Vec::new(),
                    ips.into_iter().map(NameServerConfig::udp).collect(),
                );
                let built = Arc::new(
                    build_resolver(config, false).map_err(|e| format!("bootstrap resolver: {e}"))?,
                );
                if let Ok(mut st) = self.state.lock() {
                    st.bootstrap_resolver = Some(built.clone());
                }
                built
            }
        };

        let lookup = resolver
            .lookup_ip(host)
            .await
            .map_err(|e| format!("bootstrap lookup of {host} failed: {e}"))?;
        let addrs: Vec<IpAddr> = lookup.iter().collect();
        addrs
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| addrs.first())
            .copied()
            .ok_or_else(|| format!("bootstrap returned no addresses for {host}"))
    }

    /// Sends one probe query straight to an upstream. Probes skip the DNS cache,
    /// the blocklists and the query log, so they only feed the upstream stats.
    pub async fn probe(&self, url: &str, name: &str) {
        let started = Instant::now();
        let ok = match self.query_server(url, name, RecordType::A).await {
            Ok(_) => true,
            Err(err) => {
                debug!(upstream = %url, %name, error = %err, "upstream probe failed");
                false
            }
        };
        self.note_probe(url, ok, started.elapsed());
    }

    fn note(&self, url: &str, ok: bool, elapsed: Duration) {
        if let Ok(mut st) = self.state.lock() {
            let entry = st.stats.entry(url.to_string()).or_default();
            record_health(entry, ok);
            if ok {
                entry.successes += 1;
                entry.sum_ms += elapsed.as_secs_f64() * 1000.0;
            } else {
                entry.errors += 1;
            }
        }
    }

    fn note_probe(&self, url: &str, ok: bool, elapsed: Duration) {
        if let Ok(mut st) = self.state.lock() {
            let entry = st.stats.entry(url.to_string()).or_default();
            record_health(entry, ok);
            if ok {
                if entry.probe_ms.len() == PROBE_WINDOW {
                    entry.probe_ms.pop_front();
                }
                entry.probe_ms.push_back(elapsed.as_secs_f64() * 1000.0);
            } else {
                entry.errors += 1;
            }
        }
    }
}

fn record_health(entry: &mut ServerStats, ok: bool) {
    entry.failures_in_row = if ok {
        0
    } else {
        entry.failures_in_row.saturating_add(1)
    };
}

/// Nearest-rank percentile of the samples, rounded to 3 decimals. None when empty.
fn percentile(samples: &VecDeque<f64>, p: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = samples.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let rank = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    let value = sorted[rank.min(sorted.len() - 1)];
    Some((value * 1000.0).round() / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_pick_the_nearest_rank() {
        let samples: VecDeque<f64> = (1..=5).map(f64::from).collect();
        assert_eq!(percentile(&samples, 50.0), Some(3.0));
        assert_eq!(percentile(&samples, 95.0), Some(5.0));
        assert_eq!(percentile(&VecDeque::new(), 50.0), None);
    }

    #[test]
    fn down_after_consecutive_failures_and_recovers_on_success() {
        let m = UpstreamManager::new();
        let url = "quic://a.example:853".to_string();
        let urls = vec![url.clone()];
        for _ in 0..DOWN_AFTER {
            m.note_probe(&url, false, Duration::ZERO);
        }
        assert!(!m.snapshot(&urls)[0].ok);

        m.note_probe(&url, true, Duration::from_millis(12));
        let s = &m.snapshot(&urls)[0];
        assert!(s.ok);
        assert_eq!(s.probe_samples, 1);
        assert_eq!(s.probe_p50_ms, Some(12.0));
    }

    #[test]
    fn probes_do_not_change_live_latency() {
        let m = UpstreamManager::new();
        let url = "quic://a.example:853".to_string();
        m.note_probe(&url, true, Duration::from_millis(5));
        let s = &m.snapshot(std::slice::from_ref(&url))[0];
        assert_eq!(s.avg_ms, 0.0);
        assert_eq!(s.probe_samples, 1);
    }
}

fn build_resolver(config: ResolverConfig, forwarding: bool) -> Result<TokioResolver, String> {
    let mut opts = ResolverOpts::default();
    opts.timeout = QUERY_TIMEOUT;
    opts.attempts = 1;
    if forwarding {
        // Caching is done by the DNS cache in front of the forwarder, so the
        // resolver's own cache is turned off to keep TTLs exact.
        opts.cache_size = 0;
        opts.use_hosts_file = ResolveHosts::Never;
    }
    let mut builder = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
    *builder.options_mut() = opts;
    builder.build().map_err(|e| e.to_string())
}

fn build_quic_resolver(addr: IpAddr, host: &str, port: u16) -> Result<TokioResolver, String> {
    let mut ns = NameServerConfig::quic(addr, Arc::from(host));
    for conn in ns.connections.iter_mut() {
        conn.port = port;
    }
    let config = ResolverConfig::from_parts(None, Vec::new(), vec![ns]);
    build_resolver(config, true)
}

fn build_doh_resolver(addr: IpAddr, host: &str, port: u16, path: &str) -> Result<TokioResolver, String> {
    let mut ns = NameServerConfig::https(addr, Arc::from(host), Some(Arc::from(path)));
    for conn in ns.connections.iter_mut() {
        conn.port = port;
    }
    let config = ResolverConfig::from_parts(None, Vec::new(), vec![ns]);
    build_resolver(config, true)
}
