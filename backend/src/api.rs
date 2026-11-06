//! HTTP API under `/api`, plus static file serving from `DNSUI_STATIC_DIR`.
//!
//! Every endpoint except health and login requires a session cookie. Admins
//! see full detail. Viewers see redacted names (see `redact.rs`), no config,
//! and no source addresses. Viewers may read the list summary, but never its
//! URLs or raw errors.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::services::{ServeDir, ServeFile};

use crate::app::{AppState, NewList, StatsSource};
use crate::auth::{Role, SESSION_COOKIE, token_from_cookie_header};
use crate::config::{ClientConfig, Config, ListConfig, ListFormat, RewriteConfig, Retention, check_resolver_name, protocol_of};
use crate::db::{self, ListPatch, ListRecord, QueryFilter, QueryRecord};
use crate::insights;
use crate::models::{HistoryResponse, QueryItem, QueryLogResponse, StatsResponse, TimelinePoint, TopEntry};
use crate::redact::{redact_client, redact_query, redact_source, redact_stats};
use crate::rules::{is_valid_domain, normalize_rule_domain};
use crate::telemetry::Range;

/// Most rows one query-log page may return.
const MAX_PAGE: usize = 1000;
/// Most points one history response may hold.
const MAX_HISTORY_POINTS: i64 = 11_000;
/// Entries in each top list.
const TOP_N: i64 = 20;
const MS_PER_DAY: i64 = 86_400_000;

pub fn router(state: Arc<AppState>) -> Router {
    let static_dir = state.static_dir.clone();
    let index = static_dir.join("index.html");
    let static_service = ServeDir::new(&static_dir).fallback(ServeFile::new(index));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(login))
        .route("/api/auth/me", get(me))
        .route("/api/auth/logout", post(logout))
        .route("/api/stats", get(stats))
        .route("/api/querylog", get(querylog))
        .route("/api/history", get(history))
        .route("/api/clients", get(clients).post(create_client))
        .route("/api/services", get(services))
        .route("/api/lists", get(lists).post(create_list))
        .route("/api/lists/refresh", post(refresh_lists))
        .route("/api/lists/{id}", put(update_list).delete(delete_list))
        .route("/api/assignments", put(assign_list))
        .route("/api/rules/block", post(block_domain))
        .route("/api/clients/{name}", put(update_client).delete(delete_client))
        .route("/api/upstream-groups", post(create_group))
        .route("/api/upstream-groups/{name}", put(update_group).delete(delete_group))
        .route("/api/resolvers", post(create_resolver))
        .route("/api/resolvers/{name}", put(update_resolver).delete(delete_resolver))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/settings/export", get(export_settings))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/reload", post(reload))
        .route("/api/{*rest}", any(api_not_found))
        .fallback_service(static_service)
        .with_state(state)
}

/// Error returned as `{"error": "..."}` with an HTTP status.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "login required")
    }
    fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "admin role required")
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
    fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message)
    }
    fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// The authenticated caller. Using it as a handler argument enforces login.
pub struct Session {
    pub username: String,
    pub role: Role,
}

impl Session {
    fn require_admin(&self) -> Result<(), ApiError> {
        if self.role.is_admin() {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}

impl FromRequestParts<Arc<AppState>> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(token_from_cookie_header)
            .ok_or_else(ApiError::unauthorized)?;
        let (username, role) = state
            .sessions
            .lookup(token)
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Session { username, role })
    }
}

fn session_cookie(state: &AppState, value: &str, max_age: i64) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}"
    );
    if state.cookie_secure {
        cookie.push_str("; Secure");
    }
    cookie
}

async fn api_not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "no such endpoint")
}

/// Runs blocking work (SQLite, file reads) off the async executor.
async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::internal(format!("task failed: {e}")))?
        .map_err(|e| ApiError::internal(format!("{e:#}")))
}

fn rfc3339_ms(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let db_status = match state.db.clone() {
        Some(db) => match tokio::task::spawn_blocking(move || db.ping()).await {
            Ok(Ok(())) => "ok".to_string(),
            Ok(Err(e)) => format!("error: {e:#}"),
            Err(e) => format!("error: {e}"),
        },
        None => match &state.db_error {
            Some(e) => format!("error: {e}"),
            None => "disabled".to_string(),
        },
    };
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "db": db_status,
        "log_dropped": state.log_dropped(),
    }))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct Who {
    username: String,
    role: Role,
}

async fn login(
    State(state): State<Arc<AppState>>,
    body: Result<Json<LoginBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    // Argon2 is slow by design. Run it off the async executor.
    let verifier = state.clone();
    let (user, pass) = (body.username.clone(), body.password);
    let role = tokio::task::spawn_blocking(move || verifier.users.verify(&user, &pass))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "invalid username or password"))?;

    let token = state.sessions.create(&body.username, role);
    let cookie = session_cookie(&state, &token, 12 * 60 * 60);
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(Who {
            username: body.username,
            role,
        }),
    )
        .into_response())
}

async fn me(session: Session) -> Json<Who> {
    Json(Who {
        username: session.username,
        role: session.role,
    })
}

async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(token) = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(token_from_cookie_header)
    {
        state.sessions.remove(token);
    }
    let cookie = session_cookie(&state, "", 0);
    ([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response()
}

// ---------------------------------------------------------------------------
// Stats and history
// ---------------------------------------------------------------------------

/// Named ranges for `/api/stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatsRange {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl StatsRange {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "month" => Some(Self::Month),
            "year" => Some(Self::Year),
            // hour, day and week share the telemetry names.
            other => Range::parse(other).map(|r| match r {
                Range::Hour => Self::Hour,
                Range::Day => Self::Day,
                Range::Week => Self::Week,
            }),
        }
    }

    fn minutes(self) -> i64 {
        match self {
            Self::Hour => 60,
            Self::Day => 24 * 60,
            Self::Week => 7 * 24 * 60,
            Self::Month => 30 * 24 * 60,
            Self::Year => 365 * 24 * 60,
        }
    }

    /// Timeline bucket in seconds: minutes for hour and day, hours for week, days for month and year.
    fn bucket_secs(self) -> i64 {
        match self {
            Self::Hour | Self::Day => 60,
            Self::Week => 3600,
            Self::Month | Self::Year => 86_400,
        }
    }

    /// The in-memory window for this range, when the in-memory store serves it.
    fn in_memory(self) -> Option<Range> {
        match self {
            Self::Hour => Some(Range::Hour),
            Self::Day => Some(Range::Day),
            Self::Week => Some(Range::Week),
            Self::Month | Self::Year => None,
        }
    }
}

/// A window of time in unix seconds. `start` is aligned to `bucket`, and `end` is exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    start: i64,
    end: i64,
    bucket: i64,
}

impl Window {
    fn buckets(&self) -> i64 {
        (self.end - self.start + self.bucket - 1) / self.bucket
    }

    fn bucket_start(&self, index: i64) -> i64 {
        self.start + index * self.bucket
    }
}

/// The window for a named range, ending at the current minute.
fn named_window(range: StatsRange, now: DateTime<Utc>) -> Window {
    let end = (now.timestamp().div_euclid(60) + 1) * 60;
    window_for(end - range.minutes() * 60, end, range.bucket_secs())
}

/// Aligns a span of unix seconds to `bucket` and returns it as a window.
fn window_for(start_s: i64, end_s: i64, bucket: i64) -> Window {
    Window {
        start: start_s.div_euclid(bucket) * bucket,
        end: end_s,
        bucket,
    }
}

/// Bucket size for a custom span, chosen so that the point count stays readable.
fn bucket_for_span(span_s: i64) -> i64 {
    if span_s <= 86_400 {
        60
    } else if span_s <= 31 * 86_400 {
        3600
    } else {
        86_400
    }
}

/// Stats for a window, from the database. Totals and timeline come from
/// `minute_stats`. Top lists and active clients come from the retained queries.
fn db_stats(
    conn: &Connection,
    w: Window,
    geo: Option<&crate::insights::GeoDb>,
) -> anyhow::Result<StatsResponse> {
    let totals = db::minute_totals(conn, w.start, w.end)?;
    let mut resp = StatsResponse {
        total: totals.total as u64,
        blocked: totals.blocked as u64,
        cached: totals.cached as u64,
        forwarded: totals.forwarded as u64,
        ..Default::default()
    };
    if totals.total > 0 {
        resp.block_rate = totals.blocked as f64 / totals.total as f64;
    }
    if let Some(avg) = totals.avg_ms() {
        resp.avg_ms = round3(avg);
    }

    let series: HashMap<i64, (i64, i64)> = db::minute_series(conn, w.start, w.end, w.bucket)?
        .into_iter()
        .map(|(idx, total, blocked)| (idx, (total, blocked)))
        .collect();
    resp.timeline = (0..w.buckets())
        .map(|i| {
            let (total, blocked) = series.get(&i).copied().unwrap_or((0, 0));
            TimelinePoint {
                t: rfc3339_ms(w.bucket_start(i) * 1000),
                total: total as u64,
                blocked: blocked as u64,
            }
        })
        .collect();

    let (from_ms, to_ms) = (w.start * 1000, w.end * 1000);
    resp.top_domains = top_entries(db::top_domains(conn, from_ms, to_ms, false, TOP_N)?);
    resp.top_blocked = blocked_entries(conn, from_ms, to_ms)?;
    resp.active_clients = db::active_clients(conn, from_ms, to_ms)? as usize;

    let domains = db::all_domain_counts(conn, from_ms, to_ms)?;
    resp.root_domains = top_entries(
        insights::root_totals(&domains)
            .into_iter()
            .take(TOP_N as usize)
            .collect(),
    );
    resp.companies = top_entries(insights::company_totals(&domains));
    resp.block_reasons = top_entries(db::block_reason_counts(conn, from_ms, to_ms)?);
    if let Some(geo) = geo {
        resp.geo_ready = true;
        let ips = db::answer_ip_counts(conn, from_ms, to_ms)?;
        resp.countries = top_entries(insights::country_totals(geo, &ips));
    }
    Ok(resp)
}

/// Blocked domains with the block source that matched them most often.
fn blocked_entries(conn: &Connection, from_ms: i64, to_ms: i64) -> anyhow::Result<Vec<TopEntry>> {
    // domain -> (total blocked, best source, best source's count)
    let mut by_domain: HashMap<String, (i64, String, i64)> = HashMap::new();
    for (domain, source, count) in db::blocked_by_domain_and_source(conn, from_ms, to_ms)? {
        let entry = by_domain.entry(domain).or_insert_with(|| (0, String::new(), 0));
        entry.0 += count;
        if count > entry.2 {
            entry.1 = source;
            entry.2 = count;
        }
    }
    let mut rows: Vec<(String, i64, String)> = by_domain
        .into_iter()
        .map(|(domain, (total, reason, _))| (domain, total, reason))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(rows
        .into_iter()
        .take(TOP_N as usize)
        .map(|(name, count, reason)| TopEntry {
            name,
            count: count as u64,
            reason: Some(reason),
        })
        .collect())
}

fn top_entries(rows: Vec<(String, i64)>) -> Vec<TopEntry> {
    rows.into_iter()
        .map(|(name, count)| TopEntry {
            name,
            count: count as u64,
            reason: None,
        })
        .collect()
}

#[derive(Deserialize)]
struct StatsQuery {
    range: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
}

async fn stats(
    session: Session,
    State(state): State<Arc<AppState>>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let range = StatsRange::parse(q.range.as_deref().unwrap_or("day")).ok_or_else(|| {
        ApiError::bad_request("range must be hour, day, week, month or year")
    })?;
    let custom = q.from.is_some() || q.to.is_some();
    let now = Utc::now();

    // The explicit window, when from or to is given. A missing to means now,
    // and a missing from means one day before to.
    let custom_window = if custom {
        let to = q.to.unwrap_or_else(|| now.timestamp_millis());
        let from = q.from.unwrap_or(to - MS_PER_DAY);
        if from >= to {
            return Err(ApiError::bad_request("from must be before to"));
        }
        let span_s = to.div_euclid(1000) - from.div_euclid(1000) + 1;
        Some(window_for(
            from.div_euclid(1000),
            to.div_euclid(1000) + 1,
            bucket_for_span(span_s),
        ))
    } else {
        None
    };

    let mut stats = match &state.stats_source {
        StatsSource::AdGuard(src) => {
            let r = range
                .in_memory()
                .filter(|_| !custom)
                .ok_or_else(|| ApiError::bad_request("the AdGuard source supports hour, day and week"))?;
            src.stats(r)
                .await
                .map_err(|e| ApiError::bad_gateway(format!("{e:#}")))?
        }
        StatsSource::Native => match state.db.clone() {
            // Without a database, the last hour comes from memory and is lost on restart.
            // With one, every range reads SQLite so the dashboard survives a restart.
            _ if range == StatsRange::Hour && !custom && state.db.is_none() => {
                state.telemetry.stats(Range::Hour, now)
            }
            Some(db) => {
                let window = custom_window.unwrap_or_else(|| named_window(range, now));
                let geo = state.geo.clone();
                blocking(move || db.with(|c| db_stats(c, window, geo.as_deref()))).await?
            }
            None => {
                if custom || range.in_memory().is_none() {
                    return Err(ApiError::bad_request(
                        "this range needs the database, which is turned off",
                    ));
                }
                state
                    .telemetry
                    .stats(range.in_memory().unwrap_or(Range::Day), now)
            }
        },
    };

    if matches!(state.stats_source, StatsSource::Native) {
        stats.upstreams = state.upstream_stats();
    }
    if !session.role.is_admin() {
        redact_stats(&mut stats);
    }
    Ok(Json(stats))
}

#[derive(Deserialize)]
struct HistoryQuery {
    from: Option<i64>,
    to: Option<i64>,
    bucket: Option<String>,
}

async fn history(
    session: Session,
    State(state): State<Arc<AppState>>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, ApiError> {
    session.require_admin()?;
    let db = state
        .db
        .clone()
        .ok_or_else(|| ApiError::unavailable("history needs the database, which is turned off"))?;
    let bucket = match q.bucket.as_deref().unwrap_or("hour") {
        "minute" => 60,
        "hour" => 3600,
        "day" => 86_400,
        _ => return Err(ApiError::bad_request("bucket must be minute, hour or day")),
    };
    let to = q.to.unwrap_or_else(|| Utc::now().timestamp_millis());
    let from = q.from.unwrap_or(to - MS_PER_DAY);
    if from >= to {
        return Err(ApiError::bad_request("from must be before to"));
    }
    let w = window_for(from.div_euclid(1000), to.div_euclid(1000) + 1, bucket);
    if w.buckets() > MAX_HISTORY_POINTS {
        return Err(ApiError::bad_request(
            "that range has too many points for this bucket; choose a larger bucket or a shorter range",
        ));
    }

    let timeline = blocking(move || {
        db.with(|c| {
            let series: HashMap<i64, (i64, i64)> = db::minute_series(c, w.start, w.end, w.bucket)?
                .into_iter()
                .map(|(idx, total, blocked)| (idx, (total, blocked)))
                .collect();
            Ok((0..w.buckets())
                .map(|i| {
                    let (total, blocked) = series.get(&i).copied().unwrap_or((0, 0));
                    TimelinePoint {
                        t: rfc3339_ms(w.bucket_start(i) * 1000),
                        total: total as u64,
                        blocked: blocked as u64,
                    }
                })
                .collect::<Vec<_>>())
        })
    })
    .await?;
    Ok(Json(HistoryResponse { timeline }))
}

// ---------------------------------------------------------------------------
// Query log
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct QueryLogQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    domain: Option<String>,
    client: Option<String>,
    result: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
}

fn record_to_item(r: QueryRecord) -> QueryItem {
    QueryItem {
        time: rfc3339_ms(r.time_ms),
        client: r.client_name,
        domain: r.domain,
        qtype: r.qtype,
        result: r.result,
        upstream: r.upstream,
        upstream_name: None,
        elapsed_ms: round3(r.elapsed_ms),
        rule: r.rule,
        client_ip: Some(r.client_ip),
    }
}

async fn querylog(
    session: Session,
    State(state): State<Arc<AppState>>,
    Query(q): Query<QueryLogQuery>,
) -> Result<Json<QueryLogResponse>, ApiError> {
    let limit = q.limit.unwrap_or(100).clamp(1, MAX_PAGE);
    let offset = q.offset.unwrap_or(0);
    let admin = session.role.is_admin();
    let domain = q.domain.filter(|s| !s.is_empty());
    let client = q.client.filter(|s| !s.is_empty());
    let result = q.result.filter(|s| !s.is_empty());
    let (from, to) = (q.from, q.to);
    if let (Some(f), Some(t)) = (from, to) {
        if f > t {
            return Err(ApiError::bad_request("from must not be after to"));
        }
    }
    // A substring of the raw domain would reveal names that viewers may not see.
    if domain.is_some() && !admin {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "the domain filter needs the admin role",
        ));
    }

    let mut resp = match (&state.stats_source, state.db.clone()) {
        (StatsSource::Native, Some(db)) => {
            let (records, total) = blocking(move || {
                db.with(|c| {
                    // A viewer names clients by their redacted label, so map that label to the raw names.
                    let clients = match &client {
                        None => None,
                        Some(name) if admin => Some(vec![name.clone()]),
                        Some(name) => Some(
                            db::distinct_client_names(c)?
                                .into_iter()
                                .filter(|raw| raw == name || redact_client(raw) == *name)
                                .collect(),
                        ),
                    };
                    let filter = QueryFilter {
                        from_ms: from,
                        to_ms: to,
                        domain,
                        clients,
                        result,
                    };
                    db::query_page(c, &filter, limit as i64, offset as i64)
                })
            })
            .await?;
            let mut items: Vec<QueryItem> = records.into_iter().map(record_to_item).collect();
            if !admin {
                items.iter_mut().for_each(redact_query);
            }
            QueryLogResponse { items, total }
        }
        // The database is off, or AdGuard is the source. Filter what is in memory.
        _ => {
            let keep = move |item: &QueryItem| {
                if let Some(r) = &result {
                    if &item.result != r {
                        return false;
                    }
                }
                if let Some(c) = &client {
                    let matches = &item.client == c || (!admin && &redact_client(&item.client) == c);
                    if !matches {
                        return false;
                    }
                }
                if let Some(d) = &domain {
                    if !item.domain.to_ascii_lowercase().contains(&d.to_ascii_lowercase()) {
                        return false;
                    }
                }
                if from.is_some() || to.is_some() {
                    let ms = DateTime::parse_from_rfc3339(&item.time)
                        .map(|t| t.timestamp_millis())
                        .unwrap_or(0);
                    if from.is_some_and(|f| ms < f) || to.is_some_and(|t| ms > t) {
                        return false;
                    }
                }
                true
            };
            let matched = match &state.stats_source {
                StatsSource::Native => state.telemetry.query_log(usize::MAX, keep),
                StatsSource::AdGuard(src) => src
                    .querylog(offset.saturating_add(limit), keep)
                    .await
                    .map_err(|e| ApiError::bad_gateway(format!("{e:#}")))?,
            };
            let total = matched.len() as i64;
            let mut items: Vec<QueryItem> = matched.into_iter().skip(offset).take(limit).collect();
            if !admin {
                items.iter_mut().for_each(redact_query);
            }
            QueryLogResponse { items, total }
        }
    };
    let rt = state.runtime();
    for item in &mut resp.items {
        item.upstream_name = rt.config.resolver_name_for_url(&item.upstream).map(str::to_string);
    }
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// Clients
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ClientView {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cidrs: Option<Vec<String>>,
    /// Admin only: the client's own allow and block entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    allow_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_domains: Option<Vec<String>>,
    upstream_group: String,
    block_lists: Vec<String>,
    /// Apps blocked for this client, by catalogue id.
    blocked_services: Vec<String>,
    /// Queries in the last hour.
    queries: u64,
    /// Blocked queries in the last hour.
    blocked: u64,
}

async fn clients(
    session: Session,
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rt = state.runtime();
    let counts = state.telemetry.client_counts(Range::Hour, Utc::now());
    let admin = session.role.is_admin();
    let views: Vec<ClientView> = rt
        .config
        .clients
        .iter()
        .map(|c| {
            let (queries, blocked) = counts.get(&c.name).copied().unwrap_or((0, 0));
            ClientView {
                name: if admin { c.name.clone() } else { redact_client(&c.name) },
                cidrs: admin.then(|| c.cidrs.clone()),
                allow_domains: admin.then(|| c.allow_domains.clone()),
                block_domains: admin.then(|| c.block_domains.clone()),
                upstream_group: c
                    .upstream_group
                    .clone()
                    .unwrap_or_else(|| rt.config.default_upstream_group.clone()),
                block_lists: c.block_lists.clone(),
                blocked_services: c.blocked_services.clone(),
                queries,
                blocked,
            }
        })
        .collect();
    let groups = if admin { group_views(&rt.config) } else { Vec::new() };
    let resolvers = if admin { resolver_views(&rt.config) } else { Vec::new() };
    Json(json!({ "clients": views, "upstream_groups": groups, "resolvers": resolvers }))
}

/// One upstream group as the admin page shows it.
#[derive(Debug, Serialize)]
struct UpstreamGroupView {
    name: String,
    /// Resolver names, in failover order.
    resolvers: Vec<String>,
    /// Clients that name this group explicitly. Others fall back to the default.
    clients: Vec<String>,
    is_default: bool,
}

fn group_views(config: &Config) -> Vec<UpstreamGroupView> {
    config
        .upstream_groups
        .iter()
        .map(|(name, resolvers)| UpstreamGroupView {
            name: name.clone(),
            resolvers: resolvers.clone(),
            clients: config
                .clients
                .iter()
                .filter(|c| c.upstream_group.as_deref() == Some(name.as_str()))
                .map(|c| c.name.clone())
                .collect(),
            is_default: *name == config.default_upstream_group,
        })
        .collect()
}

/// One resolver as the admin page shows it.
#[derive(Debug, Serialize)]
struct ResolverView {
    name: String,
    url: String,
    /// `doh` or `doq`, from the URL scheme.
    protocol: Option<&'static str>,
    /// Groups that list this resolver.
    groups: Vec<String>,
}

fn resolver_views(config: &Config) -> Vec<ResolverView> {
    config
        .resolvers
        .iter()
        .map(|(name, url)| ResolverView {
            name: name.clone(),
            url: url.clone(),
            protocol: protocol_of(url),
            groups: config
                .upstream_groups
                .iter()
                .filter(|(_, names)| names.contains(name))
                .map(|(group, _)| group.clone())
                .collect(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

/// One list source as the API shows it.
#[derive(Debug, Serialize)]
struct ListView {
    id: i64,
    kind: String,
    name: String,
    url: String,
    enabled: bool,
    format: String,
    rules: i64,
    last_updated: Option<String>,
    error: Option<String>,
    /// Block lists only, admin only: the clients that block with this list.
    assigned_clients: Vec<String>,
    /// Block lists only, admin only: devices that match no client block with this list.
    assigned_default: bool,
}

#[derive(Debug, Serialize)]
struct ListsView {
    block: Vec<ListView>,
    allow: Vec<ListView>,
}

fn list_view(r: &ListRecord, admin: bool) -> ListView {
    ListView {
        id: r.id,
        kind: r.kind.clone(),
        name: r.name.clone(),
        url: if admin { r.url.clone() } else { redact_source(&r.url) },
        enabled: r.enabled,
        format: r.format.clone(),
        rules: r.rules,
        last_updated: r.last_updated.map(rfc3339_ms),
        error: r.error.as_ref().map(|e| if admin { e.clone() } else { "failed".into() }),
        assigned_clients: Vec::new(),
        assigned_default: false,
    }
}

/// Lists for the response. With the database on, one entry per row. With it off,
/// one entry per config source, with synthetic negative ids (read-only).
fn lists_view(state: &AppState, admin: bool) -> anyhow::Result<ListsView> {
    if let Some(rows) = state.list_rows()? {
        let (block, allow): (Vec<_>, Vec<_>) = rows.iter().partition(|r| r.kind == "block");
        let rt = state.runtime();
        return Ok(ListsView {
            block: block
                .into_iter()
                .map(|r| with_assignment(list_view(r, admin), &rt.config, admin))
                .collect(),
            allow: allow.into_iter().map(|r| list_view(r, admin)).collect(),
        });
    }

    let rt = state.runtime();
    let mut next_id = -1;
    let mut from_config = |kind: &str, name: &str, cfg: &crate::config::ListConfig, loaded: Option<&crate::lists::ListData>| {
        let mut out = Vec::new();
        for source in &cfg.sources {
            let record = ListRecord {
                id: next_id,
                kind: kind.to_string(),
                name: name.to_string(),
                url: source.clone(),
                enabled: true,
                format: db::format_name(cfg.format).to_string(),
                rules: loaded.map(|l| l.domains.len() as i64).unwrap_or(0),
                last_updated: loaded.map(|l| l.loaded_at.timestamp_millis()),
                error: loaded.and_then(|l| {
                    (!l.source_errors.is_empty()).then(|| l.source_errors.join("; "))
                }),
            };
            next_id -= 1;
            let view = list_view(&record, admin);
            out.push(if kind == "block" {
                with_assignment(view, &rt.config, admin)
            } else {
                view
            });
        }
        out
    };
    let mut block = Vec::new();
    for (name, cfg) in &rt.config.lists {
        block.extend(from_config("block", name, cfg, rt.lists.get(name).map(|a| a.as_ref())));
    }
    let mut allow = Vec::new();
    for (name, cfg) in &rt.config.allow_lists {
        let loaded = rt
            .global_allow_lists
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, a)| a.as_ref());
        allow.extend(from_config("allow", name, cfg, loaded));
    }
    Ok(ListsView { block, allow })
}

/// `GET /api/services`: the apps a client can block, from the catalogue. `note` is set
/// when the cached copy is in use because the download failed.
async fn services(_session: Session, State(state): State<Arc<AppState>>) -> Json<Value> {
    let rt = state.runtime();
    let items: Vec<Value> = rt
        .catalogue
        .iter()
        .map(|s| json!({ "id": s.id, "name": s.name, "group": s.group, "domains": s.data.domains.len() }))
        .collect();
    Json(json!({ "services": items, "note": rt.catalogue.note }))
}

async fn lists(session: Session, State(state): State<Arc<AppState>>) -> Result<Json<ListsView>, ApiError> {
    let admin = session.role.is_admin();
    let view = blocking(move || lists_view(&state, admin)).await?;
    Ok(Json(view))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateListBody {
    kind: String,
    name: String,
    url: String,
    enabled: Option<bool>,
    format: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateListBody {
    name: Option<String>,
    url: Option<String>,
    enabled: Option<bool>,
    format: Option<String>,
}

/// Names are used in client configs and rule strings, so keep them plain.
fn validate_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    if name.chars().count() > 64 {
        return Err(ApiError::bad_request("name must be 64 characters or fewer"));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::bad_request("name must not contain control characters"));
    }
    Ok(name.to_string())
}

/// A list source is an http(s) URL or an absolute file path.
fn validate_source(raw: &str) -> Result<String, ApiError> {
    let source = raw.trim();
    if source.is_empty() {
        return Err(ApiError::bad_request("url is required"));
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        let parsed = url::Url::parse(source)
            .map_err(|e| ApiError::bad_request(format!("url is not valid: {e}")))?;
        if parsed.host_str().is_none() {
            return Err(ApiError::bad_request("url has no host"));
        }
        return Ok(source.to_string());
    }
    if source.contains("://") || !std::path::Path::new(source).is_absolute() {
        return Err(ApiError::bad_request(
            "url must be an http(s) URL or an absolute file path",
        ));
    }
    Ok(source.to_string())
}

fn validate_format(raw: Option<String>) -> Result<String, ApiError> {
    match raw {
        None => Ok(db::format_name(ListFormat::Auto).to_string()),
        Some(f) => db::parse_format(f.trim())
            .map(|fmt| db::format_name(fmt).to_string())
            .ok_or_else(|| ApiError::bad_request("format must be auto, hosts, plain or abp")),
    }
}

/// Lists can change only when the database is on. Otherwise they come from the config file.
fn require_list_store(state: &AppState) -> Result<(), ApiError> {
    if state.db.is_none() {
        return Err(ApiError::unavailable(
            "lists are managed in the database, which is turned off (DNSUI_DB_ENABLED)",
        ));
    }
    Ok(())
}

async fn create_list(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<CreateListBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    require_list_store(&state)?;

    let kind = match body.kind.as_str() {
        "block" | "allow" => body.kind.clone(),
        _ => return Err(ApiError::bad_request("kind must be \"block\" or \"allow\"")),
    };
    let new = NewList {
        kind,
        name: validate_name(&body.name)?,
        url: validate_source(&body.url)?,
        enabled: body.enabled.unwrap_or(true),
        format: validate_format(body.format)?,
    };
    let record = state
        .list_create(new)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    Ok((StatusCode::CREATED, Json(list_view(&record, true))).into_response())
}

async fn update_list(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    body: Result<Json<UpdateListBody>, JsonRejection>,
) -> Result<Json<ListView>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    require_list_store(&state)?;
    if body.name.is_none() && body.url.is_none() && body.enabled.is_none() && body.format.is_none() {
        return Err(ApiError::bad_request(
            "nothing to update: send any of name, url, enabled, format",
        ));
    }

    let patch = ListPatch {
        name: body.name.as_deref().map(validate_name).transpose()?,
        url: body.url.as_deref().map(validate_source).transpose()?,
        enabled: body.enabled,
        format: body.format.map(|f| validate_format(Some(f))).transpose()?,
    };
    let record = state
        .list_update(id, patch)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?
        .ok_or_else(|| ApiError::not_found("no list with that id"))?;
    Ok(Json(list_view(&record, true)))
}

async fn delete_list(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    require_list_store(&state)?;
    let removed = state
        .list_delete(id)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    if !removed {
        return Err(ApiError::not_found("no list with that id"));
    }
    Ok(Json(json!({ "ok": true })))
}

async fn refresh_lists(session: Session, State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    state
        .refresh_lists()
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    let view = {
        let state = state.clone();
        blocking(move || lists_view(&state, true)).await?
    };
    Ok(Json(json!({ "ok": true, "block": view.block, "allow": view.allow })))
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize)]
struct YamlBody {
    yaml: String,
}

async fn get_config(session: Session, State(state): State<Arc<AppState>>) -> Result<Json<YamlBody>, ApiError> {
    session.require_admin()?;
    let yaml = state
        .config_yaml()
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    Ok(Json(YamlBody { yaml }))
}

async fn put_config(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<YamlBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let config: Config = serde_yaml::from_str(&body.yaml)
        .map_err(|e| ApiError::bad_request(format!("invalid YAML: {e}")))?;
    config
        .validate()
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;

    // Client and default block-list references must name a list that exists.
    // With the database on, the names come from the database, not the YAML lists section.
    let names: HashSet<String> = match state.db.clone() {
        Some(db) => blocking(move || {
            db.with(|c| {
                Ok(db::lists_all(c)?
                    .into_iter()
                    .filter(|r| r.kind == "block")
                    .map(|r| r.name)
                    .collect())
            })
        })
        .await?,
        None => config.lists.keys().cloned().collect(),
    };
    config
        .validate_list_refs(&names)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;

    let summary = state
        .save_config(config)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    Ok(Json(json!({ "ok": true, "lists": summary.lists })))
}

async fn reload(session: Session, State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> {
    session.require_admin()?;
    let summary = state
        .reload_from_disk()
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(json!({ "ok": true, "lists": summary.lists })))
}

/// Fills in which clients, and whether unknown devices, block with a block list.
fn with_assignment(mut view: ListView, config: &Config, admin: bool) -> ListView {
    if admin && view.kind == "block" {
        view.assigned_clients = config
            .clients
            .iter()
            .filter(|c| c.block_lists.iter().any(|l| l == &view.name))
            .map(|c| c.name.clone())
            .collect();
        view.assigned_default = config.default_block_lists.iter().any(|l| l == &view.name);
    }
    view
}

/// Adds or removes `name` from a list of names, without duplicates.
fn set_member(list: &mut Vec<String>, name: &str, member: bool) {
    list.retain(|l| l != name);
    if member {
        list.push(name.to_string());
    }
}

/// Adds `value` to a list unless it is already there.
fn push_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

/// Names of every block list that can be assigned.
fn block_list_names(state: &AppState) -> anyhow::Result<HashSet<String>> {
    match state.list_rows()? {
        Some(rows) => Ok(rows
            .into_iter()
            .filter(|r| r.kind == "block")
            .map(|r| r.name)
            .collect()),
        None => Ok(state.runtime().config.lists.keys().cloned().collect()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignBody {
    /// The block list name. Sources that share a name are assigned together.
    name: String,
    /// Client names that block with this list.
    clients: Vec<String>,
    /// Whether devices that match no client block with this list.
    default: bool,
}

/// `PUT /api/assignments`: sets which clients, and unknown devices, use a block list.
async fn assign_list(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<AssignBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let shared = Arc::clone(&state);
    let known = blocking(move || block_list_names(&shared)).await?;
    if !known.contains(&body.name) {
        return Err(ApiError::bad_request(format!("no block list named '{}'", body.name)));
    }

    let mut config = state.runtime().config.clone();
    for wanted in &body.clients {
        if !config.clients.iter().any(|c| &c.name == wanted) {
            return Err(ApiError::bad_request(format!("no client named '{wanted}'")));
        }
    }
    for client in &mut config.clients {
        let member = body.clients.contains(&client.name);
        set_member(&mut client.block_lists, &body.name, member);
    }
    set_member(&mut config.default_block_lists, &body.name, body.default);
    config
        .validate_list_refs(&known)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;

    state
        .save_rules(config)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(json!({
        "ok": true,
        "name": body.name,
        "clients": body.clients,
        "default": body.default,
    })))
}

/// Trims a group name and rejects names that are empty, too long or contain control characters.
fn validate_group_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("the group name cannot be empty"));
    }
    if name.chars().count() > 64 {
        return Err(ApiError::bad_request("the group name must be 64 characters or fewer"));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::bad_request("the group name cannot contain control characters"));
    }
    Ok(name)
}

/// Trims each entry, then drops blanks and repeats. Keeps the order given.
fn clean_list(raw: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for url in raw {
        let url = url.trim().to_string();
        if !url.is_empty() && !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// Validates a changed config and saves it. Upstream and list references are
/// checked before anything is written, so a bad edit never reaches the runtime.
async fn save_checked(state: &AppState, config: Config, known_lists: &HashSet<String>) -> Result<(), ApiError> {
    config
        .validate()
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    config
        .validate_list_refs(known_lists)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    state
        .save_rules(config)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupBody {
    /// Required when creating. Ignored when updating, where the name comes from the path.
    name: Option<String>,
    /// Resolver names, in failover order.
    resolvers: Vec<String>,
}

/// The full rule set for one client. Every save sends all of it, so a field left
/// out cannot clear a rule by accident. `name` is read only when creating.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientBody {
    name: Option<String>,
    cidrs: Vec<String>,
    upstream_group: String,
    block_lists: Vec<String>,
    allow_domains: Vec<String>,
    block_domains: Vec<String>,
    blocked_services: Vec<String>,
}

/// An app id from the catalogue: lowercase letters, digits and underscores.
/// Only the shape is checked here. The catalogue may be offline when a client is
/// saved, and an id it no longer lists is skipped when the rules are built.
fn is_service_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Checks the group and block-list references, then builds the client they describe.
fn client_from_body(
    body: ClientBody,
    name: String,
    known: &HashSet<String>,
    config: &Config,
) -> Result<ClientConfig, ApiError> {
    if !config.upstream_groups.contains_key(&body.upstream_group) {
        return Err(ApiError::bad_request(format!("no upstream group named '{}'", body.upstream_group)));
    }
    let mut block_lists: Vec<String> = Vec::new();
    for list in body.block_lists {
        if !known.contains(&list) {
            return Err(ApiError::bad_request(format!("no block list named '{list}'")));
        }
        push_unique(&mut block_lists, &list);
    }
    let mut blocked_services: Vec<String> = Vec::new();
    for id in body.blocked_services {
        if !is_service_id(&id) {
            return Err(ApiError::bad_request(format!("'{id}' is not a valid app id")));
        }
        push_unique(&mut blocked_services, &id);
    }
    Ok(ClientConfig {
        name,
        cidrs: clean_list(body.cidrs),
        upstream_group: Some(body.upstream_group),
        block_lists,
        allow_domains: clean_list(body.allow_domains),
        block_domains: clean_list(body.block_domains),
        blocked_services,
    })
}

/// Trims a client name and rejects names that are empty or too long.
fn validate_client_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("the client name cannot be empty"));
    }
    if name.chars().count() > 64 {
        return Err(ApiError::bad_request("the client name must be 64 characters or fewer"));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::bad_request("the client name cannot contain control characters"));
    }
    Ok(name)
}

/// `POST /api/clients`: adds a configured client.
async fn create_client(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<ClientBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let name = validate_client_name(body.name.as_deref().unwrap_or(""))?;

    let mut config = state.runtime().config.clone();
    if config.clients.iter().any(|c| c.name == name) {
        return Err(ApiError::bad_request(format!("a client named '{name}' already exists")));
    }
    let known = known_block_lists(&state).await?;
    let client = client_from_body(body, name.clone(), &known, &config)?;
    config.clients.push(client);

    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

/// `PUT /api/clients/{name}`: replaces a client's rules. The name in the path is kept.
async fn update_client(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Result<Json<ClientBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let known = known_block_lists(&state).await?;

    let mut config = state.runtime().config.clone();
    if !config.clients.iter().any(|c| c.name == name) {
        return Err(ApiError::not_found(format!("no client named '{name}'")));
    }
    let replacement = client_from_body(body, name.clone(), &known, &config)?;
    if let Some(slot) = config.clients.iter_mut().find(|c| c.name == name) {
        *slot = replacement;
    }

    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

/// `DELETE /api/clients/{name}`: removes a configured client.
async fn delete_client(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let mut config = state.runtime().config.clone();
    let before = config.clients.len();
    config.clients.retain(|c| c.name != name);
    if config.clients.len() == before {
        return Err(ApiError::not_found(format!("no client named '{name}'")));
    }
    let known = known_block_lists(&state).await?;
    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/upstream-groups`: adds a named group of upstream servers.
async fn create_group(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<GroupBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let name = validate_group_name(body.name.as_deref().unwrap_or(""))?;

    let mut config = state.runtime().config.clone();
    if config.upstream_groups.contains_key(&name) {
        return Err(ApiError::bad_request(format!("an upstream group named '{name}' already exists")));
    }
    config.upstream_groups.insert(name.clone(), clean_list(body.resolvers));

    let known = known_block_lists(&state).await?;
    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

/// `PUT /api/upstream-groups/{name}`: replaces the resolvers in a group.
async fn update_group(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Result<Json<GroupBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;

    let mut config = state.runtime().config.clone();
    let servers = config
        .upstream_groups
        .get_mut(&name)
        .ok_or_else(|| ApiError::not_found(format!("no upstream group named '{name}'")))?;
    *servers = clean_list(body.resolvers);

    let known = known_block_lists(&state).await?;
    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

/// `DELETE /api/upstream-groups/{name}`. Refuses the default group and any group a client still uses.
async fn delete_group(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;

    let mut config = state.runtime().config.clone();
    if !config.upstream_groups.contains_key(&name) {
        return Err(ApiError::not_found(format!("no upstream group named '{name}'")));
    }
    if name == config.default_upstream_group {
        return Err(ApiError::bad_request(
            "this is the default group. Choose another default in the config before deleting it",
        ));
    }
    let users: Vec<String> = config
        .clients
        .iter()
        .filter(|c| c.upstream_group.as_deref() == Some(name.as_str()))
        .map(|c| c.name.clone())
        .collect();
    if !users.is_empty() {
        return Err(ApiError::bad_request(format!(
            "'{name}' is still used by {}. Move those clients to another group first",
            users.join(", ")
        )));
    }
    config.upstream_groups.remove(&name);

    let known = known_block_lists(&state).await?;
    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolverBody {
    /// Required when creating. When updating, a different name renames the resolver.
    name: Option<String>,
    url: String,
}

/// Trims a resolver name and rejects names that are empty, too long or URL-shaped.
fn validate_resolver_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim().to_string();
    check_resolver_name(&name).map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    if name.chars().count() > 64 {
        return Err(ApiError::bad_request("the resolver name must be 64 characters or fewer"));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::bad_request("the resolver name cannot contain control characters"));
    }
    Ok(name)
}

/// The resolver that already uses `url`, ignoring `except`.
fn resolver_using_url<'a>(config: &'a Config, url: &str, except: Option<&str>) -> Option<&'a str> {
    config
        .resolvers
        .iter()
        .find(|(name, u)| u.as_str() == url && Some(name.as_str()) != except)
        .map(|(name, _)| name.as_str())
}

/// `POST /api/resolvers`: adds a named upstream server.
async fn create_resolver(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<ResolverBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let name = validate_resolver_name(body.name.as_deref().unwrap_or(""))?;
    let url = body.url.trim().to_string();

    let mut config = state.runtime().config.clone();
    if config.resolvers.contains_key(&name) {
        return Err(ApiError::bad_request(format!("a resolver named '{name}' already exists")));
    }
    if let Some(owner) = resolver_using_url(&config, &url, None) {
        return Err(ApiError::bad_request(format!("'{owner}' already uses this URL")));
    }
    config.resolvers.insert(name.clone(), url);

    let known = known_block_lists(&state).await?;
    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

/// `PUT /api/resolvers/{name}`: changes a resolver's URL and, when `name` differs
/// from the path, renames it. A rename carries into every group that lists it.
async fn update_resolver(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(old): Path<String>,
    body: Result<Json<ResolverBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;

    let mut config = state.runtime().config.clone();
    if !config.resolvers.contains_key(&old) {
        return Err(ApiError::not_found(format!("no resolver named '{old}'")));
    }
    let new = match body.name.as_deref() {
        Some(raw) => validate_resolver_name(raw)?,
        None => old.clone(),
    };
    let url = body.url.trim().to_string();
    if new != old && config.resolvers.contains_key(&new) {
        return Err(ApiError::bad_request(format!("a resolver named '{new}' already exists")));
    }
    if let Some(owner) = resolver_using_url(&config, &url, Some(&old)) {
        return Err(ApiError::bad_request(format!("'{owner}' already uses this URL")));
    }
    if new != old {
        config.rename_resolver(&old, &new);
    }
    config.resolvers.insert(new.clone(), url);

    let known = known_block_lists(&state).await?;
    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true, "name": new })))
}

/// `DELETE /api/resolvers/{name}`. Refuses a resolver that a group still lists.
async fn delete_resolver(
    session: Session,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;

    let mut config = state.runtime().config.clone();
    if config.resolvers.remove(&name).is_none() {
        return Err(ApiError::not_found(format!("no resolver named '{name}'")));
    }
    let groups: Vec<&str> = config
        .upstream_groups
        .iter()
        .filter(|(_, names)| names.contains(&name))
        .map(|(group, _)| group.as_str())
        .collect();
    if !groups.is_empty() {
        return Err(ApiError::bad_request(format!(
            "'{name}' is still used by {}. Remove it from those groups first",
            groups.join(", ")
        )));
    }

    let known = known_block_lists(&state).await?;
    save_checked(&state, config, &known).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Names of the block lists that exist now, from the database when it is on.
async fn known_block_lists(state: &Arc<AppState>) -> Result<HashSet<String>, ApiError> {
    let shared = Arc::clone(state);
    blocking(move || block_list_names(&shared)).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockBody {
    domain: String,
    /// A configured client name. Omit to block for every device.
    client: Option<String>,
}

/// `POST /api/rules/block`: blocks one domain for one client, or for every device.
async fn block_domain(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<BlockBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let domain = normalize_rule_domain(&body.domain);
    if !is_valid_domain(&domain) {
        return Err(ApiError::bad_request(format!("'{}' is not a valid domain", body.domain)));
    }

    let mut config = state.runtime().config.clone();
    let scope = match &body.client {
        None => {
            push_unique(&mut config.block_domains, &domain);
            "every device".to_string()
        }
        Some(name) => {
            let client = config
                .clients
                .iter_mut()
                .find(|c| &c.name == name)
                .ok_or_else(|| ApiError::bad_request(format!("no client named '{name}'")))?;
            push_unique(&mut client.block_domains, &domain);
            format!("client {name}")
        }
    };

    state
        .save_rules(config)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(json!({ "ok": true, "domain": domain, "scope": scope })))
}

/// What the Settings page edits. Every save sends all of it, so a field left out cannot
/// clear a setting by accident. Listeners are not here: changing a port needs a restart
/// and can lock you out of the dashboard, so they stay in `config.yaml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsBody {
    rewrites: Vec<RewriteConfig>,
    allow_domains: Vec<String>,
    block_domains: Vec<String>,
    bootstrap: Vec<String>,
    default_upstream_group: String,
    retention: Retention,
}

/// Trims, normalizes and de-duplicates a domain list, rejecting anything that is not a hostname.
fn clean_domains(raw: Vec<String>, what: &str) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = Vec::new();
    for item in clean_list(raw) {
        let domain = normalize_rule_domain(&item);
        if !is_valid_domain(&domain) {
            return Err(ApiError::bad_request(format!("'{item}' in {what} is not a valid domain")));
        }
        push_unique(&mut out, &domain);
    }
    Ok(out)
}

/// Applies a settings save to a copy of the config and validates the result.
fn settings_config(body: SettingsBody, mut config: Config) -> Result<Config, ApiError> {
    let mut rewrites: Vec<RewriteConfig> = Vec::new();
    for rewrite in body.rewrites {
        let domain = rewrite.domain.trim().to_lowercase();
        if rewrites.iter().any(|r| r.domain == domain) {
            return Err(ApiError::bad_request(format!("'{domain}' has more than one local answer")));
        }
        rewrites.push(RewriteConfig { domain, answer: rewrite.answer.trim().to_string() });
    }

    let mut bootstrap: Vec<IpAddr> = Vec::new();
    for raw in clean_list(body.bootstrap) {
        let ip: IpAddr = raw
            .parse()
            .map_err(|_| ApiError::bad_request(format!("'{raw}' is not an IP address")))?;
        if !bootstrap.contains(&ip) {
            bootstrap.push(ip);
        }
    }

    config.rewrites = rewrites;
    config.allow_domains = clean_domains(body.allow_domains, "the allow list")?;
    config.block_domains = clean_domains(body.block_domains, "the block list")?;
    config.bootstrap = bootstrap;
    config.default_upstream_group = body.default_upstream_group.trim().to_string();
    config.retention = body.retention;
    config
        .validate()
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(config)
}

fn settings_json(config: &Config) -> Value {
    json!({
        "rewrites": config.rewrites,
        "allow_domains": config.allow_domains,
        "block_domains": config.block_domains,
        "bootstrap": config.bootstrap,
        "default_upstream_group": config.default_upstream_group,
        "retention": config.retention,
        "listen": config.listen,
        "groups": config.upstream_groups.keys().collect::<Vec<_>>(),
    })
}

/// `GET /api/settings`: the settings the Settings page edits, plus the read-only listeners.
async fn get_settings(session: Session, State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    Ok(Json(settings_json(&state.runtime().config)))
}

/// `PUT /api/settings`: replaces local answers, global allow and block domains, bootstrap
/// resolvers, the default upstream group and retention.
async fn put_settings(
    session: Session,
    State(state): State<Arc<AppState>>,
    body: Result<Json<SettingsBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    session.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let config = settings_config(body, state.runtime().config.clone())?;
    let view = settings_json(&config);
    state
        .save_rules(config)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(view))
}

/// The settings as one portable YAML file. Lists live in the database once it is on, so
/// the file's own `lists` sections can be stale: the enabled database lists replace them.
/// Accounts, password hashes, sessions and query history are never part of a config.
fn export_yaml(config: &Config, rows: Option<Vec<ListRecord>>) -> anyhow::Result<String> {
    let mut config = config.clone();
    if let Some(rows) = rows {
        let mut block: std::collections::BTreeMap<String, ListConfig> = Default::default();
        let mut allow: std::collections::BTreeMap<String, ListConfig> = Default::default();
        for row in rows.into_iter().filter(|r| r.enabled) {
            let format = db::parse_format(&row.format).unwrap_or(ListFormat::Auto);
            let target = if row.kind == "allow" { &mut allow } else { &mut block };
            let entry = target
                .entry(row.name)
                .or_insert_with(|| ListConfig { sources: Vec::new(), format });
            if entry.format != format {
                entry.format = ListFormat::Auto;
            }
            entry.sources.push(row.url);
        }
        config.lists = block;
        config.allow_lists = allow;
    }
    let header = "# AlwaysDNS settings export.\n\
                  # Contains resolver URLs (some include a personal profile ID), client networks and\n\
                  # local answers. Check it before you share it. Accounts and query history are not included.\n";
    Ok(format!("{header}{}", config.to_yaml()?))
}

/// `GET /api/settings/export`: downloads the settings as `alwaysdns-settings.yaml`.
async fn export_settings(session: Session, State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    session.require_admin()?;
    let shared = Arc::clone(&state);
    let rows = blocking(move || shared.list_rows()).await?;
    let yaml = export_yaml(&state.runtime().config, rows).map_err(|e| ApiError::internal(format!("{e:#}")))?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"alwaysdns-settings.yaml\""),
        ],
        yaml,
    )
        .into_response())
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    fn base() -> Config {
        Config::parse_yaml(
            "bootstrap: [\"9.9.9.10\"]\nresolvers:\n  Quad9: \"https://dns10.quad9.net/dns-query\"\nupstream_groups:\n  default: [Quad9]\n  kids: [Quad9]\n",
        )
        .unwrap()
    }

    fn body() -> SettingsBody {
        SettingsBody {
            rewrites: vec![RewriteConfig { domain: " NAS.Example.Test ".into(), answer: " 192.0.2.5 ".into() }],
            allow_domains: vec!["Ok.Example.Org".into(), "ok.example.org".into(), "  ".into()],
            block_domains: vec!["*.Bad.Example".into()],
            bootstrap: vec!["9.9.9.10".into(), "9.9.9.10".into(), "2620:fe::10".into()],
            default_upstream_group: "kids".into(),
            retention: Retention { query_days: 14, stats_days: 90 },
        }
    }

    #[test]
    fn a_save_is_cleaned_and_applied() {
        let config = settings_config(body(), base()).unwrap();
        assert_eq!(config.rewrites[0].domain, "nas.example.test");
        assert_eq!(config.rewrites[0].answer, "192.0.2.5");
        assert_eq!(config.allow_domains, vec!["ok.example.org"]);
        assert_eq!(config.block_domains, vec!["bad.example"], "a wildcard prefix is stored as the suffix");
        assert_eq!(config.bootstrap.len(), 2, "the repeated address is dropped");
        assert_eq!(config.default_upstream_group, "kids");
        assert_eq!(config.retention.query_days, 14);
    }

    #[test]
    fn bad_input_is_rejected_with_a_reason() {
        let mut b = body();
        b.bootstrap = vec!["not-an-ip".into()];
        assert!(settings_config(b, base()).is_err());

        let mut b = body();
        b.rewrites[0].answer = "999.1.1.1".into();
        assert!(settings_config(b, base()).is_err());

        let mut b = body();
        b.rewrites.push(RewriteConfig { domain: "nas.example.test".into(), answer: "192.0.2.6".into() });
        assert!(settings_config(b, base()).is_err(), "one answer per name");

        let mut b = body();
        b.retention.query_days = 0;
        assert!(settings_config(b, base()).is_err(), "zero days would delete all history");

        let mut b = body();
        b.default_upstream_group = "missing".into();
        assert!(settings_config(b, base()).is_err());

        let mut b = body();
        b.block_domains = vec!["bad domain!".into()];
        assert!(settings_config(b, base()).is_err());
    }

    fn row(kind: &str, name: &str, url: &str, enabled: bool, format: &str) -> ListRecord {
        ListRecord {
            id: 1,
            kind: kind.into(),
            name: name.into(),
            url: url.into(),
            enabled,
            format: format.into(),
            rules: 0,
            last_updated: None,
            error: None,
        }
    }

    #[test]
    fn export_uses_the_database_lists_and_can_be_loaded_back() {
        let rows = vec![
            row("block", "ads", "https://lists.example.test/a.txt", true, "hosts"),
            row("block", "ads", "https://lists.example.test/b.txt", true, "abp"),
            row("block", "off", "https://lists.example.test/off.txt", false, "auto"),
            row("allow", "trusted", "/etc/lists/trusted.txt", true, "plain"),
        ];
        let yaml = export_yaml(&base(), Some(rows)).unwrap();
        assert!(yaml.starts_with("# AlwaysDNS settings export."));

        let loaded = Config::parse_yaml(&yaml).expect("an export is a valid config");
        assert_eq!(loaded.lists["ads"].sources.len(), 2, "rows with one name merge into one list");
        assert_eq!(loaded.lists["ads"].format, ListFormat::Auto, "mixed formats fall back to auto");
        assert!(!loaded.lists.contains_key("off"), "a disabled list is not exported");
        assert_eq!(loaded.allow_lists["trusted"].sources, vec!["/etc/lists/trusted.txt"]);
    }

    #[test]
    fn export_keeps_the_config_lists_when_there_is_no_database() {
        let mut config = base();
        config.lists.insert("ads".into(), ListConfig { sources: vec!["https://lists.example.test/a.txt".into()], format: ListFormat::Hosts });
        let yaml = export_yaml(&config, None).unwrap();
        assert!(Config::parse_yaml(&yaml).unwrap().lists.contains_key("ads"));
    }

    #[test]
    fn bootstrap_can_be_emptied_only_when_no_resolver_needs_it() {
        let mut b = body();
        b.bootstrap = vec![];
        assert!(settings_config(b, base()).is_err(), "Quad9 is given by hostname, so it needs bootstrap");
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;

    #[test]
    fn group_names_are_trimmed_and_checked() {
        assert_eq!(validate_group_name("  NextDNS  ").unwrap(), "NextDNS");
        assert!(validate_group_name("   ").is_err());
        assert!(validate_group_name(&"x".repeat(65)).is_err());
        assert!(validate_group_name("bad\tname").is_err());
    }

    #[test]
    fn upstream_urls_are_trimmed_and_deduplicated_in_order() {
        let cleaned = clean_list(vec![
            " quic://a.example:853 ".into(),
            "".into(),
            "https://b.example/dns-query".into(),
            "quic://a.example:853".into(),
        ]);
        assert_eq!(cleaned, vec!["quic://a.example:853", "https://b.example/dns-query"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_ranges_parse_and_bucket_as_specified() {
        assert_eq!(StatsRange::parse("month"), Some(StatsRange::Month));
        assert_eq!(StatsRange::parse("century"), None);
        // Minutes for hour and day, hours for week, days for month and year.
        assert_eq!(StatsRange::Day.bucket_secs(), 60);
        assert_eq!(StatsRange::Week.bucket_secs(), 3600);
        assert_eq!(StatsRange::Year.bucket_secs(), 86_400);
        assert_eq!(StatsRange::Week.minutes(), 10_080);
    }

    #[test]
    fn named_window_spans_the_range_and_ends_now() {
        let now = DateTime::<Utc>::from_timestamp(1_700_000_030, 0).unwrap();
        let w = named_window(StatsRange::Day, now);
        assert_eq!(w.end - w.start, 86_400);
        assert_eq!(w.end, 1_700_000_040, "end is the start of the next minute");
        assert_eq!(w.buckets(), 1440);
        // Week uses hour buckets: 168 hours, plus one when the start is not on an hour boundary.
        assert!(named_window(StatsRange::Week, now).buckets() >= 168);
    }

    #[test]
    fn custom_span_chooses_bucket_by_length() {
        assert_eq!(bucket_for_span(3600), 60);
        assert_eq!(bucket_for_span(7 * 86_400), 3600);
        assert_eq!(bucket_for_span(90 * 86_400), 86_400);
    }

    #[test]
    fn list_inputs_are_validated() {
        assert!(validate_source("https://example.com/list.txt").is_ok());
        assert!(validate_source("/etc/dnsui/lists/x.txt").is_ok());
        assert!(validate_source("relative/path.txt").is_err());
        assert!(validate_source("ftp://example.com/x").is_err());
        assert!(validate_source("https://").is_err());
        assert!(validate_source("   ").is_err());
        assert!(validate_name("  ads  ").unwrap() == "ads");
        assert!(validate_name("").is_err());
        assert!(validate_name("bad\nname").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
        assert_eq!(validate_format(None).unwrap(), "auto");
        assert!(validate_format(Some("zip".into())).is_err());
    }

    #[test]
    fn viewers_never_see_list_urls_or_raw_errors() {
        let record = ListRecord {
            id: 1,
            kind: "block".into(),
            name: "ads".into(),
            url: "https://secret.example/list.txt".into(),
            enabled: true,
            format: "auto".into(),
            rules: 3,
            last_updated: Some(1_700_000_000_000),
            error: Some("download failed: https://secret.example/list.txt 503".into()),
        };
        let viewer = serde_json::to_string(&list_view(&record, false)).unwrap();
        assert!(!viewer.contains("secret.example"));
        assert!(viewer.contains("\"error\":\"failed\""));
        let admin = serde_json::to_string(&list_view(&record, true)).unwrap();
        assert!(admin.contains("secret.example"));
    }

    #[test]
    fn history_window_has_contiguous_buckets() {
        let w = window_for(1_700_000_000, 1_700_003_600, 60);
        assert_eq!(w.buckets(), 61);
        assert_eq!(w.bucket_start(0), w.start);
        assert_eq!(w.bucket_start(1) - w.bucket_start(0), 60);
    }
}

#[cfg(test)]
mod assignment_tests {
    use super::*;

    #[test]
    fn set_member_adds_once_and_removes() {
        let mut names = vec!["ads".to_string()];
        set_member(&mut names, "spy", true);
        set_member(&mut names, "spy", true);
        assert_eq!(names, vec!["ads".to_string(), "spy".to_string()]);
        set_member(&mut names, "ads", false);
        assert_eq!(names, vec!["spy".to_string()]);
    }

    #[test]
    fn service_ids_are_the_catalogue_shape() {
        for ok in ["youtube", "4chan", "amazon_streaming"] {
            assert!(is_service_id(ok), "{ok} should be accepted");
        }
        for bad in ["", "YouTube", "you tube", "a-b", "x".repeat(65).as_str(), "../etc"] {
            assert!(!is_service_id(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn push_unique_skips_duplicates() {
        let mut domains = Vec::new();
        push_unique(&mut domains, "x.example");
        push_unique(&mut domains, "x.example");
        assert_eq!(domains, vec!["x.example".to_string()]);
    }

    #[test]
    fn assignment_reports_clients_and_default() {
        let yaml = r#"
listen:
  dns: "0.0.0.0:53"
  http: "0.0.0.0:8080"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
clients:
  - name: Laptop
    cidrs: ["192.0.2.10/32"]
    block_lists: ["ads"]
  - name: Kids
    cidrs: ["198.51.100.0/24"]
    block_lists: []
lists:
  ads:
    sources: ["https://example.com/a.txt"]
default_block_lists: ["ads"]
"#;
        let config = Config::parse_yaml(yaml).unwrap();
        let view = ListView {
            id: 1,
            kind: "block".into(),
            name: "ads".into(),
            url: String::new(),
            enabled: true,
            format: "auto".into(),
            rules: 0,
            last_updated: None,
            error: None,
            assigned_clients: Vec::new(),
            assigned_default: false,
        };
        let view = with_assignment(view, &config, true);
        assert_eq!(view.assigned_clients, vec!["Laptop".to_string()]);
        assert!(view.assigned_default);

        // Viewers never see assignments.
        let fresh = ListView {
            id: 1,
            kind: "block".into(),
            name: "ads".into(),
            url: String::new(),
            enabled: true,
            format: "auto".into(),
            rules: 0,
            last_updated: None,
            error: None,
            assigned_clients: Vec::new(),
            assigned_default: false,
        };
        let hidden = with_assignment(fresh, &config, false);
        assert!(hidden.assigned_clients.is_empty());
        assert!(!hidden.assigned_default);
    }
}
