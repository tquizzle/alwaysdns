//! Pure matching primitives: domain normalization, parent-suffix matching,
//! CIDR lookup (most specific wins), and rewrite lookup (exact, then wildcard).

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

use anyhow::{Result, bail};
use ipnet::IpNet;

/// A set of domain names. Membership is checked with parent-suffix matching.
pub type DomainSet = HashSet<String>;

/// Lowercases, trims, and removes the trailing root dot.
pub fn normalize_name(raw: &str) -> String {
    raw.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Normalizes an allow/block entry. A leading `*.` is dropped, because suffix
/// matching already covers subdomains.
pub fn normalize_rule_domain(raw: &str) -> String {
    let name = normalize_name(raw);
    match name.strip_prefix("*.") {
        Some(rest) => rest.to_string(),
        None => name,
    }
}

/// True for a syntactically valid hostname made of LDH labels (plus `_`).
pub fn is_valid_domain(name: &str) -> bool {
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// Yields `name`, then each parent: `a.b.c` gives `a.b.c`, `b.c`, `c`.
pub fn suffixes(name: &str) -> impl Iterator<Item = &str> {
    let mut rest: Option<&str> = if name.is_empty() { None } else { Some(name) };
    std::iter::from_fn(move || {
        let cur = rest?;
        rest = cur.find('.').map(|i| &cur[i + 1..]);
        Some(cur)
    })
}

/// Returns the first suffix of `name` (the name itself or a parent) found in `set`.
pub fn match_suffix(set: &DomainSet, name: &str) -> Option<String> {
    if set.is_empty() {
        return None;
    }
    suffixes(name).find(|s| set.contains(*s)).map(str::to_string)
}

/// Parses `192.0.2.5` (as /32) or `10.0.0.0/24` into a network.
pub fn parse_cidr(raw: &str) -> Result<IpNet> {
    let raw = raw.trim();
    if raw.contains('/') {
        let net: IpNet = raw
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid CIDR '{raw}': {e}"))?;
        return Ok(net);
    }
    let ip: IpAddr = match raw.parse() {
        Ok(ip) => ip,
        Err(_) => bail!("invalid CIDR or IP '{raw}'"),
    };
    let prefix = if ip.is_ipv4() { 32 } else { 128 };
    Ok(IpNet::new(ip, prefix).expect("full-length prefix is always valid"))
}

/// Maps IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) back to IPv4 so that
/// they match IPv4 CIDRs.
pub fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => ip,
        },
        other => other,
    }
}

/// Maps a source address to a client index. Entries are kept sorted by prefix
/// length, longest first, so the first hit is the most specific.
#[derive(Debug, Default, Clone)]
pub struct CidrTable {
    entries: Vec<(IpNet, usize)>,
}

impl CidrTable {
    pub fn new(mut entries: Vec<(IpNet, usize)>) -> Self {
        // Stable sort: on equal prefix length the earlier config entry wins.
        entries.sort_by_key(|e| std::cmp::Reverse(e.0.prefix_len()));
        Self { entries }
    }

    pub fn lookup(&self, ip: IpAddr) -> Option<usize> {
        let ip = canonical_ip(ip);
        self.entries
            .iter()
            .find(|(net, _)| net.contains(&ip))
            .map(|(_, idx)| *idx)
    }
}

/// Local answers for names. Exact names are checked first, then the longest
/// `*.suffix` wildcard that matches.
#[derive(Debug, Default, Clone)]
pub struct RewriteTable {
    exact: HashMap<String, IpAddr>,
    /// (suffix without the leading `*.`, answer), longest suffix first.
    wildcard: Vec<(String, IpAddr)>,
}

impl RewriteTable {
    pub fn build(rules: &[(String, IpAddr)]) -> Result<Self> {
        let mut exact = HashMap::new();
        let mut wildcard = Vec::new();
        for (domain, answer) in rules {
            let name = normalize_name(domain);
            if let Some(suffix) = name.strip_prefix("*.") {
                if !is_valid_domain(suffix) {
                    bail!("rewrite wildcard '{domain}' is not valid");
                }
                wildcard.push((suffix.to_string(), *answer));
            } else {
                if !is_valid_domain(&name) {
                    bail!("rewrite domain '{domain}' is not valid");
                }
                exact.insert(name, *answer);
            }
        }
        wildcard.sort_by_key(|e| std::cmp::Reverse(e.0.len()));
        Ok(Self { exact, wildcard })
    }

    /// Returns the answer and the pattern that matched (for the query log).
    pub fn lookup(&self, name: &str) -> Option<(IpAddr, String)> {
        if let Some(answer) = self.exact.get(name) {
            return Some((*answer, name.to_string()));
        }
        for (suffix, answer) in &self.wildcard {
            if name.len() > suffix.len()
                && name.ends_with(suffix.as_str())
                && name.as_bytes()[name.len() - suffix.len() - 1] == b'.'
            {
                return Some((*answer, format!("*.{suffix}")));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn set(items: &[&str]) -> DomainSet {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cidr_most_specific_wins() {
        let table = CidrTable::new(vec![
            (parse_cidr("198.51.100.0/24").unwrap(), 1), // WiFi
            (parse_cidr("198.51.100.65/32").unwrap(), 0), // Laptop (more specific)
        ]);
        assert_eq!(table.lookup("198.51.100.65".parse().unwrap()), Some(0));
        assert_eq!(table.lookup("198.51.100.66".parse().unwrap()), Some(1));
        assert_eq!(table.lookup("192.168.1.1".parse().unwrap()), None);
    }

    #[test]
    fn cidr_matches_mapped_ipv6_and_bare_ip() {
        let table = CidrTable::new(vec![(parse_cidr("192.0.2.10").unwrap(), 3)]);
        assert_eq!(table.lookup("::ffff:192.0.2.10".parse().unwrap()), Some(3));
        assert_eq!(table.lookup("192.0.2.11".parse().unwrap()), None);
    }

    #[test]
    fn cidr_ipv6_and_invalid_input() {
        let table = CidrTable::new(vec![(parse_cidr("fd00::/64").unwrap(), 5)]);
        assert_eq!(table.lookup("fd00::42".parse().unwrap()), Some(5));
        assert!(parse_cidr("10.0.0.0/33").is_err());
        assert!(parse_cidr("not-an-ip").is_err());
    }

    #[test]
    fn rewrite_wildcard_matches_subdomains_not_apex() {
        let table = RewriteTable::build(&[
            ("*.example.test".to_string(), Ipv4Addr::new(10, 0, 1, 116).into()),
            ("dns.example.test".to_string(), Ipv4Addr::new(10, 0, 1, 54).into()),
        ])
        .unwrap();

        let (addr, pat) = table.lookup("nas.example.test").unwrap();
        assert_eq!(addr, IpAddr::from(Ipv4Addr::new(10, 0, 1, 116)));
        assert_eq!(pat, "*.example.test");

        // Deeper names match the wildcard.
        assert!(table.lookup("a.b.example.test").is_some());
        // The bare suffix does not match "*.example.test".
        assert!(table.lookup("loc").is_none());
        // Exact entries take precedence over the wildcard.
        let (addr, pat) = table.lookup("dns.example.test").unwrap();
        assert_eq!(addr, IpAddr::from(Ipv4Addr::new(10, 0, 1, 54)));
        assert_eq!(pat, "dns.example.test");
        // Unrelated names do not match.
        assert!(table.lookup("example.com").is_none());
        // A suffix that merely ends with the same characters must not match.
        assert!(table.lookup("xloc").is_none());
    }

    #[test]
    fn rewrite_longest_wildcard_wins() {
        let table = RewriteTable::build(&[
            ("*.home.example.test".to_string(), Ipv4Addr::new(10, 0, 0, 2).into()),
            ("*.example.test".to_string(), Ipv4Addr::new(10, 0, 0, 1).into()),
        ])
        .unwrap();
        let (addr, _) = table.lookup("box.home.example.test").unwrap();
        assert_eq!(addr, IpAddr::from(Ipv4Addr::new(10, 0, 0, 2)));
    }

    #[test]
    fn block_matches_name_and_every_parent() {
        let blocked = set(&["tracker.net", "ads.example.com"]);
        assert_eq!(
            match_suffix(&blocked, "tracker.net").as_deref(),
            Some("tracker.net")
        );
        assert_eq!(
            match_suffix(&blocked, "pixel.eu.tracker.net").as_deref(),
            Some("tracker.net")
        );
        assert_eq!(
            match_suffix(&blocked, "a.ads.example.com").as_deref(),
            Some("ads.example.com")
        );
        // Parent must match on a label boundary.
        assert_eq!(match_suffix(&blocked, "mytracker.net"), None);
        // A child of a blocked name does not block the parent.
        assert_eq!(match_suffix(&blocked, "example.com"), None);
    }

    #[test]
    fn suffix_iteration_order() {
        let v: Vec<&str> = suffixes("a.b.c").collect();
        assert_eq!(v, vec!["a.b.c", "b.c", "c"]);
        assert_eq!(suffixes("").count(), 0);
    }

    #[test]
    fn normalization() {
        assert_eq!(normalize_name(" Example.COM. "), "example.com");
        assert_eq!(normalize_rule_domain("*.Example.com"), "example.com");
        assert!(is_valid_domain("a-b.example.com"));
        assert!(!is_valid_domain("-bad.example.com"));
        assert!(!is_valid_domain("bad..example.com"));
        assert!(!is_valid_domain("has space.com"));
    }
}
