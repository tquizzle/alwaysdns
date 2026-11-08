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
use crate::safesearch::{self, SafeSearch};
use crate::services::Catalogue;

/// Outcome of the policy checks, before any answer is produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A rewrite applies. Carries the local answer.
    Rewrite(IpAddr),
    /// Safe search applies. The name is answered as an alias of the provider's safe host.
    SafeSearch(SafeSearch),
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
    /// Safe search for this client, after applying the global default.
    pub safe_search: bool,
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
    /// Safe search for clients that match no CIDR.
    safe_search: bool,
    /// Apps clients can block. Empty until `with_services` supplies the catalogue.
    pub catalogue: Arc<Catalogue>,
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
                safe_search: client.safe_search.unwrap_or(config.safe_search),
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
            safe_search: config.safe_search,
            catalogue: Arc::new(Catalogue::default()),
            lists,
            config,
        })
    }

    /// Adds each client's blocked apps to its block lists. An id missing from the
    /// catalogue is skipped with a warning, like a list that is not loaded: a catalogue
    /// that is down or has dropped a service must not stop DNS.
    pub fn with_services(mut self, catalogue: Arc<Catalogue>) -> Self {
        for (runtime, configured) in self.clients.iter_mut().zip(&self.config.clients) {
            for id in &configured.blocked_services {
                match catalogue.get(id) {
                    Some(service) => runtime
                        .block_lists
                        .push((service.data.name.clone(), service.data.clone())),
                    None => warn!(
                        service = %id,
                        client = %configured.name,
                        "blocked app is not in the catalogue; ignoring it"
                    ),
                }
            }
        }
        self.catalogue = catalogue;
        self
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
        // A block still wins. An allow rule does not switch safe search off, the same
        // as it does not stop a rewrite.
        let (verdict, rule) = match self.safe_search_for(idx, name) {
            Some(found) if verdict != Verdict::Blocked => {
                (Verdict::SafeSearch(found), format!("safesearch:{}", found.provider))
            }
            _ => (verdict, rule),
        };
        Decision {
            verdict,
            rule,
            client,
            group,
        }
    }

    /// The safe-search redirect for `name`, when it is on for this client.
    fn safe_search_for(&self, idx: Option<usize>, name: &str) -> Option<SafeSearch> {
        let on = idx.map_or(self.safe_search, |i| self.clients[i].safe_search);
        if on { safesearch::lookup(name) } else { None }
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

    const APPS_CFG: &str = r#"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
clients:
  - name: Kid
    cidrs: ["10.0.3.0/24"]
    blocked_services: ["youtube", "no_such_app"]
  - name: Parent
    cidrs: ["10.0.4.0/24"]
"#;

    const APPS_CATALOGUE: &str = r#"{"blocked_services": [
        {"id": "youtube", "name": "YouTube", "group": "streaming", "rules": ["||youtube.com^", "||googlevideo.com^"]}
    ]}"#;

    fn apps_runtime() -> Runtime {
        let config = Config::parse_yaml(APPS_CFG).unwrap();
        let catalogue = crate::services::parse_catalogue(APPS_CATALOGUE).unwrap();
        Runtime::build_with(config, BTreeMap::new(), Vec::new())
            .unwrap()
            .with_services(Arc::new(catalogue))
    }

    #[test]
    fn a_blocked_app_blocks_its_domains_for_that_client_only() {
        let rt = apps_runtime();
        let kid = rt.decide(ip("10.0.3.7"), "rr1.googlevideo.com");
        assert_eq!(kid.verdict, Verdict::Blocked);
        assert_eq!(kid.rule, "list:app/youtube:googlevideo.com");
        // Another client, and an unknown one, still reach the app.
        assert_eq!(rt.decide(ip("10.0.4.7"), "www.youtube.com").verdict, Verdict::Pass);
        assert_eq!(rt.decide(ip("192.168.50.1"), "www.youtube.com").verdict, Verdict::Pass);
    }

    #[test]
    fn an_app_missing_from_the_catalogue_is_ignored_not_fatal() {
        // "no_such_app" is on the client but not in the catalogue. DNS must still work.
        let rt = apps_runtime();
        assert_eq!(rt.decide(ip("10.0.3.7"), "example.com").verdict, Verdict::Pass);
    }

    #[test]
    fn a_client_allow_rule_overrides_a_blocked_app() {
        let yaml = APPS_CFG.replace(
            "blocked_services: [\"youtube\", \"no_such_app\"]",
            "blocked_services: [\"youtube\"]\n    allow_domains: [\"music.youtube.com\"]",
        );
        let catalogue = crate::services::parse_catalogue(APPS_CATALOGUE).unwrap();
        let rt = Runtime::build_with(Config::parse_yaml(&yaml).unwrap(), BTreeMap::new(), Vec::new())
            .unwrap()
            .with_services(Arc::new(catalogue));
        assert_eq!(rt.decide(ip("10.0.3.7"), "music.youtube.com").verdict, Verdict::Allowed);
        assert_eq!(rt.decide(ip("10.0.3.7"), "www.youtube.com").verdict, Verdict::Blocked);
    }

    #[test]
    fn dangling_list_reference_is_skipped_not_fatal() {
        // The CFG client names "ads", but no list named "ads" is loaded.
        let config = Config::parse_yaml(CFG).unwrap();
        let rt = Runtime::build_with(config, BTreeMap::new(), Vec::new()).unwrap();
        let d = rt.decide(ip("198.51.100.9"), "pixel.doubleclick.net");
        assert_eq!(d.verdict, Verdict::Pass);
    }

    const SAFE_CFG: &str = r#"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
clients:
  - name: Kid
    cidrs: ["192.0.2.20/32"]
  - name: Parent
    cidrs: ["192.0.2.21/32"]
    safe_search: false
  - name: Guest
    cidrs: ["192.0.2.22/32"]
    safe_search: true
"#;

    fn safe_runtime(tail: &str) -> Runtime {
        let yaml = format!("{SAFE_CFG}{tail}");
        Runtime::build_with(Config::parse_yaml(&yaml).unwrap(), BTreeMap::new(), Vec::new()).unwrap()
    }

    fn is_safe_search(rt: &Runtime, client: &str, name: &str) -> bool {
        matches!(rt.decide(ip(client), name).verdict, Verdict::SafeSearch(_))
    }

    #[test]
    fn safe_search_is_off_unless_someone_turns_it_on() {
        let rt = safe_runtime("");
        assert!(!is_safe_search(&rt, "203.0.113.9", "www.google.com"));
        assert!(!is_safe_search(&rt, "192.0.2.20", "www.google.com"));
    }

    #[test]
    fn global_on_covers_unknown_clients_and_clients_that_inherit() {
        let rt = safe_runtime("safe_search: true\n");
        let d = rt.decide(ip("203.0.113.9"), "www.google.com");
        assert!(matches!(d.verdict, Verdict::SafeSearch(_)));
        assert_eq!(d.rule, "safesearch:google");
        assert!(is_safe_search(&rt, "192.0.2.20", "www.youtube.com"));
        // Only the search front doors are covered. Everything else passes untouched.
        assert!(!is_safe_search(&rt, "192.0.2.20", "mail.google.com"));
    }

    #[test]
    fn a_client_set_to_off_beats_the_global_on() {
        let rt = safe_runtime("safe_search: true\n");
        assert!(!is_safe_search(&rt, "192.0.2.21", "www.google.com"));
    }

    #[test]
    fn a_client_set_to_on_beats_the_global_off() {
        let rt = safe_runtime("safe_search: false\n");
        assert!(is_safe_search(&rt, "192.0.2.22", "www.bing.com"));
        assert!(!is_safe_search(&rt, "192.0.2.20", "www.bing.com"));
        assert!(!is_safe_search(&rt, "203.0.113.9", "www.bing.com"));
    }

    #[test]
    fn a_block_rule_wins_over_safe_search() {
        let rt = safe_runtime("safe_search: true\nblock_domains: [\"www.bing.com\"]\n");
        assert_eq!(rt.decide(ip("192.0.2.20"), "www.bing.com").verdict, Verdict::Blocked);
    }

    #[test]
    fn an_allow_rule_does_not_switch_safe_search_off() {
        let yaml = r#"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
clients:
  - name: Kid
    cidrs: ["192.0.2.20/32"]
    block_lists: ["ads"]
    allow_domains: ["www.youtube.com"]
lists:
  ads:
    sources: ["https://example.com/hosts.txt"]
safe_search: true
"#;
        let mut lists = BTreeMap::new();
        lists.insert("ads".to_string(), list("ads", &["www.youtube.com"]));
        let rt = Runtime::build_with(Config::parse_yaml(yaml).unwrap(), lists, Vec::new()).unwrap();
        // The allow rule beats the list, and safe search still applies on top.
        assert!(is_safe_search(&rt, "192.0.2.20", "www.youtube.com"));
    }

    #[test]
    fn a_local_rewrite_wins_over_safe_search() {
        let rt = safe_runtime(
            "safe_search: true\nrewrites:\n  - domain: \"www.google.com\"\n    answer: \"203.0.113.5\"\n",
        );
        assert!(matches!(rt.decide(ip("192.0.2.20"), "www.google.com").verdict, Verdict::Rewrite(_)));
    }

    #[test]
    fn safe_search_settings_survive_a_yaml_round_trip() {
        let config = Config::parse_yaml(&format!("{SAFE_CFG}safe_search: true\n")).unwrap();
        let again = Config::parse_yaml(&config.to_yaml().unwrap()).unwrap();
        assert!(again.safe_search);
        let overrides: Vec<_> = again.clients.iter().map(|c| c.safe_search).collect();
        assert_eq!(overrides, vec![None, Some(false), Some(true)]);
    }

    #[test]
    fn existing_configs_do_not_gain_a_safe_search_key_when_exported() {
        let yaml = Config::parse_yaml(CFG).unwrap().to_yaml().unwrap();
        assert!(!yaml.contains("safe_search"), "{yaml}");
    }
}
