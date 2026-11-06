//! Block list parsing (hosts, plain domains, ABP `||domain^`) and loading
//! (HTTP download with an on-disk cache, or local file paths).

use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;
use tracing::{info, warn};

use crate::config::{ListConfig, ListFormat};
use crate::rules::{is_valid_domain, normalize_name};

/// Hostnames that appear in hosts files but are not blocking targets.
const HOSTS_RESERVED: &[&str] = &[
    "localhost",
    "localhost.localdomain",
    "local",
    "broadcasthost",
    "ip6-localhost",
    "ip6-loopback",
    "ip6-localnet",
    "ip6-mcastprefix",
    "ip6-allnodes",
    "ip6-allrouters",
    "ip6-allhosts",
];

/// ABP modifiers that narrow a rule to a context this resolver cannot see.
/// Such rules are skipped rather than widened into a global block.
const UNSUPPORTED_ABP_MODIFIERS: &[&str] = &[
    "domain=",
    "client=",
    "denyallow=",
    "dnstype=",
    "to=",
    "from=",
    "method=",
    "header=",
    "badfilter",
];

/// Result of parsing a whole list text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedList {
    pub domains: HashSet<String>,
    /// Lines that were not blank or comments and could not be used.
    pub skipped: usize,
}

/// Parses list text in the given format.
pub fn parse_list(text: &str, format: ListFormat) -> ParsedList {
    let mut out = ParsedList::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        match parse_line(line, format) {
            Some(names) => {
                for name in names {
                    out.domains.insert(name);
                }
            }
            None => out.skipped += 1,
        }
    }
    out
}

/// Returns `None` when the line is unsupported (counted as skipped), or
/// `Some(names)`, where an empty vector means the line is intentionally ignored.
fn parse_line(line: &str, format: ListFormat) -> Option<Vec<String>> {
    // Cosmetic filters and exception rules are never domain blocks.
    if line.contains("##") || line.contains("#@#") || line.contains("#?#") || line.contains("#$#") {
        return None;
    }
    if line.starts_with("@@") {
        return None;
    }
    match format {
        ListFormat::Abp => parse_abp(line),
        ListFormat::Hosts => parse_hosts(strip_inline_comment(line)),
        ListFormat::Plain => parse_plain(strip_inline_comment(line)),
        ListFormat::Auto => {
            if line.starts_with("||") {
                parse_abp(line)
            } else {
                let body = strip_inline_comment(line);
                let first = body.split_whitespace().next()?;
                if first.parse::<IpAddr>().is_ok() {
                    parse_hosts(body)
                } else {
                    parse_plain(body)
                }
            }
        }
    }
}

fn strip_inline_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => line[..i].trim_end(),
        None => line,
    }
}

fn parse_abp(line: &str) -> Option<Vec<String>> {
    let rest = line.strip_prefix("||")?;
    let host_end = rest
        .find(['^', '/', '$', '|', '*', ':'])
        .unwrap_or(rest.len());
    let host = &rest[..host_end];
    let tail = &rest[host_end..];

    // Anything after the host must be `^`, optionally followed by `|` or `$options`.
    // Path, port, wildcard, or anchor rules are not domain-wide blocks.
    let options = if tail.is_empty() {
        None
    } else {
        let after_caret = tail.strip_prefix('^')?;
        if after_caret.is_empty() || after_caret.starts_with('|') {
            None
        } else {
            Some(after_caret.strip_prefix('$')?)
        }
    };

    if let Some(opts) = options {
        let unsupported = opts
            .split(',')
            .map(str::trim)
            .any(|opt| UNSUPPORTED_ABP_MODIFIERS.iter().any(|p| opt.starts_with(p)));
        if unsupported {
            return None;
        }
    }

    let name = normalize_name(host);
    if is_valid_domain(&name) {
        Some(vec![name])
    } else {
        None
    }
}

fn parse_hosts(body: &str) -> Option<Vec<String>> {
    let mut tokens = body.split_whitespace();
    let ip: IpAddr = tokens.next()?.parse().ok()?;
    if !(ip.is_unspecified() || ip.is_loopback()) {
        return None;
    }
    let names: Vec<String> = tokens
        .map(normalize_name)
        .filter(|n| !HOSTS_RESERVED.contains(&n.as_str()))
        .collect();
    if names.is_empty() {
        // Only reserved names (e.g. "127.0.0.1 localhost"): nothing to block.
        return Some(Vec::new());
    }
    if names.iter().all(|n| is_valid_domain(n)) {
        Some(names)
    } else {
        None
    }
}

fn parse_plain(body: &str) -> Option<Vec<String>> {
    let token = body.split_whitespace().next()?;
    if token.contains('/') || token.contains(':') {
        return None;
    }
    let token = token.strip_prefix("*.").unwrap_or(token);
    let name = normalize_name(token);
    if is_valid_domain(&name) {
        Some(vec![name])
    } else {
        None
    }
}

/// Everything known about one loaded list.
#[derive(Debug, Clone)]
pub struct ListData {
    pub name: String,
    pub domains: Arc<HashSet<String>>,
    pub skipped: usize,
    pub sources_total: usize,
    pub sources_ok: usize,
    /// Human-readable problems, including sources served from cache.
    pub source_errors: Vec<String>,
    pub loaded_at: DateTime<Utc>,
}

/// Loads every list in a `name -> config` map concurrently. Lists that fail
/// entirely still appear (with zero domains and an error), so references stay valid.
pub async fn load_lists_map(
    http: &reqwest::Client,
    lists: &BTreeMap<String, ListConfig>,
    cache_dir: &Path,
) -> BTreeMap<String, Arc<ListData>> {
    let mut set = JoinSet::new();
    for (name, cfg) in lists {
        let http = http.clone();
        let name = name.clone();
        let cfg = cfg.clone();
        let cache_dir = cache_dir.to_path_buf();
        set.spawn(async move {
            let data = load_list(&http, &name, &cfg, &cache_dir).await;
            (name, data)
        });
    }
    let mut out = BTreeMap::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((name, data)) => {
                info!(
                    list = %name,
                    domains = data.domains.len(),
                    skipped = data.skipped,
                    "list loaded"
                );
                out.insert(name, Arc::new(data));
            }
            Err(e) => warn!(error = %e, "list loader task failed"),
        }
    }
    out
}

/// Loads one list. Each source is parsed separately and the results are merged.
pub async fn load_list(
    http: &reqwest::Client,
    name: &str,
    cfg: &ListConfig,
    cache_dir: &Path,
) -> ListData {
    let mut domains: HashSet<String> = HashSet::new();
    let mut skipped = 0usize;
    let mut sources_ok = 0usize;
    let mut errors = Vec::new();

    for source in &cfg.sources {
        match fetch_source(http, source, cache_dir).await {
            Ok((text, from_cache_note)) => {
                let parsed = parse_list(&text, cfg.format);
                skipped += parsed.skipped;
                domains.extend(parsed.domains);
                sources_ok += 1;
                if let Some(note) = from_cache_note {
                    errors.push(format!("{source}: {note}"));
                }
            }
            Err(e) => {
                warn!(list = name, source, error = %e, "list source failed");
                errors.push(format!("{source}: {e}"));
            }
        }
    }

    ListData {
        name: name.to_string(),
        domains: Arc::new(domains),
        skipped,
        sources_total: cfg.sources.len(),
        sources_ok,
        source_errors: errors,
        loaded_at: Utc::now(),
    }
}

/// Returns the text of one source. For URLs, a successful download is written
/// to the cache, and a failed download falls back to the cached copy. The
/// second value is a note when the cached copy was used.
pub(crate) async fn fetch_source(
    http: &reqwest::Client,
    source: &str,
    cache_dir: &Path,
) -> Result<(String, Option<String>), String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let cache_path = cache_path_for(cache_dir, source);
        match download(http, source).await {
            Ok(text) => {
                if let Err(e) = write_cache(&cache_path, &text).await {
                    warn!(source, error = %e, "could not write list cache");
                }
                Ok((text, None))
            }
            Err(download_err) => match tokio::fs::read_to_string(&cache_path).await {
                Ok(cached) => Ok((
                    cached,
                    Some(format!("download failed ({download_err}); using cached copy")),
                )),
                Err(_) => Err(format!("download failed: {download_err}")),
            },
        }
    } else {
        tokio::fs::read_to_string(source)
            .await
            .map(|t| (t, None))
            .map_err(|e| format!("read failed: {e}"))
    }
}

async fn download(http: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = http
        .get(url)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = resp.error_for_status().map_err(|e| e.to_string())?;
    resp.text().await.map_err(|e| e.to_string())
}

pub fn cache_path_for(cache_dir: &Path, source: &str) -> PathBuf {
    let digest = Sha256::digest(source.as_bytes());
    cache_dir.join(format!("{}.txt", hex::encode(digest)))
}

async fn write_cache(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, text).await?;
    tokio::fs::rename(&tmp, path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_format_parses_both_loopback_and_unspecified() {
        let text = "\
# comment line
0.0.0.0 ads.example.com
127.0.0.1 tracker.example.net   # inline comment
0.0.0.0 localhost
0.0.0.0 a.one.com b.one.com
192.168.1.1 router.lan
";
        let parsed = parse_list(text, ListFormat::Hosts);
        assert!(parsed.domains.contains("ads.example.com"));
        assert!(parsed.domains.contains("tracker.example.net"));
        assert!(parsed.domains.contains("a.one.com"));
        assert!(parsed.domains.contains("b.one.com"));
        assert!(!parsed.domains.contains("localhost"));
        // The 192.168.x line is not a blocking entry and is skipped.
        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.domains.len(), 4);
    }

    #[test]
    fn abp_lines_parse_and_skip_unsupported() {
        let text = "\
! Title: test
||doubleclick.net^
||ads.example.org^$important
||tracker.io^|
@@||allowed.example^
||site.com/path^
example.com##.banner
||regex-like.com^$domain=foo.com
||bad_host!^
";
        let parsed = parse_list(text, ListFormat::Abp);
        assert!(parsed.domains.contains("doubleclick.net"));
        assert!(parsed.domains.contains("ads.example.org"));
        assert!(parsed.domains.contains("tracker.io"));
        assert!(!parsed.domains.contains("allowed.example"));
        assert!(!parsed.domains.contains("site.com"));
        assert!(!parsed.domains.contains("regex-like.com"));
        // @@ exception, path rule, cosmetic rule, scoped rule, invalid host.
        assert_eq!(parsed.skipped, 5);
        assert_eq!(parsed.domains.len(), 3);
    }

    #[test]
    fn auto_detects_each_line_style() {
        let text = "\
||abp.example^
0.0.0.0 hosts.example
plain.example
";
        let parsed = parse_list(text, ListFormat::Auto);
        assert_eq!(parsed.domains.len(), 3);
        assert_eq!(parsed.skipped, 0);
    }

    #[test]
    fn plain_format_rejects_urls() {
        let parsed = parse_list("https://example.com/x\nexample.org\n", ListFormat::Plain);
        assert!(parsed.domains.contains("example.org"));
        assert_eq!(parsed.skipped, 1);
    }

    #[test]
    fn cache_path_is_stable_per_source() {
        let dir = Path::new("/tmp/cache-test");
        let a = cache_path_for(dir, "https://example.com/hosts.txt");
        let b = cache_path_for(dir, "https://example.com/hosts.txt");
        let c = cache_path_for(dir, "https://example.com/other.txt");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
