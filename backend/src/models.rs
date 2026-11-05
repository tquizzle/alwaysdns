//! JSON shapes returned by the HTTP API. The native telemetry and the AdGuard
//! read-only source both produce these types, so the frontend never needs to
//! know which backend is active.

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct StatsResponse {
    pub total: u64,
    pub blocked: u64,
    pub cached: u64,
    pub forwarded: u64,
    /// Fraction of queries blocked, from 0.0 to 1.0.
    pub block_rate: f64,
    pub avg_ms: f64,
    pub active_clients: usize,
    pub timeline: Vec<TimelinePoint>,
    pub top_domains: Vec<TopEntry>,
    pub top_blocked: Vec<TopEntry>,
    pub upstreams: Vec<UpstreamStat>,
    /// Query counts per registrable domain, largest first.
    pub root_domains: Vec<TopEntry>,
    /// Query counts per company (Google, Apple, Microsoft, Amazon, Meta).
    pub companies: Vec<TopEntry>,
    /// Blocked queries per block source (list name, custom rules, client rules).
    pub block_reasons: Vec<TopEntry>,
    /// Query counts per destination country, named by ISO code.
    pub countries: Vec<TopEntry>,
    /// False when no GeoLite2 database is loaded, so the map has no data.
    pub geo_ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelinePoint {
    /// RFC 3339 start of the bucket, UTC.
    pub t: String,
    pub total: u64,
    pub blocked: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopEntry {
    pub name: String,
    pub count: u64,
    /// For blocked domains: the block source that matched most often.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpstreamStat {
    /// The resolver's friendly name, or the URL when it has none.
    pub name: String,
    /// The server URL, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// `doh` or `doq`, from the URL scheme.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<&'static str>,
    /// False after three failures in a row, live or probe. True when never attempted.
    pub ok: bool,
    /// Average of live client queries, in ms. Zero when none have been answered.
    pub avg_ms: f64,
    pub errors: u64,
    /// Median round trip of the recent probes, in ms. None before the first probe.
    pub probe_p50_ms: Option<f64>,
    /// 95th percentile of the recent probes, in ms.
    pub probe_p95_ms: Option<f64>,
    /// How many successful probes the percentiles cover, at most one an hour's worth.
    pub probe_samples: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryItem {
    pub time: String,
    pub client: String,
    pub domain: String,
    pub qtype: String,
    pub result: String,
    pub upstream: String,
    /// The resolver's friendly name, when a resolver has this upstream's URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_name: Option<String>,
    pub elapsed_ms: f64,
    pub rule: String,
    /// Source address. Included for admins only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryLogResponse {
    pub items: Vec<QueryItem>,
    /// Matches for the filters, across all pages.
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryResponse {
    pub timeline: Vec<TimelinePoint>,
}
