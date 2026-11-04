//! One-shot conversion of an AdGuard Home config into this project's config
//! format (`dnsui import-adguard`).
//!
//! Every rule that is dropped or changed is recorded in the report, so the
//! CLI can print the list to stderr.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result};
use serde_yaml::Value;

use crate::config::{ClientConfig, Config, DEFAULT_GROUP, ListConfig, ListFormat, RewriteConfig};
use crate::rules::{is_valid_domain, normalize_name, normalize_rule_domain, parse_cidr};

const FALLBACK_DOH: &str = "https://dns10.quad9.net/dns-query";
const FALLBACK_BOOTSTRAP: &str = "9.9.9.10";

#[derive(Debug, Default)]
pub struct ImportReport {
    pub clients: usize,
    pub lists: usize,
    pub allows: usize,
    pub blocks: usize,
    pub rewrites: usize,
    /// Rules or settings that were removed entirely.
    pub dropped: Vec<String>,
    /// Rules or settings that were kept but altered.
    pub changed: Vec<String>,
}

pub fn import_adguard(text: &str) -> Result<(Config, ImportReport)> {
    let root: Value = serde_yaml::from_str(text).context("parsing AdGuardHome.yaml")?;
    let mut report = ImportReport::default();

    // Upstreams and bootstrap.
    let mut default_urls = Vec::new();
    for entry in str_seq(&root["dns"]["upstream_dns"]) {
        match upstream_only(&entry) {
            Some(url) => default_urls.push(url),
            None => report
                .dropped
                .push(format!("default upstream '{entry}': only DoH upstreams are supported")),
        }
    }
    if default_urls.is_empty() {
        report.changed.push(format!(
            "no DoH default upstream found; using {FALLBACK_DOH}"
        ));
        default_urls.push(FALLBACK_DOH.to_string());
    }

    let mut bootstrap = Vec::new();
    for entry in str_seq(&root["dns"]["bootstrap_dns"]) {
        match entry.parse::<std::net::IpAddr>() {
            Ok(ip) => bootstrap.push(ip),
            Err(_) => report
                .dropped
                .push(format!("bootstrap '{entry}': only plain IP bootstrap servers are supported")),
        }
    }
    if bootstrap.is_empty() {
        report
            .changed
            .push(format!("no bootstrap IP found; using {FALLBACK_BOOTSTRAP}"));
        bootstrap.push(FALLBACK_BOOTSTRAP.parse().expect("valid IP literal"));
    }

    let mut upstream_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    upstream_groups.insert(DEFAULT_GROUP.to_string(), default_urls);
    let mut used_groups: HashSet<String> = HashSet::from([DEFAULT_GROUP.to_string()]);

    // Filters become lists. The enabled set is shared by every client.
    let mut lists: BTreeMap<String, ListConfig> = BTreeMap::new();
    let mut enabled_lists: Vec<String> = Vec::new();
    let mut used_list_names: HashSet<String> = HashSet::new();
    for filter in root["filters"].as_sequence().into_iter().flatten() {
        let enabled = filter["enabled"].as_bool().unwrap_or(true);
        let Some(url) = filter["url"].as_str() else {
            continue;
        };
        if !enabled {
            continue;
        }
        let id = filter["id"].as_i64().map(|i| i.to_string()).unwrap_or_default();
        let raw_name = filter["name"].as_str().unwrap_or("");
        let base = {
            let s = sanitize(raw_name);
            if s.is_empty() { format!("filter_{id}") } else { s }
        };
        let name = unique(base, &mut used_list_names);
        lists.insert(
            name.clone(),
            ListConfig {
                sources: vec![url.to_string()],
                format: ListFormat::Auto,
            },
        );
        enabled_lists.push(name);
    }
    report.lists = lists.len();

    let whitelist_count = root["whitelist_filters"].as_sequence().map(Vec::len).unwrap_or(0);
    if whitelist_count > 0 {
        report.dropped.push(format!(
            "whitelist_filters: {whitelist_count} allow-list subscription(s) not supported; allow rules must be added manually"
        ));
    }

    // Clients.
    let mut clients: Vec<ClientConfig> = Vec::new();
    let mut client_names: HashSet<String> = HashSet::new();
    for persistent in root["clients"]["persistent"].as_sequence().into_iter().flatten() {
        let Some(name) = persistent["name"].as_str().map(str::to_string).filter(|n| !n.trim().is_empty()) else {
            report.dropped.push("client without a name was skipped".to_string());
            continue;
        };
        if !client_names.insert(name.clone()) {
            report.dropped.push(format!("client '{name}': duplicate name skipped"));
            continue;
        }

        let mut cidrs = Vec::new();
        for id in str_seq(&persistent["ids"]) {
            match parse_cidr(&id) {
                Ok(net) => cidrs.push(net.to_string()),
                Err(_) => report.dropped.push(format!(
                    "client '{name}': identifier '{id}' is not an IP or CIDR"
                )),
            }
        }

        let mut urls = Vec::new();
        for entry in str_seq(&persistent["upstreams"]) {
            match upstream_only(&entry) {
                Some(url) => urls.push(url),
                None => report.dropped.push(format!(
                    "client '{name}': upstream '{entry}' is not DoH and was dropped"
                )),
            }
        }
        let group = if urls.is_empty() {
            None
        } else {
            let group = unique(sanitize_group(&name), &mut used_groups);
            upstream_groups.insert(group.clone(), urls);
            Some(group)
        };

        clients.push(ClientConfig {
            name,
            cidrs,
            upstream_group: group,
            block_lists: enabled_lists.clone(),
            allow_domains: Vec::new(),
            block_domains: Vec::new(),
        });
    }
    report.clients = clients.len();

    // Default block lists: the same enabled set, applied to unknown clients.
    let default_block_lists = enabled_lists.clone();

    // User rules.
    let mut global_allow = Vec::new();
    let mut global_block = Vec::new();
    for raw in str_seq(&root["user_rules"]) {
        apply_user_rule(
            &raw,
            &mut clients,
            &mut global_allow,
            &mut global_block,
            &mut report,
        );
    }
    report.allows = global_allow.len() + clients.iter().map(|c| c.allow_domains.len()).sum::<usize>();
    report.blocks = global_block.len() + clients.iter().map(|c| c.block_domains.len()).sum::<usize>();

    // Rewrites.
    let mut rewrites = Vec::new();
    for entry in root["filtering"]["rewrites"].as_sequence().into_iter().flatten() {
        let domain = entry["domain"].as_str().unwrap_or("").trim().to_string();
        let answer = entry["answer"].as_str().unwrap_or("").trim().to_string();
        let body = normalize_name(&domain);
        let body = body.strip_prefix("*.").unwrap_or(&body).to_string();
        if !is_valid_domain(&body) {
            report.dropped.push(format!("rewrite '{domain}': not a valid domain"));
            continue;
        }
        if answer.parse::<std::net::IpAddr>().is_err() {
            report.dropped.push(format!(
                "rewrite '{domain}' -> '{answer}': only IP answers are supported"
            ));
            continue;
        }
        rewrites.push(RewriteConfig { domain, answer });
    }
    report.rewrites = rewrites.len();

    let config = Config {
        listen: Default::default(),
        bootstrap,
        upstream_groups,
        clients,
        lists,
        allow_lists: BTreeMap::new(),
        retention: Default::default(),
        default_block_lists,
        rewrites,
        allow_domains: global_allow,
        block_domains: global_block,
        default_upstream_group: DEFAULT_GROUP.to_string(),
    };
    config
        .validate()
        .context("the imported config is not valid; please report this")?;
    Ok((config, report))
}

/// Parses one AdGuard user rule. Each outcome is recorded in the report.
fn apply_user_rule(
    raw: &str,
    clients: &mut [ClientConfig],
    global_allow: &mut Vec<String>,
    global_block: &mut Vec<String>,
    report: &mut ImportReport,
) {
    let rule = raw.trim();
    if rule.is_empty() || rule.starts_with('!') || rule.starts_with('#') {
        return;
    }
    if rule.contains("##") || rule.contains("#@#") || rule.contains("#$#") || rule.contains("#?#") {
        report.dropped.push(format!("'{rule}': cosmetic rules are not supported"));
        return;
    }
    let (is_allow, body) = match rule.strip_prefix("@@") {
        Some(rest) => (true, rest),
        None => (false, rule),
    };
    if body.starts_with('/') {
        report
            .dropped
            .push(format!("'{rule}': regex rules are not supported"));
        return;
    }
    let Some(rest) = body.strip_prefix("||") else {
        report.dropped.push(format!("'{rule}': only ||domain^ rules are supported"));
        return;
    };

    let (pattern, modifiers) = match rest.split_once('$') {
        Some((p, m)) => (p, Some(m)),
        None => (rest, None),
    };
    let domain_raw = pattern.strip_suffix('^').unwrap_or(pattern);
    if domain_raw.is_empty() {
        report.dropped.push(format!("'{rule}': empty domain"));
        return;
    }
    if domain_raw.contains(['/', '*', '|', ':']) {
        report.dropped.push(format!("'{rule}': path or wildcard rules are not supported"));
        return;
    }
    let domain = normalize_rule_domain(domain_raw);
    if !is_valid_domain(&domain) {
        report.dropped.push(format!("'{rule}': invalid domain"));
        return;
    }

    let mut important = false;
    let mut client_scope: Option<String> = None;
    if let Some(mods) = modifiers {
        for m in mods.split(',').map(str::trim).filter(|m| !m.is_empty()) {
            if m == "important" {
                important = true;
            } else if let Some(value) = m.strip_prefix("client=") {
                client_scope = Some(value.trim_matches(|c| c == '\'' || c == '"').to_string());
            } else {
                report.dropped.push(format!(
                    "'{rule}': modifier '${m}' is not supported; rule dropped"
                ));
                return;
            }
        }
    }
    if important {
        report.changed.push(format!(
            "'{rule}': $important ignored; domain kept as a normal {} rule",
            if is_allow { "allow" } else { "block" }
        ));
    }

    // A client scope applies only if it names a configured client.
    let target_client = client_scope.as_ref().and_then(|scope| {
        clients.iter().position(|c| &c.name == scope)
    });
    if let Some(scope) = &client_scope {
        if target_client.is_none() {
            report.changed.push(format!(
                "'{rule}': client '{scope}' not found; applied globally"
            ));
        }
    }

    match (is_allow, target_client) {
        (true, Some(idx)) => clients[idx].allow_domains.push(domain),
        (true, None) => global_allow.push(domain),
        (false, Some(idx)) => clients[idx].block_domains.push(domain),
        (false, None) => global_block.push(domain),
    }
}

/// Returns the URL if it is a DoH (https://) or DoQ (quic://) endpoint.
/// Disabled entries such as "# quic://..." are skipped.
fn upstream_only(entry: &str) -> Option<String> {
    let trimmed = entry.trim();
    if trimmed.starts_with("https://") || trimmed.starts_with("quic://") {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn str_seq(v: &Value) -> Vec<String> {
    v.as_sequence()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

fn sanitize(raw: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore && !out.is_empty() {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

fn sanitize_group(name: &str) -> String {
    let s = sanitize(name);
    if s.is_empty() { "client".to_string() } else { s }
}

fn unique(base: String, used: &mut HashSet<String>) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}_{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
dns:
  upstream_dns:
    - https://dns10.quad9.net/dns-query
    - 9.9.9.9
    - "[/local/]10.0.0.1"
  bootstrap_dns:
    - 9.9.9.10
clients:
  persistent:
    - name: Laptop
      ids: ["192.0.2.10", "198.51.100.65", "203.0.113.0/24", "aa:bb:cc:dd:ee:ff"]
      upstreams: ["https://abc123.dns.nextdns.io/dns-query", "9.9.9.9"]
      use_global_settings: true
    - name: Kids Room
      ids: ["10.0.3.5"]
      upstreams: []
filters:
  - enabled: true
    url: https://example.com/ads.txt
    name: AdGuard DNS filter
    id: 1
  - enabled: false
    url: https://example.com/off.txt
    name: Off
    id: 2
whitelist_filters:
  - url: https://example.com/allow.txt
user_rules:
  - "||doubleclick.net^"
  - "@@||tracker.example.com^$client='Laptop'"
  - "@@||allowed.example^"
  - "||tracker.io^$important"
  - "/regex\\d+/"
  - "example.com##.banner"
  - "||site.com/path^"
  - "! a comment"
  - "@@||nope.example^$client='Ghost'"
filtering:
  rewrites:
    - domain: "*.example.test"
      answer: 203.0.113.116
    - domain: dns.example.test
      answer: 192.0.2.53
    - domain: cname.example.test
      answer: other.example
"#;

    #[test]
    fn imports_sample_config() {
        let (cfg, report) = import_adguard(SAMPLE).expect("sample imports");

        assert_eq!(cfg.clients.len(), 2);
        let laptop = cfg.clients.iter().find(|c| c.name == "Laptop").unwrap();
        assert_eq!(laptop.cidrs, vec!["192.0.2.10/32", "198.51.100.65/32", "203.0.113.0/24"]);
        assert_eq!(laptop.upstream_group.as_deref(), Some("laptop"));
        assert_eq!(cfg.upstream_groups["laptop"], vec!["https://abc123.dns.nextdns.io/dns-query"]);
        assert_eq!(laptop.block_lists, vec!["adguard_dns_filter"]);
        assert_eq!(laptop.allow_domains, vec!["tracker.example.com"]);

        let kids = cfg.clients.iter().find(|c| c.name == "Kids Room").unwrap();
        assert_eq!(kids.upstream_group, None);

        assert_eq!(cfg.lists.len(), 1);
        assert_eq!(cfg.default_block_lists, vec!["adguard_dns_filter"]);
        // The rule scoped to the unknown client "Ghost" falls back to global.
        assert_eq!(cfg.allow_domains, vec!["allowed.example", "nope.example"]);
        assert_eq!(cfg.block_domains, vec!["doubleclick.net", "tracker.io"]);
        assert_eq!(cfg.rewrites.len(), 2);
        assert_eq!(report.rewrites, 2);
        assert_eq!(report.allows, 3);
        assert_eq!(report.blocks, 2);
        assert_eq!(report.clients, 2);

        let dropped = report.dropped.join("\n");
        assert!(dropped.contains("whitelist_filters"));
        assert!(dropped.contains("9.9.9.9"));
        assert!(dropped.contains("regex"));
        assert!(dropped.contains("cosmetic"));
        assert!(dropped.contains("/path"));
        assert!(dropped.contains("aa:bb:cc:dd:ee:ff"));
        assert!(dropped.contains("cname.example.test"));

        let changed = report.changed.join("\n");
        assert!(changed.contains("$important"));
        assert!(changed.contains("'Ghost'"));
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("AdGuard DNS filter"), "adguard_dns_filter");
        assert_eq!(sanitize("  Kids (room) "), "kids_room");
        assert_eq!(sanitize_group("!!!"), "client");
    }
}

#[cfg(test)]
mod upstream_filter_tests {
    use super::upstream_only;

    #[test]
    fn keeps_doh_and_doq_skips_disabled() {
        assert_eq!(upstream_only("quic://def456.dns.nextdns.io").as_deref(), Some("quic://def456.dns.nextdns.io"));
        assert_eq!(upstream_only("https://dns10.quad9.net/dns-query").as_deref(), Some("https://dns10.quad9.net/dns-query"));
        assert_eq!(upstream_only("# quic://789abc.dns.nextdns.io"), None);
        assert_eq!(upstream_only("198.51.100.50"), None);
    }
}
