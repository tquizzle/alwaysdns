//! Forced safe search: which names are redirected, and to which host.
//!
//! The provider hosts below come from each provider's own network-admin
//! documentation: Google (`forcesafesearch.google.com`), Bing (`strict.bing.com`),
//! DuckDuckGo (`safe.duckduckgo.com`) and YouTube Restricted Mode
//! (`restrict.youtube.com`, the strict mode). Google needs one entry per country
//! domain; the list is Google's `supported_domains`.

/// Where a covered name is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeSearch {
    /// Short provider name, used in the query log rule (`safesearch:<provider>`).
    pub provider: &'static str,
    /// Host the name is aliased to.
    pub target: &'static str,
}

const GOOGLE: SafeSearch = SafeSearch { provider: "google", target: "forcesafesearch.google.com" };
const BING: SafeSearch = SafeSearch { provider: "bing", target: "strict.bing.com" };
const DUCKDUCKGO: SafeSearch = SafeSearch { provider: "duckduckgo", target: "safe.duckduckgo.com" };
const YOUTUBE: SafeSearch = SafeSearch { provider: "youtube", target: "restrict.youtube.com" };

/// Exact names for the non-Google providers.
const EXACT: &[(&str, SafeSearch)] = &[
    ("www.bing.com", BING),
    ("edgeservices.bing.com", BING),
    ("duckduckgo.com", DUCKDUCKGO),
    ("www.duckduckgo.com", DUCKDUCKGO),
    ("www.youtube.com", YOUTUBE),
    ("m.youtube.com", YOUTUBE),
    ("youtubei.googleapis.com", YOUTUBE),
    ("youtube.googleapis.com", YOUTUBE),
    ("www.youtube-nocookie.com", YOUTUBE),
];

/// What follows `google.` in each of Google's country domains.
const GOOGLE_SUFFIXES: &[&str] = &[
    "com", "ad", "ae", "com.af", "com.ag", "al", "am", "co.ao", "com.ar", "as", "at", "com.au", "az",
    "ba", "com.bd", "be", "bf", "bg", "com.bh", "bi", "bj", "com.bn", "com.bo", "com.br", "bs", "bt",
    "co.bw", "by", "com.bz", "ca", "cd", "cf", "cg", "ch", "ci", "co.ck", "cl", "cm", "cn", "com.co",
    "co.cr", "com.cu", "cv", "com.cy", "cz", "de", "dj", "dk", "dm", "com.do", "dz", "com.ec", "ee",
    "com.eg", "es", "com.et", "fi", "com.fj", "fm", "fr", "ga", "ge", "gg", "com.gh", "com.gi", "gl",
    "gm", "gr", "com.gt", "gy", "com.hk", "hn", "hr", "ht", "hu", "co.id", "ie", "co.il", "im", "co.in",
    "iq", "is", "it", "je", "com.jm", "jo", "co.jp", "co.ke", "com.kh", "ki", "kg", "co.kr", "com.kw",
    "kz", "la", "com.lb", "li", "lk", "co.ls", "lt", "lu", "lv", "com.ly", "co.ma", "md", "me", "mg",
    "mk", "ml", "com.mm", "mn", "com.mt", "mu", "mv", "mw", "com.mx", "com.my", "co.mz", "com.na",
    "com.ng", "com.ni", "ne", "nl", "no", "com.np", "nr", "nu", "co.nz", "com.om", "com.pa", "com.pe",
    "com.pg", "com.ph", "com.pk", "pl", "pn", "com.pr", "ps", "pt", "com.py", "com.qa", "ro", "ru",
    "rw", "com.sa", "com.sb", "sc", "se", "com.sg", "sh", "si", "sk", "com.sl", "sn", "so", "sm", "sr",
    "st", "com.sv", "td", "tg", "co.th", "com.tj", "tl", "tm", "tn", "to", "com.tr", "tt", "com.tw",
    "co.tz", "com.ua", "co.ug", "co.uk", "com.uy", "co.uz", "com.vc", "co.ve", "co.vi", "com.vn", "vu",
    "ws", "rs", "co.za", "co.zm", "co.zw", "cat",
];

/// The safe-search redirect for `name`, if one applies. `name` must be
/// normalized (lowercase, no trailing dot). Only exact names match: `google.com`
/// and `www.google.com` are covered, `mail.google.com` and `notgoogle.com` are not.
pub fn lookup(name: &str) -> Option<SafeSearch> {
    if let Some((_, hit)) = EXACT.iter().find(|(n, _)| *n == name) {
        return Some(*hit);
    }
    let rest = name.strip_prefix("www.").unwrap_or(name);
    let suffix = rest.strip_prefix("google.")?;
    GOOGLE_SUFFIXES.contains(&suffix).then_some(GOOGLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str) -> Option<&'static str> {
        lookup(name).map(|s| s.target)
    }

    #[test]
    fn google_apex_www_and_country_domains_are_covered() {
        for name in ["google.com", "www.google.com", "www.google.co.uk", "google.com.au", "www.google.cat"] {
            assert_eq!(target(name), Some("forcesafesearch.google.com"), "{name}");
        }
    }

    #[test]
    fn other_google_names_and_lookalikes_are_not_covered() {
        // Only the search front doors are redirected. Mail, Drive and the like must keep working.
        for name in [
            "mail.google.com",
            "drive.google.com",
            "notgoogle.com",
            "www.notgoogle.com",
            "google.example",
            "www.google.com.evil.example",
            "google.",
            "www.google.zz",
            "google.com.example",
        ] {
            assert_eq!(lookup(name), None, "{name}");
        }
    }

    #[test]
    fn bing_duckduckgo_and_youtube_use_their_documented_hosts() {
        assert_eq!(target("www.bing.com"), Some("strict.bing.com"));
        assert_eq!(target("edgeservices.bing.com"), Some("strict.bing.com"));
        assert_eq!(target("duckduckgo.com"), Some("safe.duckduckgo.com"));
        for name in [
            "www.youtube.com",
            "m.youtube.com",
            "youtubei.googleapis.com",
            "youtube.googleapis.com",
            "www.youtube-nocookie.com",
        ] {
            assert_eq!(target(name), Some("restrict.youtube.com"), "{name}");
        }
    }

    #[test]
    fn youtube_names_google_says_to_leave_alone_are_not_redirected() {
        // Google's guidance: do not alias these, or sign-in and short links break.
        for name in ["youtube.com", "youtu.be", "s.ytimg.com", "googleapis.com", "youtubeeducation.com"] {
            assert_eq!(lookup(name), None, "{name}");
        }
    }

    #[test]
    fn provider_names_label_the_query_log_rule() {
        assert_eq!(lookup("www.google.com").map(|s| s.provider), Some("google"));
        assert_eq!(lookup("www.bing.com").map(|s| s.provider), Some("bing"));
        assert_eq!(lookup("www.youtube.com").map(|s| s.provider), Some("youtube"));
    }

    #[test]
    fn google_list_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for s in GOOGLE_SUFFIXES {
            assert!(seen.insert(*s), "duplicate suffix {s}");
        }
    }
}
