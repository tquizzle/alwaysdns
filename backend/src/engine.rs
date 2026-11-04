//! Immutable runtime snapshot built from a validated config and loaded lists.
//! The DNS path reads one snapshot per query. Reloads swap in a new snapshot.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tracing::warn;

use crate::config::{Config, parse_client_cidrs};
use crate::lists::ListData;
use crate::rules::{CidrTable, DomainSet, RewriteTable, match_suffix, normalize_rule_domain};

/// Outcome of the policy checks, before any answer is produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A rewrite applies. Carries the local answer.
    Rewrite(IpAddr),
    /// A block rule applies and no allow rule overrides it.
    Blocked,
    /// A block rule would apply, but an allow rule overrides it.
    Allowed,
    /// No block rule applies.
    Pass,
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub verdict: Verdict,
    /// The matched rule, for example `block:global:x.com`, `list:ads:x.com`,
    /// `allow:client:x.com`, or `rewrite:*.example.test`. Empty when nothing matched.
    pub rule: String,
    /// Configured client name, or `None` for an unknown client.
    pub client: Option<String>,
    /// Upstream group to use for this client.
    pub group: String,
}

pub struct RuntimeClient {
    pub name: String,
    pub group: String,
    pub block_lists: Vec<(String, Arc<ListData>)>,
    pub allow: DomainSet,
    pub block: DomainSet,
}

pub struct Runtime {
    pub config: Config,
    pub clients: Vec<RuntimeClient>,
    pub lists: BTreeMap<String, Arc<ListData>>,
    cidrs: CidrTable,
    default_group: String,
    default_block_lists: Vec<(String, Arc<ListData>)>,
    global_allow: DomainSet,
    /// Allow lists (global). Their matches are reported as `allow:list:<name>:<domain>`.
    pub global_allow_lists: Vec<(String, Arc<ListData>)>,
    global_block: DomainSet,
    rewrites: RewriteTable,
}

fn domain_set(items: &[String]) -> DomainSet {
    items.iter().map(|d| normalize_rule_domain(d)).collect()
}

/// Resolves list names to loaded lists. A name with no loaded list is skipped
/// with a warning. A deleted list must not stop DNS, so this never fails.
fn list_refs(
    names: &[String],
    lists: &BTreeMap<String, Arc<ListData>>,
) -> Vec<(String, Arc<ListData>)> {
    names
        .iter()
        .filter_map(|n| match lists.get(n) {
            Some(l) => Some((n.clone(), l.clone())),
            None => {
                warn!(list = %n, "list referenced by config is not loaded; ignoring it");
                None
            }
        })
        .collect()
}

impl Runtime {
    /// Builds a snapshot from block lists (by name) and global allow lists.
    pub fn build_with(
        config: Config,
        lists: BTreeMap<String, Arc<ListData>>,
        global_allow_lists: Vec<(String, Arc<ListData>)>,
    ) -> Result<Self> {
        config.validate()?;

        let mut clients = Vec::with_capacity(config.clients.len());
        let mut cidr_entries = Vec::new();
        for (idx, client) in config.clients.iter().enumerate() {
            for net in parse_client_cidrs(client)? {
                cidr_entries.push((net, idx));
            }
            clients.push(RuntimeClient {
                name: client.name.clone(),
                group: client
                    .upstream_group
                    .clone()
                    .unwrap_or_else(|| config.default_upstream_group.clone()),
                block_lists: list_refs(&client.block_lists, &lists),
                allow: domain_set(&client.allow_domains),
                block: domain_set(&client.block_domains),
            });
        }

        let rewrite_rules = config
            .rewrites
            .iter()
            .map(|r| {
                r.answer
                    .parse::<IpAddr>()
                    .map(|ip| (r.domain.clone(), ip))
                    .map_err(|e| anyhow!("rewrite '{}': {e}", r.domain))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            clients,
            cidrs: CidrTable::new(cidr_entries),
            default_group: config.default_upstream_group.clone(),
            default_block_lists: list_refs(&config.default_block_lists, &lists),
            global_allow: domain_set(&config.allow_domains),
            global_allow_lists,
            global_block: domain_set(&config.block_domains),
            rewrites: RewriteTable::build(&rewrite_rules)?,
            lists,
            config,
        })
    }

    /// Client index for a source address, if any CIDR matches.
    pub fn client_index(&self, ip: IpAddr) -> Option<usize> {
        self.cidrs.lookup(ip)
    }

    /// Applies policy to one query. `name` must already be normalized
    /// (lowercase, no trailing dot).
    ///
    /// Order: allow lists, then rewrites, then block lists. An allow rule only
    /// removes blocks. It does not stop rewrites.
    pub fn decide(&self, ip: IpAddr, name: &str) -> Decision {
        let idx = self.client_index(ip);
        let (client, group) = match idx {
            Some(i) => (Some(self.clients[i].name.clone()), self.clients[i].group.clone()),
            None => (None, self.default_group.clone()),
        };

        let allow_rule = idx
            .and_then(|i| {
                match_suffix(&self.clients[i].allow, name).map(|s| format!("allow:client:{s}"))
            })
            .or_else(|| {
                match_suffix(&self.global_allow, name).map(|s| format!("allow:global:{s}"))
            })
            .or_else(|| {
                self.global_allow_lists.iter().find_map(|(list, data)| {
                    match_suffix(&data.domains, name).map(|s| format!("allow:list:{list}:{s}"))
                })
            });

        if let Some((answer, pattern)) = self.rewrites.lookup(name) {
            return Decision {
                verdict: Verdict::Rewrite(answer),
                rule: format!("rewrite:{pattern}"),
                client,
                group,
            };
        }

        let (verdict, rule) = match (self.block_rule(idx, name), allow_rule) {
            (Some(_), Some(allow)) => (Verdict::Allowed, allow),
            (Some(block), None) => (Verdict::Blocked, block),
            (None, _) => (Verdict::Pass, String::new()),
        };
        Decision {
            verdict,
            rule,
            client,
            group,
        }
    }

    fn block_rule(&self, idx: Option<usize>, name: &str) -> Option<String> {
        match idx {
            Some(i) => {
                let client = &self.clients[i];
                if let Some(s) = match_suffix(&client.block, name) {
                    return Some(format!("block:client:{s}"));
                }
                for (list, data) in &client.block_lists {
                    if let Some(s) = match_suffix(&data.domains, name) {
                        return Some(format!("list:{list}:{s}"));
                    }
                }
            }
            None => {
                for (list, data) in &self.default_block_lists {
                    if let Some(s) = match_suffix(&data.domains, name) {
                        return Some(format!("list:{list}:{s}"));
                    }
                }
            }
        }
        match_suffix(&self.global_block, name).map(|s| format!("block:global:{s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::ListData;
    use chrono::Utc;
    use std::collections::HashSet;
    use std::net::Ipv4Addr;

    fn list(name: &str, domains: &[&str]) -> Arc<ListData> {
        let set: HashSet<String> = domains.iter().map(|d| d.to_string()).collect();
        Arc::new(ListData {
            name: name.to_string(),
            domains: Arc::new(set),
            skipped: 0,
            sources_total: 1,
            sources_ok: 1,
            source_errors: vec![],
            loaded_at: Utc::now(),
        })
    }

    const CFG: &str = r#"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
  nextdns: ["https://abc123.dns.nextdns.io/dns-query"]
clients:
  - name: Laptop
    cidrs: ["192.0.2.10/32"]
    upstream_group: nextdns
    block_lists: ["ads"]
    allow_domains: ["tracker.example.com"]
  - name: WiFi
    cidrs: ["198.51.100.0/24"]
    block_lists: ["ads"]
rewrites:
  - domain: "*.example.test"
    answer: "203.0.113.116"
allow_domains: ["ok.example.org"]
block_domains: ["custom.bad"]
default_block_lists: ["ads"]
lists:
  ads:
    sources: ["https://example.com/hosts.txt"]
"#;

    fn runtime() -> Runtime {
        let config = Config::parse_yaml(CFG).unwrap();
        let mut lists = BTreeMap::new();
        lists.insert(
            "ads".to_string(),
            list("ads", &["doubleclick.net", "tracker.example.com", "ok.example.org"]),
        );
        Runtime::build_with(config, lists, Vec::new()).unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn known_client_blocks_with_list_rule() {
        let rt = runtime();
        let d = rt.decide(ip("198.51.100.9"), "pixel.doubleclick.net");
        assert_eq!(d.verdict, Verdict::Blocked);
        assert_eq!(d.rule, "list:ads:doubleclick.net");
        assert_eq!(d.client.as_deref(), Some("WiFi"));
        assert_eq!(d.group, "default");
    }

    #[test]
    fn client_allow_overrides_list_block_for_that_client_only() {
        let rt = runtime();
        let laptop = rt.decide(ip("192.0.2.10"), "tracker.example.com");
        assert_eq!(laptop.verdict, Verdict::Allowed);
        assert_eq!(laptop.rule, "allow:client:tracker.example.com");
        assert_eq!(laptop.group, "nextdns");

        let wifi = rt.decide(ip("198.51.100.9"), "tracker.example.com");
        assert_eq!(wifi.verdict, Verdict::Blocked);
    }

    #[test]
    fn global_allow_overrides_default_lists_for_unknown_clients() {
        let rt = runtime();
        let d = rt.decide(ip("192.168.50.1"), "ok.example.org");
        assert_eq!(d.verdict, Verdict::Allowed);
        assert_eq!(d.client, None);
        assert_eq!(d.group, "default");
        // Unknown clients still get default lists for other names.
        let blocked = rt.decide(ip("192.168.50.1"), "doubleclick.net");
        assert_eq!(blocked.verdict, Verdict::Blocked);
    }

    #[test]
    fn rewrite_takes_precedence_over_block() {
        let rt = runtime();
        let d = rt.decide(ip("198.51.100.9"), "nas.example.test");
        assert_eq!(d.verdict, Verdict::Rewrite(ip("203.0.113.116")));
        assert_eq!(d.rule, "rewrite:*.example.test");
    }

    #[test]
    fn global_custom_block_applies_to_everyone() {
        let rt = runtime();
        let d = rt.decide(ip("192.0.2.10"), "a.custom.bad");
        assert_eq!(d.verdict, Verdict::Blocked);
        assert_eq!(d.rule, "block:global:custom.bad");
    }

    #[test]
    fn unmatched_name_passes() {
        let rt = runtime();
        let d = rt.decide(ip("198.51.100.9"), "example.com");
        assert_eq!(d.verdict, Verdict::Pass);
        assert!(d.rule.is_empty());
    }

    #[test]
    fn ipv4_mapped_source_matches_client() {
        let rt = runtime();
        let d = rt.decide(ip("::ffff:192.0.2.10"), "doubleclick.net");
        assert_eq!(d.client.as_deref(), Some("Laptop"));
        let _ = Ipv4Addr::LOCALHOST;
    }

    #[test]
    fn allow_list_overrides_block_list_and_reports_its_name() {
        let config = Config::parse_yaml(CFG).unwrap();
        let mut lists = BTreeMap::new();
        lists.insert("ads".to_string(), list("ads", &["doubleclick.net"]));
        let trusted = list("trusted", &["doubleclick.net"]);
        let rt = Runtime::build_with(config, lists, vec![("trusted".to_string(), trusted)]).unwrap();
        let d = rt.decide(ip("192.168.50.1"), "pixel.doubleclick.net");
        assert_eq!(d.verdict, Verdict::Allowed);
        assert_eq!(d.rule, "allow:list:trusted:doubleclick.net");
    }

    #[test]
    fn dangling_list_reference_is_skipped_not_fatal() {
        // The CFG client names "ads", but no list named "ads" is loaded.
        let config = Config::parse_yaml(CFG).unwrap();
        let rt = Runtime::build_with(config, BTreeMap::new(), Vec::new()).unwrap();
        let d = rt.decide(ip("198.51.100.9"), "pixel.doubleclick.net");
        assert_eq!(d.verdict, Verdict::Pass);
    }
}
