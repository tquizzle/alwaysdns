//! Configuration file model, parsing, and validation.
//!
//! The config is YAML on disk and JSON over the HTTP API. Both use the same
//! serde model, so `GET /api/config` returns exactly what `PUT /api/config`
//! accepts.

use std::collections::{BTreeMap, HashSet};
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
        let cfg: Config = serde_yaml::from_str(text).context("parsing config YAML")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        Self::parse_yaml(&text).with_context(|| format!("invalid config {}", path.display()))
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
        for (group, urls) in &self.upstream_groups {
            if group.trim().is_empty() {
                bail!("upstream group names cannot be empty");
            }
            if urls.is_empty() {
                bail!("upstream group '{group}' has no servers");
            }
            for raw in urls {
                let url = parse_upstream_url(raw)
                    .with_context(|| format!("upstream group '{group}'"))?;
                let host = url.host_str().unwrap_or_default();
                if host.parse::<IpAddr>().is_err() {
                    needs_bootstrap = true;
                }
            }
        }
        if needs_bootstrap && self.bootstrap.is_empty() {
            bail!("bootstrap is required because a DoH upstream is given by hostname");
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

    /// Every distinct upstream URL referenced by any group, in config order.
    pub fn upstream_urls(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for urls in self.upstream_groups.values() {
            for url in urls {
                if seen.insert(url.clone()) {
                    out.push(url.clone());
                }
            }
        }
        out
    }
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
