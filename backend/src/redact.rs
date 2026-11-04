//! Viewer-role redaction. Domain names, client names and upstream URLs are
//! replaced by short, stable, non-reversible hashes. The TLD is kept so that
//! traffic shape stays readable.

use sha2::{Digest, Sha256};

use crate::models::{QueryItem, StatsResponse};

/// Eight hex characters of a salted SHA-256.
pub fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(format!("dnsui-redact-v1:{value}").as_bytes());
    hex::encode(&digest[..4])
}

/// `example.com` becomes `1a2b3c4d.com`. A single-label name becomes just the hash.
pub fn redact_domain(domain: &str) -> String {
    let hash = short_hash(domain);
    match domain.rsplit_once('.') {
        Some((_, tld)) if !tld.is_empty() => format!("{hash}.{tld}"),
        _ => hash,
    }
}

pub fn redact_client(name: &str) -> String {
    format!("Client-{}", short_hash(name))
}

pub fn redact_upstream(url: &str) -> String {
    format!("upstream-{}", short_hash(url))
}

/// Replaces a list source (URL or file path) for viewers.
pub fn redact_source(source: &str) -> String {
    format!("source-{}", short_hash(source))
}

/// Keeps the rule kind and the list name, and redacts the domain part.
/// Examples: `block:global:x.com` becomes `block:global:<hash>.com`, and
/// `list:ads:x.com` becomes `list:ads:<hash>.com`.
pub fn redact_rule(rule: &str) -> String {
    if rule.is_empty() {
        return String::new();
    }
    match rule.rsplit_once(':') {
        Some((prefix, domain)) if !domain.is_empty() => {
            format!("{prefix}:{}", redact_domain(domain))
        }
        _ => rule.to_string(),
    }
}

pub fn redact_stats(stats: &mut StatsResponse) {
    // Block reasons are list names and company or country totals, so they stay visible.
    for entry in stats
        .top_domains
        .iter_mut()
        .chain(stats.top_blocked.iter_mut())
        .chain(stats.root_domains.iter_mut())
    {
        entry.name = redact_domain(&entry.name);
    }
    for up in &mut stats.upstreams {
        up.name = redact_upstream(&up.name);
    }
}

pub fn redact_query(item: &mut QueryItem) {
    item.domain = redact_domain(&item.domain);
    item.client = redact_client(&item.client);
    item.upstream = redact_upstream(&item.upstream);
    item.rule = redact_rule(&item.rule);
    item.client_ip = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_keeps_tld_and_hides_name() {
        let r = redact_domain("secret.example.com");
        assert!(r.ends_with(".com"));
        assert!(!r.contains("secret"));
        assert_eq!(r, redact_domain("secret.example.com"));
    }

    #[test]
    fn rule_redaction_keeps_kind() {
        assert!(redact_rule("list:ads:tracker.net").starts_with("list:ads:"));
        assert!(redact_rule("list:ads:tracker.net").ends_with(".net"));
        assert_eq!(redact_rule(""), "");
    }
}
