//! Dashboard breakdowns computed from stored query rows: root domains, company
//! totals (GAFAM), and destination countries from a MaxMind GeoLite2 database.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;

use maxminddb::{Reader, geoip2};

/// Public suffixes made of two labels. `example.co.uk` has root `example.co.uk`,
/// not `co.uk`. This covers the common cases, not the full Public Suffix List.
const TWO_LABEL_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "com.au", "net.au", "org.au", "co.nz", "co.jp",
    "com.br", "com.cn", "co.in", "com.mx", "co.za", "com.tr", "com.sg", "com.hk",
];

/// Company and the registrable domains that belong to it. Matching is on the root
/// domain, so `gateway.icloud.com` counts toward `icloud.com`.
pub const COMPANIES: &[(&str, &[&str])] = &[
    (
        "Google",
        &[
            "google.com", "googleapis.com", "gstatic.com", "googlevideo.com", "youtube.com",
            "ytimg.com", "ggpht.com", "gvt1.com", "gvt2.com", "googleusercontent.com",
            "doubleclick.net", "googletagmanager.com", "google-analytics.com", "1e100.net",
            "android.com", "gmail.com", "googletagservices.com",
        ],
    ),
    (
        "Apple",
        &[
            "apple.com", "apple-dns.net", "aaplimg.com", "icloud.com", "mzstatic.com",
            "apple.news", "cdn-apple.com", "apple-cloudkit.com", "itunes.com",
        ],
    ),
    (
        "Microsoft",
        &[
            "microsoft.com", "windows.com", "windowsupdate.com", "msn.com", "live.com",
            "office.com", "office365.com", "msftconnecttest.com", "msedge.net", "bing.com",
            "azure.com", "skype.com", "xbox.com", "sharepoint.com", "outlook.com", "msftncsi.com",
        ],
    ),
    (
        "Amazon",
        &[
            "amazon.com", "amazonaws.com", "amazon-adsystem.com", "media-amazon.com",
            "primevideo.com", "ssl-images-amazon.com", "amazon.co.uk",
        ],
    ),
    (
        "Meta",
        &[
            "facebook.com", "fbcdn.net", "facebook.net", "instagram.com", "cdninstagram.com",
            "whatsapp.net", "whatsapp.com", "meta.com", "messenger.com",
        ],
    ),
];

/// The registrable domain of `name`: its last two labels, or three where the
/// suffix is one of the two-label public suffixes.
pub fn root_domain(name: &str) -> String {
    let lower = name.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = lower.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() <= 2 {
        return labels.join(".");
    }
    let last_two = format!("{}.{}", labels[labels.len() - 2], labels[labels.len() - 1]);
    if TWO_LABEL_SUFFIXES.contains(&last_two.as_str()) {
        return labels[labels.len() - 3..].join(".");
    }
    last_two
}

/// Query counts per root domain, largest first.
pub fn root_totals(domains: &[(String, i64)]) -> Vec<(String, i64)> {
    let mut totals: HashMap<String, i64> = HashMap::new();
    for (domain, count) in domains {
        *totals.entry(root_domain(domain)).or_default() += count;
    }
    sorted(totals)
}

/// The company a root domain belongs to, if it is in `COMPANIES`.
pub fn company_for_root(root: &str) -> Option<&'static str> {
    COMPANIES
        .iter()
        .find(|(_, roots)| roots.contains(&root))
        .map(|(name, _)| *name)
}

/// Query counts per company, for the companies in `COMPANIES` that were queried.
/// Companies with no queries are left out.
pub fn company_totals(domains: &[(String, i64)]) -> Vec<(String, i64)> {
    let mut totals: HashMap<String, i64> = HashMap::new();
    for (domain, count) in domains {
        if let Some(company) = company_for_root(&root_domain(domain)) {
            *totals.entry(company.to_string()).or_default() += count;
        }
    }
    sorted(totals)
}

fn sorted(totals: HashMap<String, i64>) -> Vec<(String, i64)> {
    let mut rows: Vec<(String, i64)> = totals.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

/// A MaxMind country database (GeoLite2-Country.mmdb or a compatible file).
pub struct GeoDb {
    reader: Reader<Vec<u8>>,
}

impl GeoDb {
    /// Opens the database at `path`. Returns `None` if the file is missing or unreadable,
    /// so the dashboard works without it and shows a hint instead.
    pub fn open(path: &Path) -> Option<Self> {
        if !path.exists() {
            return None;
        }
        Reader::open_readfile(path).ok().map(|reader| Self { reader })
    }

    /// ISO 3166-1 alpha-2 country code for `ip`, if the database knows it.
    pub fn country(&self, ip: &str) -> Option<String> {
        let addr: IpAddr = ip.parse().ok()?;
        let result = self.reader.lookup(addr).ok()?;
        let country: geoip2::Country = result.decode().ok()??;
        country.country.iso_code.map(str::to_string)
    }
}

/// Query counts per destination country, given answer-address counts.
/// Addresses the database does not place are left out.
pub fn country_totals(geo: &GeoDb, answer_ips: &[(String, i64)]) -> Vec<(String, i64)> {
    let mut totals: HashMap<String, i64> = HashMap::new();
    for (ip, count) in answer_ips {
        if let Some(code) = geo.country(ip) {
            *totals.entry(code).or_default() += count;
        }
    }
    sorted(totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_domain_keeps_registrable_part() {
        assert_eq!(root_domain("applplatform.apple.com"), "apple.com");
        assert_eq!(root_domain("g.live.com."), "live.com");
        assert_eq!(root_domain("tracker.example.co.uk"), "example.co.uk");
        assert_eq!(root_domain("example.co.uk"), "example.co.uk");
        assert_eq!(root_domain("localhost"), "localhost");
        assert_eq!(root_domain("Nas.TEST"), "nas.test");
    }

    #[test]
    fn company_totals_sum_their_domains() {
        let domains = vec![
            ("p.googlevideo.com".to_string(), 10),
            ("youtube.com".to_string(), 5),
            ("api.apple.com".to_string(), 7),
            ("random.example".to_string(), 99),
        ];
        assert_eq!(
            company_totals(&domains),
            vec![("Google".to_string(), 15), ("Apple".to_string(), 7)]
        );
    }

    #[test]
    fn root_totals_group_subdomains() {
        let domains = vec![
            ("a.example.com".to_string(), 2),
            ("b.example.com".to_string(), 3),
            ("example.org".to_string(), 1),
        ];
        assert_eq!(
            root_totals(&domains),
            vec![("example.com".to_string(), 5), ("example.org".to_string(), 1)]
        );
    }
}
