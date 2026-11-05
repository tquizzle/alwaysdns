//! Shared application state and the reload pipeline.
//!
//! The runtime snapshot sits behind an `RwLock<Arc<_>>`. A reload builds a
//! complete new snapshot, validates it, and swaps it in. A failed reload leaves
//! the current snapshot untouched. Reloads and list edits are serialized by
//! `reload_lock`, which also holds the per-source list cache.
//!
//! With the database on, lists come from its `lists` table. Each row is one
//! source, and rows that share a kind and name form one runtime list. With the
//! database off, lists come from `config.yaml` as before.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

use anyhow::{Result, anyhow};
use chrono::Utc;
use serde::Serialize;
use tracing::{error, info};

use crate::adguard::AdGuardSource;
use crate::auth::{Sessions, Users};
use crate::cache::DnsCache;
use crate::config::{Config, ListConfig, ListFormat};
use crate::db::{self, Db, ListPatch, ListRecord};
use crate::engine::Runtime;
use crate::lists::{ListData, load_list, load_lists_map};
use crate::models::UpstreamStat;
use crate::upstream::{PROBE_INTERVAL, PROBE_NAMES};
use crate::recorder::{CHANNEL_CAPACITY, LogSink};
use crate::telemetry::Telemetry;
use crate::upstream::UpstreamManager;

pub enum StatsSource {
    Native,
    AdGuard(AdGuardSource),
}

pub struct Options {
    pub config_path: PathBuf,
    pub cache_dir: PathBuf,
    pub static_dir: PathBuf,
    pub cookie_secure: bool,
    /// Path of `dnsui.db`. `None` turns the database off.
    pub db_path: Option<PathBuf>,
    /// Path of a MaxMind country database. Missing file means no destination map.
    pub geoip_db: Option<PathBuf>,
}

/// Which database-backed lists to download during a load.
#[derive(Debug, Clone)]
pub enum Refresh {
    /// Download every enabled list.
    All,
    /// Download these rows (by id). Other rows reuse their cached copy.
    Rows(Vec<i64>),
}

/// Loaded data for each database row, by row id. Guarded by `reload_lock`.
type SourceCache = HashMap<i64, Arc<ListData>>;

/// A new list row, already validated by the API.
#[derive(Debug, Clone)]
pub struct NewList {
    pub kind: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub format: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListSummary {
    pub name: String,
    pub domains: usize,
    pub skipped: usize,
    pub sources_total: usize,
    pub sources_ok: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReloadSummary {
    pub lists: Vec<ListSummary>,
}

pub struct AppState {
    pub config_path: PathBuf,
    pub cache_dir: PathBuf,
    pub static_dir: PathBuf,
    pub cookie_secure: bool,
    pub http: reqwest::Client,
    runtime: RwLock<Arc<Runtime>>,
    pub cache: DnsCache,
    pub upstreams: UpstreamManager,
    pub telemetry: Telemetry,
    pub users: Users,
    pub sessions: Sessions,
    pub stats_source: StatsSource,
    /// The database, when it is on and opened. `None` falls back to the
    /// in-memory query log and to lists from `config.yaml`.
    pub db: Option<Arc<Db>>,
    /// Why the database is off, when it was requested but could not open.
    pub db_error: Option<String>,
    /// Query writer for the database. Present exactly when `db` is.
    pub log_sink: Option<LogSink>,
    /// Country database for the destination map. `None` when the file is absent.
    pub geo: Option<Arc<crate::insights::GeoDb>>,
    reload_lock: tokio::sync::Mutex<SourceCache>,
}

impl AppState {
    /// Opens the database (if enabled), loads the config, downloads or reads
    /// every list, and builds the first runtime snapshot. Fails if the config is invalid.
    /// A database that cannot be opened is not fatal: DNS keeps running on the
    /// config lists, and `/api/health` reports the error.
    pub async fn new(
        http: reqwest::Client,
        opts: Options,
        users: Users,
        stats_source: StatsSource,
    ) -> Result<Arc<Self>> {
        let config = Config::load_and_migrate(&opts.config_path)?;
        let (db, db_error, log_sink) = open_database(opts.db_path.as_deref(), &config);

        let upstreams = UpstreamManager::new();
        let mut cache = SourceCache::new();
        let (runtime, _summary) = assemble(
            &http,
            &opts.cache_dir,
            db.as_deref(),
            &mut cache,
            config,
            &Refresh::All,
        )
        .await?;
        upstreams.set_bootstrap(runtime.config.bootstrap.clone());

        let geo = opts
            .geoip_db
            .as_deref()
            .and_then(crate::insights::GeoDb::open)
            .map(Arc::new);
        match &geo {
            Some(_) => info!("destination map: country database loaded"),
            None => info!("destination map: no country database found, map is off"),
        }

        Ok(Arc::new(Self {
            config_path: opts.config_path,
            cache_dir: opts.cache_dir,
            static_dir: opts.static_dir,
            cookie_secure: opts.cookie_secure,
            http,
            runtime: RwLock::new(Arc::new(runtime)),
            cache: DnsCache::new(),
            upstreams,
            telemetry: Telemetry::new(),
            users,
            sessions: Sessions::new(),
            stats_source,
            db,
            db_error,
            log_sink,
            geo,
            reload_lock: tokio::sync::Mutex::new(cache),
        }))
    }

    pub fn runtime(&self) -> Arc<Runtime> {
        match self.runtime.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// `POST /api/reload`: re-reads the config file and reloads every list.
    pub async fn reload_from_disk(&self) -> Result<ReloadSummary> {
        let mut cache = self.reload_lock.lock().await;
        let config = Config::load(&self.config_path)?;
        self.apply(config, &Refresh::All, &mut cache).await
    }

    /// The config the process is running with, as YAML. The file on disk can differ
    /// from this (for example after an AdGuard import), so the runtime config is the source.
    pub fn config_yaml(&self) -> Result<String> {
        self.runtime().config.to_yaml()
    }

    /// `PUT /api/config`: validates and writes the config, then applies it.
    pub async fn save_config(&self, config: Config) -> Result<ReloadSummary> {
        let mut cache = self.reload_lock.lock().await;
        config.save(&self.config_path)?;
        self.apply(config, &Refresh::All, &mut cache).await
    }

    /// Saves a change to list assignments or block rules. Lists keep their cached
    /// copies, so nothing is downloaded again.
    pub async fn save_rules(&self, config: Config) -> Result<()> {
        let mut cache = self.reload_lock.lock().await;
        config.save(&self.config_path)?;
        self.apply(config, &Refresh::Rows(Vec::new()), &mut cache).await?;
        Ok(())
    }

    /// Periodic refresh and `POST /api/lists/refresh`: re-downloads every enabled list.
    pub async fn refresh_lists(&self) -> Result<ReloadSummary> {
        let mut cache = self.reload_lock.lock().await;
        let config = self.runtime().config.clone();
        self.apply(config, &Refresh::All, &mut cache).await
    }

    /// Adds a list source, loads it, and reloads the rules.
    pub async fn list_create(&self, new: NewList) -> Result<ListRecord> {
        let db = self.require_db()?.clone();
        let mut cache = self.reload_lock.lock().await;
        let id = db.with(|c| db::lists_insert(c, &new.kind, &new.name, &new.url, new.enabled, &new.format))?;
        self.apply_rows(&mut cache, vec![id]).await?;
        db.with(|c| db::lists_get(c, id))?
            .ok_or_else(|| anyhow!("list {id} vanished after insert"))
    }

    /// Edits a list source and reloads it. `Ok(None)` when no list has that id.
    pub async fn list_update(&self, id: i64, patch: ListPatch) -> Result<Option<ListRecord>> {
        let db = self.require_db()?.clone();
        let mut cache = self.reload_lock.lock().await;
        let found = db.with(|c| db::lists_update(c, id, &patch))?;
        if !found {
            return Ok(None);
        }
        self.apply_rows(&mut cache, vec![id]).await?;
        db.with(|c| db::lists_get(c, id))
    }

    /// Deletes a list source and reloads the rules. `Ok(false)` when no list has that id.
    pub async fn list_delete(&self, id: i64) -> Result<bool> {
        let db = self.require_db()?.clone();
        let mut cache = self.reload_lock.lock().await;
        let removed = db.with(|c| db::lists_delete(c, id))?;
        if removed {
            self.apply_rows(&mut cache, Vec::new()).await?;
        }
        Ok(removed)
    }

    /// Every list row, or `None` when the database is off.
    pub fn list_rows(&self) -> Result<Option<Vec<ListRecord>>> {
        match &self.db {
            Some(db) => Ok(Some(db.with(db::lists_all)?)),
            None => Ok(None),
        }
    }

    pub fn require_db(&self) -> Result<&Arc<Db>> {
        self.db
            .as_ref()
            .ok_or_else(|| anyhow!("the database is disabled"))
    }

    /// Ages out old history. Returns `(queries, minutes)` deleted, or `None` when the database is off.
    pub async fn run_retention(&self) -> Result<Option<(usize, usize)>> {
        let Some(db) = self.db.clone() else {
            return Ok(None);
        };
        let retention = self.runtime().config.retention;
        let now_ms = Utc::now().timestamp_millis();
        let counts = tokio::task::spawn_blocking(move || {
            db.with(|c| db::apply_retention(c, now_ms, retention.query_days, retention.stats_days))
        })
        .await
        .map_err(|e| anyhow!("retention task failed: {e}"))??;
        Ok(Some(counts))
    }

    /// Writes queued query-log rows before shutdown. Returns false if the writer did not confirm.
    pub fn flush_query_log(&self) -> bool {
        match &self.log_sink {
            Some(sink) => sink.flush(std::time::Duration::from_secs(5)),
            None => true,
        }
    }

    /// Rows the query writer has dropped since start.
    pub fn log_dropped(&self) -> u64 {
        self.log_sink
            .as_ref()
            .map(|s| s.counters().dropped.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    async fn apply(
        &self,
        config: Config,
        refresh: &Refresh,
        cache: &mut SourceCache,
    ) -> Result<ReloadSummary> {
        let (runtime, summary) = assemble(
            &self.http,
            &self.cache_dir,
            self.db.as_deref(),
            cache,
            config,
            refresh,
        )
        .await?;
        self.install(runtime);
        Ok(summary)
    }

    /// Reloads the rows named in `ids` and keeps everything else from cache.
    async fn apply_rows(&self, cache: &mut SourceCache, ids: Vec<i64>) -> Result<ReloadSummary> {
        let config = self.runtime().config.clone();
        self.apply(config, &Refresh::Rows(ids), cache).await
    }

    fn install(&self, runtime: Runtime) {
        self.upstreams.set_bootstrap(runtime.config.bootstrap.clone());
        {
            let mut guard = match self.runtime.write() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            *guard = Arc::new(runtime);
        }
        // Cached answers may no longer match the new rules or upstreams.
        self.cache.clear();
    }

    /// Upstream health for every server a group uses, named by resolver.
    pub fn upstream_stats(&self) -> Vec<UpstreamStat> {
        let rt = self.runtime();
        let mut stats = self.upstreams.snapshot(&rt.config.upstream_urls());
        for stat in &mut stats {
            if let Some(name) = stat.url.as_deref().and_then(|u| rt.config.resolver_name_for_url(u)) {
                stat.name = name.to_string();
            }
        }
        stats
    }

    /// Probes every configured upstream once a minute on a background task. The
    /// first round runs at startup. Probes use the upstream resolvers directly, so
    /// they never reach the query log, the DNS cache or the block counts.
    pub fn spawn_probes(self: &Arc<Self>) {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PROBE_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut round: usize = 0;
            loop {
                tick.tick().await;
                let name = PROBE_NAMES[round % PROBE_NAMES.len()];
                round = round.wrapping_add(1);
                let urls = state.runtime().config.upstream_urls();
                let mut probes = tokio::task::JoinSet::new();
                for url in urls {
                    let state = Arc::clone(&state);
                    probes.spawn(async move { state.upstreams.probe(&url, name).await });
                }
                while probes.join_next().await.is_some() {}
            }
        });
    }
}

/// Opens the database, imports config lists on first use, and starts the
/// query writer. Failures are logged and turn the database off.
fn open_database(
    path: Option<&Path>,
    config: &Config,
) -> (Option<Arc<Db>>, Option<String>, Option<LogSink>) {
    let Some(path) = path else {
        return (None, None, None);
    };
    match open_and_start(path, config) {
        Ok((db, sink)) => {
            info!(path = %path.display(), schema = db::SCHEMA_VERSION, "database ready");
            (Some(db), None, Some(sink))
        }
        Err(e) => {
            let message = format!("{e:#}");
            error!(error = %message, "database unavailable; using the in-memory query log and config lists");
            (None, Some(message), None)
        }
    }
}

fn open_and_start(path: &Path, config: &Config) -> Result<(Arc<Db>, LogSink)> {
    let db = Db::open(path)?;
    let imported = db.with_mut(|c| db::import_config_lists(c, config))?;
    if imported > 0 {
        info!(rows = imported, "imported list sources from config.yaml into the database");
    }
    let sink = LogSink::spawn(db.writer_connection()?, CHANNEL_CAPACITY)?;
    Ok((Arc::new(db), sink))
}

/// Loads every list the runtime needs, then builds a snapshot.
///
/// With a database, each enabled row is loaded if `refresh` asks for it or if
/// it has no cached copy. Its status is written back to the row. Rows that
/// share a kind and name are merged into one list. Without a database, the
/// config's lists are loaded.
async fn assemble(
    http: &reqwest::Client,
    cache_dir: &Path,
    db: Option<&Db>,
    cache: &mut SourceCache,
    config: Config,
    refresh: &Refresh,
) -> Result<(Runtime, ReloadSummary)> {
    let (block, allow) = match db {
        Some(db) => load_from_db(http, cache_dir, db, cache, refresh).await?,
        None => {
            cache.clear();
            let block = load_lists_map(http, &config.lists, cache_dir).await;
            let allow = load_lists_map(http, &config.allow_lists, cache_dir).await;
            (block, allow)
        }
    };
    let summary = summarize(&block, &allow);
    let allow_lists: Vec<(String, Arc<ListData>)> = allow.into_iter().collect();
    let runtime = Runtime::build_with(config, block, allow_lists)?;
    Ok((runtime, summary))
}

type Lists = BTreeMap<String, Arc<ListData>>;

async fn load_from_db(
    http: &reqwest::Client,
    cache_dir: &Path,
    db: &Db,
    cache: &mut SourceCache,
    refresh: &Refresh,
) -> Result<(Lists, Lists)> {
    let rows = db.with(db::lists_all)?;
    let mut live: HashSet<i64> = HashSet::new();
    let mut block_groups: BTreeMap<String, Vec<Arc<ListData>>> = BTreeMap::new();
    let mut allow_groups: BTreeMap<String, Vec<Arc<ListData>>> = BTreeMap::new();

    for row in rows {
        live.insert(row.id);
        if !row.enabled {
            cache.remove(&row.id);
            continue;
        }

        let wanted = match refresh {
            Refresh::All => true,
            Refresh::Rows(ids) => ids.contains(&row.id),
        };
        if wanted || !cache.contains_key(&row.id) {
            let format = db::parse_format(&row.format).unwrap_or(ListFormat::Auto);
            let cfg = ListConfig {
                sources: vec![row.url.clone()],
                format,
            };
            let data = load_list(http, &row.name, &cfg, cache_dir).await;
            let error = source_error(&data);
            let rules = data.domains.len() as i64;
            let now = Utc::now().timestamp_millis();
            db.with(|c| db::lists_set_status(c, row.id, rules, now, error.as_deref()))?;
            info!(list = %row.name, kind = %row.kind, rules, "list source loaded");
            cache.insert(row.id, Arc::new(data));
        }

        let Some(data) = cache.get(&row.id).cloned() else {
            continue;
        };
        let groups = if row.kind == "allow" {
            &mut allow_groups
        } else {
            &mut block_groups
        };
        groups.entry(row.name).or_default().push(data);
    }

    cache.retain(|id, _| live.contains(id));
    Ok((merge_groups(block_groups), merge_groups(allow_groups)))
}

/// The error text stored for a source: every problem the loader reported, or `None`.
fn source_error(data: &ListData) -> Option<String> {
    if data.source_errors.is_empty() {
        None
    } else {
        Some(data.source_errors.join("; "))
    }
}

/// Merges the sources that share a name into one list. A name with a single
/// source reuses that source's data without copying it.
fn merge_groups(groups: BTreeMap<String, Vec<Arc<ListData>>>) -> Lists {
    groups
        .into_iter()
        .map(|(name, parts)| {
            if let [only] = parts.as_slice() {
                return (name, only.clone());
            }
            let mut domains: HashSet<String> = HashSet::new();
            let mut merged = ListData {
                name: name.clone(),
                domains: Arc::new(HashSet::new()),
                skipped: 0,
                sources_total: 0,
                sources_ok: 0,
                source_errors: Vec::new(),
                loaded_at: Utc::now(),
            };
            for part in &parts {
                domains.extend(part.domains.iter().cloned());
                merged.skipped += part.skipped;
                merged.sources_total += part.sources_total;
                merged.sources_ok += part.sources_ok;
                merged.source_errors.extend(part.source_errors.iter().cloned());
                merged.loaded_at = merged.loaded_at.min(part.loaded_at);
            }
            merged.domains = Arc::new(domains);
            (name, Arc::new(merged))
        })
        .collect()
}

fn summarize(block: &Lists, allow: &Lists) -> ReloadSummary {
    let summary = |name: String, l: &ListData| ListSummary {
        name,
        domains: l.domains.len(),
        skipped: l.skipped,
        sources_total: l.sources_total,
        sources_ok: l.sources_ok,
        errors: l.source_errors.clone(),
    };
    let mut lists: Vec<ListSummary> = block.values().map(|l| summary(l.name.clone(), l)).collect();
    lists.extend(allow.values().map(|l| summary(format!("allow: {}", l.name), l)));
    ReloadSummary { lists }
}
