//! Per-app blocking. A "service" is a named bundle of domains, such as YouTube or
//! Discord, taken from AdGuard's blocked-services catalogue. A client blocks a service
//! by naming its id, and the engine treats the bundle like one more block list.
//!
//! The catalogue is downloaded at runtime and cached on disk, never bundled with the
//! program, because its repository is GPL-3.0. Rules that need more than a plain
//! domain match (wildcards, anchors, `$dnstype`) are skipped, as in any other list.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use serde::Deserialize;

use crate::config::ListFormat;
use crate::lists::{ListData, fetch_source, parse_list};

pub const CATALOGUE_URL: &str = "https://adguardteam.github.io/HostlistsRegistry/assets/services.json";

/// Prefix of the synthetic list name. It has no colon, because the by-list
/// statistics read the list name from `list:<name>:<domain>` up to the first colon.
const LIST_PREFIX: &str = "app/";

#[derive(Deserialize)]
struct RawCatalogue {
    blocked_services: Vec<RawService>,
}

#[derive(Deserialize)]
struct RawService {
    id: String,
    name: String,
    group: String,
    #[serde(default)]
    rules: Vec<String>,
}

pub struct Service {
    pub id: String,
    pub name: String,
    pub group: String,
    /// The service's domains, shaped like a block list named `app/<id>`.
    pub data: Arc<ListData>,
}

/// Every known service. Empty when the catalogue could not be fetched and none is cached.
#[derive(Default)]
pub struct Catalogue {
    services: BTreeMap<String, Service>,
    /// Set when the cached copy was used because the download failed.
    pub note: Option<String>,
}

impl Catalogue {
    pub fn get(&self, id: &str) -> Option<&Service> {
        self.services.get(id)
    }

    /// Services in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Service> {
        self.services.values()
    }

    pub fn len(&self) -> usize {
        self.services.len()
    }

    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }
}

/// Parses the catalogue JSON. Each service's rules go through the Adblock list parser.
pub fn parse_catalogue(json: &str) -> Result<Catalogue, String> {
    let raw: RawCatalogue = serde_json::from_str(json).map_err(|e| format!("invalid catalogue: {e}"))?;
    let mut services = BTreeMap::new();
    for item in raw.blocked_services {
        let parsed = parse_list(&item.rules.join("\n"), ListFormat::Abp);
        let data = ListData {
            name: format!("{LIST_PREFIX}{}", item.id),
            domains: Arc::new(parsed.domains),
            skipped: parsed.skipped,
            sources_total: 1,
            sources_ok: 1,
            source_errors: Vec::new(),
            loaded_at: Utc::now(),
        };
        services.insert(
            item.id.clone(),
            Service {
                id: item.id,
                name: item.name,
                group: item.group,
                data: Arc::new(data),
            },
        );
    }
    Ok(Catalogue { services, note: None })
}

/// Downloads the catalogue, falling back to the cached copy.
pub async fn load_catalogue(http: &reqwest::Client, cache_dir: &Path) -> Result<Catalogue, String> {
    let (text, note) = fetch_source(http, CATALOGUE_URL, cache_dir).await?;
    let mut catalogue = parse_catalogue(&text)?;
    catalogue.note = note;
    Ok(catalogue)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "blocked_services": [
            {"id": "youtube", "name": "YouTube", "group": "streaming",
             "rules": ["||youtube.com^", "||ytimg.com^", "||googlevideo.com^", "||*.yt3.ggpht.com^"]},
            {"id": "amazon", "name": "Amazon", "group": "shopping",
             "rules": ["||amazon.com^", "||amazonaws.com^$dnstype=~CNAME", "|anchored.example.net^"]}
        ],
        "groups": [{"id": "streaming"}]
    }"#;

    #[test]
    fn plain_domain_rules_become_the_services_domains() {
        let catalogue = parse_catalogue(SAMPLE).unwrap();
        assert_eq!(catalogue.len(), 2);
        let youtube = catalogue.get("youtube").unwrap();
        assert_eq!(youtube.name, "YouTube");
        assert_eq!(youtube.group, "streaming");
        assert!(youtube.data.domains.contains("youtube.com"));
        assert!(youtube.data.domains.contains("googlevideo.com"));
    }

    #[test]
    fn rules_that_are_not_plain_domains_are_skipped_not_widened() {
        // A wildcard, a `$dnstype` rule and a start anchor must not turn into a block of
        // the whole domain: that would block more than the catalogue intends.
        let catalogue = parse_catalogue(SAMPLE).unwrap();
        let amazon = catalogue.get("amazon").unwrap();
        assert!(amazon.data.domains.contains("amazon.com"));
        assert!(!amazon.data.domains.contains("amazonaws.com"));
        assert_eq!(amazon.data.domains.len(), 1);
        assert_eq!(amazon.data.skipped, 2);
        assert_eq!(catalogue.get("youtube").unwrap().data.skipped, 1);
    }

    #[test]
    fn list_name_has_no_colon_so_by_list_statistics_keep_each_app_separate() {
        let catalogue = parse_catalogue(SAMPLE).unwrap();
        let name = &catalogue.get("youtube").unwrap().data.name;
        assert_eq!(name, "app/youtube");
        assert!(!name.contains(':'));
    }

    #[test]
    fn malformed_json_is_an_error_not_an_empty_catalogue() {
        assert!(parse_catalogue("not json").is_err());
        assert!(parse_catalogue("{}").is_err());
    }
}
