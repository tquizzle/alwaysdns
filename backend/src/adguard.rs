//! Read-only stats and query-log source backed by an AdGuard Home instance
//! (`STATS_SOURCE=adguard`). It issues GET requests only and never writes to
//! AdGuard. DNS serving stays native. Only the `/api/stats` and
//! `/api/querylog` endpoints switch to this source.
//!
//! AdGuard keeps its own retention window, so ranges are approximate. Hour
//! and day figures are the last N points of its series, and week uses
//! whatever AdGuard has retained. The top lists are AdGuard's own totals.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;
use url::Url;

use crate::models::{QueryItem, StatsResponse, TimelinePoint, TopEntry, UpstreamStat};
use crate::telemetry::Range;

const TOP_N: usize = 20;

pub struct AdGuardSource {
    http: reqwest::Client,
    base: Url,
    user: String,
    password: String,
}

impl AdGuardSource {
    pub fn new(http: reqwest::Client, base_url: &str, user: &str, password: &str) -> Result<Self> {
        let base = Url::parse(base_url).with_context(|| format!("ADGUARD_URL '{base_url}' is not a URL"))?;
        Ok(Self {
            http,
            base,
            user: user.to_string(),
            password: password.to_string(),
        })
    }

    /// GET only. There is deliberately no write method on this type.
    async fn get_json(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        let mut url = self.base.join(path).context("building AdGuard URL")?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        let resp = self
            .http
            .get(url)
            .basic_auth(&self.user, Some(&self.password))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("AdGuard request failed")?;
        let resp = resp
            .error_for_status()
            .context("AdGuard returned an error status")?;
        resp.json::<Value>().await.context("AdGuard returned invalid JSON")
    }

    pub async fn stats(&self, range: Range) -> Result<StatsResponse> {
        let v = self.get_json("/control/stats", &[]).await?;
        let now = Utc::now();

        let units_minutes: i64 = if v["time_units"].as_str() == Some("days") {
            24 * 60
        } else {
            60
        };
        let points_wanted = ((range.minutes() + units_minutes - 1) / units_minutes).max(1) as usize;

        let total_series = u64_series(&v["dns_queries"]);
        let blocked_series = u64_series(&v["blocked_filtering"]);

        let (total, blocked, timeline) = if total_series.is_empty() {
            (
                v["num_dns_queries"].as_u64().unwrap_or(0),
                v["num_blocked_filtering"].as_u64().unwrap_or(0),
                Vec::new(),
            )
        } else {
            let take = points_wanted.min(total_series.len());
            let start = total_series.len() - take;
            let mut tot = 0;
            let mut blk = 0;
            let mut timeline = Vec::with_capacity(take);
            for i in 0..take {
                let idx = start + i;
                let t_total = total_series[idx];
                let t_blocked = blocked_series.get(idx).copied().unwrap_or(0);
                tot += t_total;
                blk += t_blocked;
                let steps_back = (take - i) as i64;
                timeline.push(TimelinePoint {
                    t: rfc3339(now.timestamp() - steps_back * units_minutes * 60),
                    total: t_total,
                    blocked: t_blocked,
                });
            }
            (tot, blk, timeline)
        };

        let mut resp = StatsResponse {
            total,
            blocked: blocked.min(total),
            cached: 0,
            forwarded: total.saturating_sub(blocked),
            block_rate: 0.0,
            avg_ms: (v["avg_processing_time"].as_f64().unwrap_or(0.0) * 1000.0 * 1000.0).round() / 1000.0,
            active_clients: v["top_clients"].as_array().map(Vec::len).unwrap_or(0),
            timeline,
            top_domains: top_entries(&v["top_queried_domains"]),
            top_blocked: top_entries(&v["top_blocked_domains"]),
            upstreams: upstream_stats(&v),
            // AdGuard does not expose the breakdowns below, so they stay empty.
            ..Default::default()
        };
        if resp.total > 0 {
            resp.block_rate = resp.blocked as f64 / resp.total as f64;
        }
        Ok(resp)
    }

    pub async fn querylog(
        &self,
        limit: usize,
        keep: impl Fn(&QueryItem) -> bool,
    ) -> Result<Vec<QueryItem>> {
        // AdGuard cannot filter by result, so fetch extra and filter here.
        let fetch = (limit.saturating_mul(4)).clamp(100, 5000);
        let v = self
            .get_json("/control/querylog", &[("limit", fetch.to_string())])
            .await?;
        let entries = v["data"]
            .as_array()
            .ok_or_else(|| anyhow!("AdGuard querylog response has no data array"))?;

        Ok(entries
            .iter()
            .map(map_querylog_entry)
            .filter(|item| keep(item))
            .take(limit)
            .collect())
    }
}

fn map_querylog_entry(e: &Value) -> QueryItem {
    let client_ip = e["client"].as_str().unwrap_or("").to_string();
    let client = e["client_info"]["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| client_ip.clone());
    let reason = e["reason"].as_str().unwrap_or("");
    let cached = e["cached"].as_bool().unwrap_or(false);
    let result = if cached {
        "cached"
    } else {
        match reason {
            r if r.starts_with("Filtered") => "blocked",
            r if r.starts_with("Rewrite") => "rewritten",
            "NotFilteredWhiteList" | "NotFilteredAllowList" => "allowed",
            _ => "forwarded",
        }
    };
    let elapsed = match &e["elapsed_ms"] {
        Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    QueryItem {
        time: e["time"].as_str().unwrap_or("").to_string(),
        client,
        domain: e["question"]["name"]
            .as_str()
            .unwrap_or("")
            .trim_end_matches('.')
            .to_string(),
        qtype: e["question"]["type"].as_str().unwrap_or("").to_string(),
        result: result.to_string(),
        upstream: e["upstream"].as_str().unwrap_or("").to_string(),
        elapsed_ms: elapsed,
        rule: e["rule"].as_str().unwrap_or("").to_string(),
        client_ip: Some(client_ip).filter(|s| !s.is_empty()),
    }
}

fn u64_series(v: &Value) -> Vec<u64> {
    v.as_array()
        .map(|arr| arr.iter().map(|x| x.as_u64().unwrap_or(0)).collect())
        .unwrap_or_default()
}

/// AdGuard encodes top lists as `[{"name": count}, ...]`.
fn top_entries(v: &Value) -> Vec<TopEntry> {
    let mut merged: HashMap<String, u64> = HashMap::new();
    for obj in v.as_array().into_iter().flatten() {
        if let Some(map) = obj.as_object() {
            for (name, count) in map {
                *merged.entry(name.clone()).or_insert(0) += count.as_u64().unwrap_or(0);
            }
        }
    }
    let mut items: Vec<TopEntry> = merged
        .into_iter()
        .map(|(name, count)| TopEntry {
            name,
            count,
            reason: None,
        })
        .collect();
    items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    items.truncate(TOP_N);
    items
}

fn upstream_stats(v: &Value) -> Vec<UpstreamStat> {
    let mut avg: HashMap<String, f64> = HashMap::new();
    for obj in v["top_upstreams_avg_time"].as_array().into_iter().flatten() {
        if let Some(map) = obj.as_object() {
            for (name, secs) in map {
                avg.insert(name.clone(), secs.as_f64().unwrap_or(0.0) * 1000.0);
            }
        }
    }
    let mut out: Vec<UpstreamStat> = Vec::new();
    for obj in v["top_upstreams_responses"].as_array().into_iter().flatten() {
        if let Some(map) = obj.as_object() {
            for name in map.keys() {
                out.push(UpstreamStat {
                    name: name.clone(),
                    // AdGuard reports no per-upstream error counts or health.
                    ok: true,
                    avg_ms: avg.get(name).copied().unwrap_or(0.0).round(),
                    errors: 0,
                    probe_p50_ms: None,
                    probe_p95_ms: None,
                    probe_samples: 0,
                });
            }
        }
    }
    out
}

fn rfc3339(unix_secs: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix_secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}
