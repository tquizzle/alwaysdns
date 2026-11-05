//! Configuration file model, parsing, and validation.
//!
//! The config is YAML on disk and JSON over the HTTP API. Both use the same
//! serde model, so `GET /api/config` returns exactly what `PUT /api/config`
//! accepts.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use anyhow::{Context, Result, bail};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::rules::{is_valid_domain, normalize_name, normalize_rule_domain, parse_cidr};

/// Name of the upstream group used when nothing more specific applies.
pub const DEFAULT_GROUP: &str = "default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub listen: Listen,
    #[serde(default)]
    pub bootstrap: Vec<IpAddr>,
    /// Named upstream servers: friendly name to `https://` (DoH) or `quic://` (DoQ) URL.
    #[serde(default)]
    pub resolvers: BTreeMap<String, String>,
    /// Group name to resolver names, in failover order.
    #[serde(default)]
    pub upstream_groups: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub clients: Vec<ClientConfig>,
    /// Block list sources. Imported into the database on first start; after
    /// that the database is the source of truth (see `db.rs`).
    #[serde(default)]
    pub lists: BTreeMap<String, ListConfig>,
    /// Allow list sources, same shape as `lists`. Imported once, like `lists`.
    #[serde(default)]
    pub allow_lists: BTreeMap<String, ListConfig>,
    /// How long query history and minute rollups are kept.
    #[serde(default)]
    pub retention: Retention,
    /// Block lists applied to clients that do not match any CIDR. These are
    /// the "global" lists for unknown clients.
    #[serde(default)]
    pub default_block_lists: Vec<String>,
    #[serde(default)]
    pub rewrites: Vec<RewriteConfig>,
    /// Global allow entries. Suffix match, always wins over block lists.
    #[serde(default)]
    pub allow_domains: Vec<String>,
    /// Global custom blocks. Suffix match.
    #[serde(default)]
    pub block_domains: Vec<String>,
    #[serde(default = "default_group_name")]
    pub default_upstream_group: String,
}

fn default_group_name() -> String {
    DEFAULT_GROUP.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    /// Days of per-query history kept in the `queries` table.
    #[serde(default = "default_query_days")]
    pub query_days: u32,
    /// Days of per-minute rollups kept in the `minute_stats` table.
    #[serde(default = "default_stats_days")]
    pub stats_days: u32,
}

fn default_query_days() -> u32 {
    30
}

fn default_stats_days() -> u32 {
    365
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            query_days: default_query_days(),
            stats_days: default_stats_days(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listen {
    #[serde(default = "default_dns_listen")]
    pub dns: String,
    #[serde(default = "default_http_listen")]
    pub http: String,
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            dns: default_dns_listen(),
            http: default_http_listen(),
        }
    }
}

fn default_dns_listen() -> String {
    "0.0.0.0:53".to_string()
}

fn default_http_listen() -> String {
    "0.0.0.0:8080".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub name: String,
    #[serde(default)]
    pub cidrs: Vec<String>,
    /// Upstream group for this client. Falls back to `default_upstream_group`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_group: Option<String>,
    #[serde(default)]
    pub block_lists: Vec<String>,
    #[serde(default)]
    pub allow_domains: Vec<String>,
    #[serde(default)]
    pub block_domains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListConfig {
    /// URLs (http or https) or local file paths.
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub format: ListFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListFormat {
    /// Detect each line: `||x^` is ABP, a leading IP is hosts, otherwise plain.
    #[default]
    Auto,
    Hosts,
    Plain,
    Abp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewriteConfig {
    /// Exact name, or `*.suffix` for a suffix wildcard.
    pub domain: String,
    /// IPv4 or IPv6 address.
    pub answer: String,
}

impl Config {
    pub fn parse_yaml(text: &str) -> Result<Self> {
        Ok(Self::parse_counting(text)?.0)
    }

    /// Parses and validates, moving raw upstream URLs found in groups (the old
    /// format) into `resolvers`. Also returns how many resolvers that created.
    fn parse_counting(text: &str) -> Result<(Self, usize)> {
        let mut cfg: Config = serde_yaml::from_str(text).context("parsing config YAML")?;
        let migrated = cfg.migrate_legacy_upstreams();
        cfg.validate()?;
        Ok((cfg, migrated))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        Self::parse_yaml(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    /// Startup load. When the file still lists raw upstream URLs in its groups,
    /// the old file is kept as `config.yaml.bak` and the migrated shape is saved.
    /// If that backup or save fails, a warning is logged and the migrated config
    /// still runs. It is saved on the next change.
    pub fn load_and_migrate(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let (cfg, migrated) = Self::parse_counting(&text)
            .with_context(|| format!("invalid config {}", path.display()))?;
        if migrated > 0 {
            let backup = path.with_extension("yaml.bak");
            let saved = std::fs::write(&backup, &text)
                .with_context(|| format!("writing {}", backup.display()))
                .and_then(|()| cfg.save(path));
            match saved {
                Ok(()) => tracing::info!(
                    resolvers = migrated,
                    backup = %backup.display(),
                    "moved upstream URLs into named resolvers"
                ),
                Err(err) => tracing::warn!(
                    error = %format!("{err:#}"),
                    "could not save the migrated config; it will be saved on the next change"
                ),
            }
        }
        Ok(cfg)
    }

    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml::to_string(self).context("serializing config")
    }

    /// Validates and writes the config atomically (temp file, then rename).
    /// The temp file sits next to `path`, so `path` must live in a directory the
    /// process can write to. A bind-mounted single file cannot be renamed over.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let tmp = path.with_extension("yaml.tmp");
        std::fs::write(&tmp, self.to_yaml()?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Copies `seed` to `path` when `path` does not exist yet. Returns true when a copy happened.
    pub fn seed_if_missing(path: &Path, seed: &Path) -> Result<bool> {
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::copy(seed, path)
            .with_context(|| format!("seeding {} from {}", path.display(), seed.display()))?;
        Ok(true)
    }

    /// Checks cross-references and value syntax. Everything the runtime relies
    /// on is verified here so that a bad config is rejected before it is applied.
    pub fn validate(&self) -> Result<()> {
        self.listen
            .dns
            .parse::<SocketAddr>()
            .with_context(|| format!("listen.dns '{}' is not a socket address", self.listen.dns))?;
        self.listen
            .http
            .parse::<SocketAddr>()
            .with_context(|| format!("listen.http '{}' is not a socket address", self.listen.http))?;

        if !self.upstream_groups.contains_key(&self.default_upstream_group) {
            bail!(
                "default_upstream_group '{}' is not defined in upstream_groups",
                self.default_upstream_group
            );
        }

        let mut needs_bootstrap = false;
        let mut url_owner: HashMap<&str, &str> = HashMap::new();
        for (name, raw) in &self.resolvers {
            check_resolver_name(name)?;
            let url = parse_upstream_url(raw).with_context(|| format!("resolver '{name}'"))?;
            if let Some(other) = url_owner.insert(raw.as_str(), name.as_str()) {
                bail!("resolvers '{other}' and '{name}' use the same URL");
            }
            if url.host_str().unwrap_or_default().parse::<IpAddr>().is_err() {
                needs_bootstrap = true;
            }
        }
        for (group, names) in &self.upstream_groups {
            if group.trim().is_empty() {
                bail!("upstream group names cannot be empty");
            }
            if names.is_empty() {
                bail!("upstream group '{group}' has no resolvers");
            }
            for name in names {
                if !self.resolvers.contains_key(name) {
                    bail!("upstream group '{group}' uses unknown resolver '{name}'");
                }
            }
        }
        if needs_bootstrap && self.bootstrap.is_empty() {
            bail!("bootstrap is required because a resolver is given by hostname");
        }

        let mut client_names = HashSet::new();
        for client in &self.clients {
            if client.name.trim().is_empty() {
                bail!("client names cannot be empty");
            }
            if !client_names.insert(client.name.as_str()) {
                bail!("duplicate client name '{}'", client.name);
            }
            for cidr in &client.cidrs {
                parse_cidr(cidr)
                    .map_err(|e| anyhow::anyhow!("client '{}': {e}", client.name))?;
            }
            if let Some(group) = &client.upstream_group {
                if !self.upstream_groups.contains_key(group) {
                    bail!("client '{}' uses unknown upstream_group '{group}'", client.name);
                }
            }
            validate_domain_entries(&client.allow_domains, &format!("client '{}' allow_domains", client.name))?;
            validate_domain_entries(&client.block_domains, &format!("client '{}' block_domains", client.name))?;
        }

        validate_list_map(&self.lists, "lists")?;
        validate_list_map(&self.allow_lists, "allow_lists")?;

        if self.retention.query_days == 0 || self.retention.stats_days == 0 {
            bail!("retention.query_days and retention.stats_days must be at least 1");
        }

        for rewrite in &self.rewrites {
            let domain = normalize_name(&rewrite.domain);
            let body = domain.strip_prefix("*.").unwrap_or(&domain);
            if !is_valid_domain(body) {
                bail!("rewrite domain '{}' is not valid", rewrite.domain);
            }
            rewrite
                .answer
                .parse::<IpAddr>()
                .with_context(|| format!("rewrite for '{}' has an invalid answer", rewrite.domain))?;
        }

        validate_domain_entries(&self.allow_domains, "allow_domains")?;
        validate_domain_entries(&self.block_domains, "block_domains")?;
        Ok(())
    }

    /// Checks that every client and default block-list reference names a list
    /// that exists. The caller supplies the names, because the database owns
    /// the lists. The API calls this before it accepts a config. The DNS
    /// runtime tolerates a missing name instead, so deleting a list never
    /// stops DNS.
    pub fn validate_list_refs(&self, block_names: &HashSet<String>) -> Result<()> {
        for client in &self.clients {
            for list in &client.block_lists {
                if !block_names.contains(list) {
                    bail!("client '{}' references unknown list '{list}'", client.name);
                }
            }
        }
        for list in &self.default_block_lists {
            if !block_names.contains(list) {
                bail!("default_block_lists references unknown list '{list}'");
            }
        }
        Ok(())
    }

    /// The URLs of a group's resolvers, in the group's failover order.
    pub fn group_urls(&self, group: &str) -> Vec<String> {
        self.upstream_groups
            .get(group)
            .map(|names| names.iter().filter_map(|n| self.resolvers.get(n).cloned()).collect())
            .unwrap_or_default()
    }

    /// Every distinct upstream URL referenced by any group, in config order.
    pub fn upstream_urls(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for group in self.upstream_groups.keys() {
            for url in self.group_urls(group) {
                if seen.insert(url.clone()) {
                    out.push(url);
                }
            }
        }
        out
    }

    /// The resolver name for a server URL, when a resolver has that URL.
    pub fn resolver_name_for_url(&self, url: &str) -> Option<&str> {
        self.resolvers
            .iter()
            .find(|(_, u)| u.as_str() == url)
            .map(|(name, _)| name.as_str())
    }

    /// Renames a resolver and every group entry that lists it. Does nothing
    /// when `old` is not a resolver.
    pub fn rename_resolver(&mut self, old: &str, new: &str) {
        let Some(url) = self.resolvers.remove(old) else {
            return;
        };
        self.resolvers.insert(new.to_string(), url);
        for names in self.upstream_groups.values_mut() {
            for name in names.iter_mut().filter(|n| n.as_str() == old) {
                *name = new.to_string();
            }
        }
    }

    /// Moves raw upstream URLs out of the groups (the old format) into
    /// `resolvers`, named from their hostnames. Anything that is neither a
    /// resolver name nor a usable URL is left for `validate` to report.
    /// Returns how many resolvers were created.
    pub(crate) fn migrate_legacy_upstreams(&mut self) -> usize {
        let mut created = 0;
        let groups = std::mem::take(&mut self.upstream_groups);
        for (group, entries) in groups {
            let mut names = Vec::with_capacity(entries.len());
            for entry in entries {
                if self.resolvers.contains_key(&entry) {
                    names.push(entry);
                    continue;
                }
                let Ok(url) = parse_upstream_url(&entry) else {
                    names.push(entry);
                    continue;
                };
                let name = match self.resolver_name_for_url(&entry).map(str::to_string) {
                    Some(existing) => existing,
                    None => {
                        created += 1;
                        let name = unique_resolver_name(&self.resolvers, &url);
                        self.resolvers.insert(name.clone(), entry);
                        name
                    }
                };
                names.push(name);
            }
            self.upstream_groups.insert(group, names);
        }
        created
    }
}

/// Rejects resolver names that are empty or that look like a URL, since a
/// URL-shaped name could not be told apart from a raw URL in an old config.
pub fn check_resolver_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("resolver names cannot be empty");
    }
    if name.contains("://") {
        bail!("resolver name '{name}' must not look like a URL");
    }
    Ok(())
}

/// `doh` or `doq`, from the URL scheme. None for anything else.
pub fn protocol_of(url: &str) -> Option<&'static str> {
    if url.starts_with("https://") {
        Some("doh")
    } else if url.starts_with("quic://") {
        Some("doq")
    } else {
        None
    }
}

/// A resolver name for a migrated URL: the host, plus the path when it is not
/// the standard `/dns-query`. A taken name gets the protocol, then a number.
fn unique_resolver_name(taken: &BTreeMap<String, String>, url: &Url) -> String {
    let host = url.host_str().unwrap_or("resolver");
    let path = url.path().trim_matches('/');
    let base = if path.is_empty() || path == "dns-query" {
        host.to_string()
    } else {
        format!("{host}/{path}")
    };
    if !taken.contains_key(&base) {
        return base;
    }
    let proto = if url.scheme() == "quic" { "DoQ" } else { "DoH" };
    let tagged = format!("{base} ({proto})");
    if !taken.contains_key(&tagged) {
        return tagged;
    }
    (2..)
        .map(|n| format!("{tagged} {n}"))
        .find(|candidate| !taken.contains_key(candidate))
        .unwrap_or_default()
}

/// Parses an upstream URL. Accepts `https://` (DNS-over-HTTPS) and
/// `quic://` (DNS-over-QUIC, port 853 by default). Both require a host.
pub fn parse_upstream_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).with_context(|| format!("'{raw}' is not a valid URL"))?;
    if url.scheme() != "https" && url.scheme() != "quic" {
        bail!("'{raw}' must use https:// (DoH) or quic:// (DoQ)");
    }
    if url.host_str().is_none() {
        bail!("'{raw}' has no host");
    }
    Ok(url)
}

fn validate_list_map(lists: &BTreeMap<String, ListConfig>, section: &str) -> Result<()> {
    for (name, list) in lists {
        if name.trim().is_empty() {
            bail!("{section}: list names cannot be empty");
        }
        if list.sources.is_empty() {
            bail!("{section}: list '{name}' has no sources");
        }
        for source in &list.sources {
            if source.trim().is_empty() {
                bail!("{section}: list '{name}' has an empty source");
            }
        }
    }
    Ok(())
}

fn validate_domain_entries(entries: &[String], context: &str) -> Result<()> {
    for entry in entries {
        let domain = normalize_rule_domain(entry);
        if !is_valid_domain(&domain) {
            bail!("{context}: '{entry}' is not a valid domain");
        }
    }
    Ok(())
}

/// Convenience used by the runtime builder when it needs parsed networks.
pub fn parse_client_cidrs(client: &ClientConfig) -> Result<Vec<IpNet>> {
    client
        .cidrs
        .iter()
        .map(|c| parse_cidr(c).map_err(|e| anyhow::anyhow!("client '{}': {e}", client.name)))
        .collect()
}

#[cfg(test)]
mod seed_tests {
    use super::*;

    #[test]
    fn seeds_missing_config_and_keeps_existing() {
        let dir = std::env::temp_dir().join(format!("dnsui-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seed = dir.join("seed.yaml");
        std::fs::write(&seed, "listen:\n  dns: \"0.0.0.0:53\"\n").unwrap();
        let target = dir.join("data").join("config.yaml");

        assert!(Config::seed_if_missing(&target, &seed).unwrap());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), std::fs::read_to_string(&seed).unwrap());

        std::fs::write(&target, "edited: true\n").unwrap();
        assert!(!Config::seed_if_missing(&target, &seed).unwrap());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "edited: true\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
listen:
  dns: "0.0.0.0:5353"
  http: "127.0.0.1:8080"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
  nextdns: ["https://abc123.dns.nextdns.io/dns-query"]
clients:
  - name: Laptop
    cidrs: ["192.0.2.10/32"]
    upstream_group: nextdns
    block_lists: ["ads"]
lists:
  ads:
    sources: ["https://example.com/hosts.txt"]
rewrites:
  - domain: "*.example.test"
    answer: "203.0.113.116"
"#;

    #[test]
    fn sample_parses_and_validates() {
        let cfg = Config::parse_yaml(SAMPLE).expect("sample should be valid");
        assert_eq!(cfg.clients.len(), 1);
        assert_eq!(cfg.default_upstream_group, "default");
    }

    #[test]
    fn rejects_unknown_group_reference() {
        let bad = SAMPLE.replace("upstream_group: nextdns", "upstream_group: nope");
        assert!(Config::parse_yaml(&bad).is_err());
    }

    #[test]
    fn rejects_plain_http_upstream() {
        let bad = SAMPLE.replace("https://dns10.quad9.net", "http://dns10.quad9.net");
        assert!(Config::parse_yaml(&bad).is_err());
    }

    #[test]
    fn hostname_upstream_requires_bootstrap() {
        let bad = SAMPLE.replace("bootstrap: [\"9.9.9.10\"]", "bootstrap: []");
        assert!(Config::parse_yaml(&bad).is_err());
    }

    #[test]
    fn retention_defaults_and_overrides() {
        let cfg = Config::parse_yaml(SAMPLE).unwrap();
        assert_eq!(cfg.retention.query_days, 30);
        assert_eq!(cfg.retention.stats_days, 365);
        let custom = SAMPLE.replace(
            "bootstrap:",
            "retention:\n  query_days: 7\n  stats_days: 90\nallow_lists: {}\nbootstrap:",
        );
        let cfg = Config::parse_yaml(&custom).unwrap();
        assert_eq!(cfg.retention.query_days, 7);
        assert_eq!(cfg.retention.stats_days, 90);
    }

    #[test]
    fn zero_retention_is_rejected() {
        let bad = SAMPLE.replace("bootstrap:", "retention:\n  query_days: 0\nbootstrap:");
        assert!(Config::parse_yaml(&bad).is_err());
    }

    #[test]
    fn allow_lists_parse_like_block_lists() {
        let text = SAMPLE.replace(
            "bootstrap:",
            "allow_lists:\n  trusted:\n    sources: [\"https://example.com/allow.txt\"]\nbootstrap:",
        );
        let cfg = Config::parse_yaml(&text).unwrap();
        assert!(cfg.allow_lists.contains_key("trusted"));
        let empty = text.replace("sources: [\"https://example.com/allow.txt\"]", "sources: []");
        assert!(Config::parse_yaml(&empty).is_err());
    }

    #[test]
    fn list_references_are_checked_against_the_given_names() {
        let cfg = Config::parse_yaml(SAMPLE).unwrap();
        let mut names = std::collections::HashSet::new();
        assert!(cfg.validate_list_refs(&names).is_err(), "ads is referenced but unknown");
        names.insert("ads".to_string());
        assert!(cfg.validate_list_refs(&names).is_ok());
    }

}

#[cfg(test)]
mod upstream_url_tests {
    use super::parse_upstream_url;

    #[test]
    fn accepts_doh_and_doq() {
        assert!(parse_upstream_url("https://dns10.quad9.net/dns-query").is_ok());
        assert!(parse_upstream_url("quic://def456.dns.nextdns.io").is_ok());
    }

    #[test]
    fn rejects_plain_and_tls() {
        assert!(parse_upstream_url("9.9.9.9").is_err());
        assert!(parse_upstream_url("tls://dns.quad9.net").is_err());
        assert!(parse_upstream_url("http://dns.quad9.net/dns-query").is_err());
        assert!(parse_upstream_url("quic://").is_err());
    }
}

#[cfg(test)]
mod resolver_tests {
    use super::*;

    const LEGACY: &str = r#"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
  laptop: ["quic://abc123.dns.nextdns.io", "https://dns.nextdns.io/abc123"]
  other: ["https://dns10.quad9.net/dns-query"]
"#;

    #[test]
    fn raw_urls_in_groups_become_named_resolvers() {
        let (cfg, created) = Config::parse_counting(LEGACY).unwrap();
        assert_eq!(created, 3);
        assert_eq!(cfg.resolvers["dns10.quad9.net"], "https://dns10.quad9.net/dns-query");
        assert_eq!(cfg.resolvers["abc123.dns.nextdns.io"], "quic://abc123.dns.nextdns.io");
        assert_eq!(cfg.resolvers["dns.nextdns.io/abc123"], "https://dns.nextdns.io/abc123");
        assert_eq!(cfg.upstream_groups["laptop"], vec!["abc123.dns.nextdns.io", "dns.nextdns.io/abc123"]);
        // The same URL in two groups is one resolver.
        assert_eq!(cfg.upstream_groups["other"], vec!["dns10.quad9.net"]);
    }

    #[test]
    fn migration_is_idempotent() {
        let (first, _) = Config::parse_counting(LEGACY).unwrap();
        let (second, created) = Config::parse_counting(&first.to_yaml().unwrap()).unwrap();
        assert_eq!(created, 0);
        assert_eq!(first, second);
    }

    #[test]
    fn same_host_in_two_protocols_gets_distinct_names() {
        let text = r#"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns.example.net/dns-query", "quic://dns.example.net"]
"#;
        let cfg = Config::parse_yaml(text).unwrap();
        assert_eq!(cfg.resolvers.len(), 2);
        assert!(cfg.resolvers.contains_key("dns.example.net"));
        assert!(cfg.resolvers.contains_key("dns.example.net (DoQ)"));
    }

    #[test]
    fn rejects_bad_resolver_setups() {
        let base = "bootstrap: [\"9.9.9.10\"]\n";
        for bad in [
            // Two names for one URL.
            "resolvers: {a: \"https://x.example/dns-query\", b: \"https://x.example/dns-query\"}\nupstream_groups: {default: [a]}",
            // A group naming a resolver that does not exist.
            "resolvers: {a: \"https://x.example/dns-query\"}\nupstream_groups: {default: [missing]}",
            // A name that looks like a URL.
            "resolvers: {\"https://x.example\": \"https://x.example/dns-query\"}\nupstream_groups: {default: [\"https://x.example\"]}",
            // A group with no resolvers.
            "resolvers: {a: \"https://x.example/dns-query\"}\nupstream_groups: {default: []}",
        ] {
            assert!(Config::parse_yaml(&format!("{base}{bad}")).is_err(), "{bad}");
        }
    }

    #[test]
    fn group_urls_follow_the_group_order() {
        let (cfg, _) = Config::parse_counting(LEGACY).unwrap();
        assert_eq!(
            cfg.group_urls("laptop"),
            vec!["quic://abc123.dns.nextdns.io", "https://dns.nextdns.io/abc123"]
        );
        assert!(cfg.group_urls("nope").is_empty());
        assert_eq!(cfg.upstream_urls().len(), 3);
        assert_eq!(
            cfg.resolver_name_for_url("quic://abc123.dns.nextdns.io"),
            Some("abc123.dns.nextdns.io")
        );
        assert_eq!(cfg.resolver_name_for_url("quic://unknown.example"), None);
    }

    #[test]
    fn rename_carries_into_every_group() {
        let (mut cfg, _) = Config::parse_counting(LEGACY).unwrap();
        cfg.rename_resolver("dns10.quad9.net", "Quad9");
        assert_eq!(cfg.upstream_groups["default"], vec!["Quad9"]);
        assert_eq!(cfg.upstream_groups["other"], vec!["Quad9"]);
        assert_eq!(cfg.resolvers["Quad9"], "https://dns10.quad9.net/dns-query");
        assert!(!cfg.resolvers.contains_key("dns10.quad9.net"));
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn example_configs_are_valid_and_already_use_resolvers() {
        for text in [
            include_str!("../config.example.yaml"),
            include_str!("../../deploy/config.example.yaml"),
        ] {
            let (cfg, migrated) = Config::parse_counting(text).unwrap();
            assert_eq!(migrated, 0, "the examples must not rely on the migration");
            assert_eq!(cfg.group_urls("nextdns").len(), 2);
        }
    }

    #[test]
    fn protocol_comes_from_the_scheme() {
        assert_eq!(protocol_of("https://a.example/dns-query"), Some("doh"));
        assert_eq!(protocol_of("quic://a.example"), Some("doq"));
        assert_eq!(protocol_of("-"), None);
    }

    #[test]
    fn startup_load_backs_up_and_saves_the_migrated_file() {
        let dir = std::env::temp_dir().join(format!("dnsui-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, LEGACY).unwrap();

        let cfg = Config::load_and_migrate(&path).unwrap();
        assert_eq!(cfg.resolvers.len(), 3);
        assert_eq!(std::fs::read_to_string(dir.join("config.yaml.bak")).unwrap(), LEGACY);
        assert!(std::fs::read_to_string(&path).unwrap().contains("resolvers:"));
        assert_eq!(Config::load(&path).unwrap(), cfg);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
