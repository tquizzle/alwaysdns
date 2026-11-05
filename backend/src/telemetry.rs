//! Native query telemetry: a ring buffer of the most recent queries, plus
//! per-minute buckets kept for seven days to serve the stats endpoint.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;

use chrono::{DateTime, SecondsFormat, Utc};

use crate::models::{QueryItem, StatsResponse, TimelinePoint, TopEntry};

/// Most recent queries kept in memory.
pub const LOG_CAPACITY: usize = 5_000;
/// Per-minute buckets older than this are dropped.
const RETENTION_MINUTES: i64 = 7 * 24 * 60;
/// Distinct domains tracked per minute. Extra names still count toward totals.
const MAX_DOMAINS_PER_MINUTE: usize = 2_000;
/// Number of entries in top lists.
const TOP_N: usize = 20;

/// Window for stats queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Hour,
    Day,
    Week,
}

impl Range {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "hour" => Some(Self::Hour),
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            _ => None,
        }
    }

    pub fn minutes(self) -> i64 {
        match self {
            Self::Hour => 60,
            Self::Day => 24 * 60,
            Self::Week => 7 * 24 * 60,
        }
    }

    /// Timeline granularity: minutes for hour, hours for day, days for week.
    pub fn bucket_minutes(self) -> i64 {
        match self {
            Self::Hour => 1,
            Self::Day => 60,
            Self::Week => 24 * 60,
        }
    }
}

/// One DNS query as observed by the resolver.
#[derive(Debug, Clone)]
pub struct QueryEvent {
    pub client_ip: IpAddr,
    /// Configured client name, or `None` for an unknown client.
    pub client: Option<String>,
    pub domain: String,
    pub qtype: String,
    pub result: &'static str,
    pub upstream: String,
    pub elapsed_ms: f64,
    pub rule: String,
}

#[derive(Default)]
struct Minute {
    total: u64,
    blocked: u64,
    cached: u64,
    forwarded: u64,
    sum_ms: f64,
    clients: HashSet<IpAddr>,
    domains: HashMap<String, u64>,
    blocked_domains: HashMap<String, u64>,
    /// Configured client name to (total, blocked).
    per_client: HashMap<String, (u64, u64)>,
}

#[derive(Default)]
struct Inner {
    log: VecDeque<QueryItem>,
    minutes: BTreeMap<i64, Minute>,
}

#[derive(Default)]
pub struct Telemetry {
    inner: Mutex<Inner>,
}

impl Telemetry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, ev: QueryEvent) {
        let now = Utc::now();
        let minute_key = now.timestamp().div_euclid(60);
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };

        inner.log.push_back(QueryItem {
            time: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            client: ev.client.clone().unwrap_or_else(|| "unknown".to_string()),
            domain: ev.domain.clone(),
            qtype: ev.qtype.clone(),
            result: ev.result.to_string(),
            upstream: ev.upstream.clone(),
            upstream_name: None,
            elapsed_ms: round3(ev.elapsed_ms),
            rule: ev.rule.clone(),
            client_ip: Some(ev.client_ip.to_string()),
        });
        while inner.log.len() > LOG_CAPACITY {
            inner.log.pop_front();
        }

        let blocked = ev.result == "blocked";
        let minute = inner.minutes.entry(minute_key).or_default();
        minute.total += 1;
        minute.sum_ms += ev.elapsed_ms;
        minute.clients.insert(ev.client_ip);
        match ev.result {
            "blocked" => minute.blocked += 1,
            "cached" => minute.cached += 1,
            "forwarded" => minute.forwarded += 1,
            _ => {}
        }
        bump_capped(&mut minute.domains, &ev.domain);
        if blocked {
            bump_capped(&mut minute.blocked_domains, &ev.domain);
        }
        if let Some(name) = &ev.client {
            let entry = minute.per_client.entry(name.clone()).or_insert((0, 0));
            entry.0 += 1;
            if blocked {
                entry.1 += 1;
            }
        }

        let cutoff = minute_key - RETENTION_MINUTES;
        while let Some((&oldest, _)) = inner.minutes.first_key_value() {
            if oldest >= cutoff {
                break;
            }
            inner.minutes.remove(&oldest);
        }
    }

    /// Most recent queries, newest first, filtered by `keep`.
    pub fn query_log(&self, limit: usize, keep: impl Fn(&QueryItem) -> bool) -> Vec<QueryItem> {
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };
        inner
            .log
            .iter()
            .rev()
            .filter(|item| keep(item))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Aggregated stats for the window ending now. `upstreams` is left empty
    /// and filled in by the caller, which owns upstream health.
    pub fn stats(&self, range: Range, now: DateTime<Utc>) -> StatsResponse {
        let Ok(inner) = self.inner.lock() else {
            return StatsResponse::default();
        };
        let now_min = now.timestamp().div_euclid(60);
        let start = now_min - range.minutes() + 1;
        let bucket = range.bucket_minutes();
        let n_buckets = (range.minutes() / bucket) as usize;

        let mut timeline: Vec<TimelinePoint> = (0..n_buckets)
            .map(|i| TimelinePoint {
                t: ts_string((start + i as i64 * bucket) * 60),
                total: 0,
                blocked: 0,
            })
            .collect();

        let mut resp = StatsResponse::default();
        let mut sum_ms = 0.0;
        let mut clients: HashSet<IpAddr> = HashSet::new();
        let mut domains: HashMap<&str, u64> = HashMap::new();
        let mut blocked_domains: HashMap<&str, u64> = HashMap::new();

        for (&min, m) in inner.minutes.range(start..=now_min) {
            resp.total += m.total;
            resp.blocked += m.blocked;
            resp.cached += m.cached;
            resp.forwarded += m.forwarded;
            sum_ms += m.sum_ms;
            clients.extend(m.clients.iter().copied());
            for (d, c) in &m.domains {
                *domains.entry(d.as_str()).or_insert(0) += c;
            }
            for (d, c) in &m.blocked_domains {
                *blocked_domains.entry(d.as_str()).or_insert(0) += c;
            }
            let idx = ((min - start) / bucket) as usize;
            if let Some(point) = timeline.get_mut(idx) {
                point.total += m.total;
                point.blocked += m.blocked;
            }
        }

        if resp.total > 0 {
            resp.avg_ms = round3(sum_ms / resp.total as f64);
            resp.block_rate = resp.blocked as f64 / resp.total as f64;
        }
        resp.active_clients = clients.len();
        resp.timeline = timeline;
        resp.top_domains = top_n(domains);
        resp.top_blocked = top_n(blocked_domains);
        resp
    }

    /// Per configured client: (queries, blocked) within the window.
    pub fn client_counts(&self, range: Range, now: DateTime<Utc>) -> HashMap<String, (u64, u64)> {
        let mut out: HashMap<String, (u64, u64)> = HashMap::new();
        let Ok(inner) = self.inner.lock() else {
            return out;
        };
        let now_min = now.timestamp().div_euclid(60);
        let start = now_min - range.minutes() + 1;
        for (_, m) in inner.minutes.range(start..=now_min) {
            for (name, (total, blocked)) in &m.per_client {
                let entry = out.entry(name.clone()).or_insert((0, 0));
                entry.0 += total;
                entry.1 += blocked;
            }
        }
        out
    }
}

fn bump_capped(map: &mut HashMap<String, u64>, key: &str) {
    if let Some(count) = map.get_mut(key) {
        *count += 1;
    } else if map.len() < MAX_DOMAINS_PER_MINUTE {
        map.insert(key.to_string(), 1);
    }
}

fn top_n(map: HashMap<&str, u64>) -> Vec<TopEntry> {
    let mut items: Vec<TopEntry> = map
        .into_iter()
        .map(|(name, count)| TopEntry {
            name: name.to_string(),
            count,
            reason: None,
        })
        .collect();
    items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    items.truncate(TOP_N);
    items
}

fn ts_string(unix_secs: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix_secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ev(domain: &str, result: &'static str, client: Option<&str>) -> QueryEvent {
        QueryEvent {
            client_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            client: client.map(str::to_string),
            domain: domain.to_string(),
            qtype: "A".to_string(),
            result,
            upstream: "https://example/dns-query".to_string(),
            elapsed_ms: 2.0,
            rule: String::new(),
        }
    }

    #[test]
    fn records_and_aggregates() {
        let t = Telemetry::new();
        t.record(ev("a.com", "forwarded", Some("Laptop")));
        t.record(ev("ads.net", "blocked", Some("Laptop")));
        t.record(ev("ads.net", "blocked", None));
        let stats = t.stats(Range::Hour, Utc::now());
        assert_eq!(stats.total, 3);
        assert_eq!(stats.blocked, 2);
        assert_eq!(stats.forwarded, 1);
        assert_eq!(stats.top_blocked[0].name, "ads.net");
        assert_eq!(stats.top_blocked[0].count, 2);
        assert_eq!(stats.timeline.len(), 60);
        let counts = t.client_counts(Range::Hour, Utc::now());
        assert_eq!(counts.get("Laptop"), Some(&(2, 1)));
    }

    #[test]
    fn ring_buffer_is_bounded_and_newest_first() {
        let t = Telemetry::new();
        for i in 0..(LOG_CAPACITY + 10) {
            t.record(ev(&format!("d{i}.com"), "forwarded", None));
        }
        let items = t.query_log(usize::MAX, |_| true);
        assert_eq!(items.len(), LOG_CAPACITY);
        assert_eq!(items[0].domain, format!("d{}.com", LOG_CAPACITY + 9));
    }

    #[test]
    fn range_parsing() {
        assert_eq!(Range::parse("hour"), Some(Range::Hour));
        assert_eq!(Range::parse("month"), None);
        assert_eq!(Range::Week.bucket_minutes(), 1440);
    }
}
