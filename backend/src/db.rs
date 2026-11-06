//! SQLite history store (`dnsui.db` in the data directory).
//!
//! Holds the query log, per-minute rollups, and the block and allow list
//! sources. The DNS path never touches this module directly. It hands rows to
//! the writer in `recorder.rs`. API handlers read through `Db::with`, which
//! runs on a blocking thread.
//!
//! Schema versioning uses `PRAGMA user_version`. `MIGRATIONS` lists one entry
//! per version. Opening a database applies every entry above its current
//! version, in order, each in its own transaction.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};

use crate::config::{Config, ListConfig, ListFormat};
use crate::recorder::Rollup;

const MS_PER_DAY: i64 = 86_400_000;
const SECS_PER_DAY: i64 = 86_400;
/// Rows removed per DELETE statement, so retention never holds the write lock for long.
const RETENTION_CHUNK: i64 = 5_000;
/// Meta key recording that config lists were imported into the lists table.
const LISTS_IMPORTED_KEY: &str = "config_lists_imported";

/// One schema migration. `version` is the version reached after `apply` runs.
pub struct Migration {
    pub version: i64,
    pub apply: fn(&Transaction) -> rusqlite::Result<()>,
}

/// Every schema version this build knows. Add a new entry with the next
/// version number to change the schema.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        apply: create_schema_v1,
    },
    Migration {
        version: 2,
        apply: create_schema_v2,
    },
];

/// The schema version this build writes (the highest entry in `MIGRATIONS`).
pub const SCHEMA_VERSION: i64 = 2;

/// Adds the first answer IP, so the dashboard can map destinations to countries.
/// Rows written before this migration keep NULL and are left out of the map.
fn create_schema_v2(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "ALTER TABLE queries ADD COLUMN answer_ip TEXT;
         CREATE INDEX IF NOT EXISTS queries_answer_ip ON queries (answer_ip, time);",
    )
}

fn create_schema_v1(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS queries (
            id          INTEGER PRIMARY KEY,
            time        INTEGER NOT NULL,
            client_ip   TEXT NOT NULL,
            client_name TEXT NOT NULL,
            domain      TEXT NOT NULL,
            qtype       TEXT NOT NULL,
            result      TEXT NOT NULL,
            upstream    TEXT NOT NULL,
            elapsed_ms  REAL NOT NULL,
            rule        TEXT NOT NULL DEFAULT ''
         );
         CREATE INDEX IF NOT EXISTS queries_time ON queries (time);
         CREATE INDEX IF NOT EXISTS queries_domain ON queries (domain);
         CREATE INDEX IF NOT EXISTS queries_client_ip ON queries (client_ip);
         CREATE TABLE IF NOT EXISTS minute_stats (
            minute    INTEGER PRIMARY KEY,
            total     INTEGER NOT NULL DEFAULT 0,
            blocked   INTEGER NOT NULL DEFAULT 0,
            cached    INTEGER NOT NULL DEFAULT 0,
            forwarded INTEGER NOT NULL DEFAULT 0,
            sum_ms    REAL NOT NULL DEFAULT 0,
            ms_count  INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS lists (
            id           INTEGER PRIMARY KEY,
            kind         TEXT NOT NULL CHECK (kind IN ('block', 'allow')),
            name         TEXT NOT NULL,
            url          TEXT NOT NULL,
            enabled      INTEGER NOT NULL DEFAULT 1,
            format       TEXT NOT NULL DEFAULT 'auto',
            rules        INTEGER NOT NULL DEFAULT 0,
            last_updated INTEGER,
            error        TEXT
         );
         CREATE INDEX IF NOT EXISTS lists_kind_name ON lists (kind, name);",
    )
}

/// Applies every migration above the database's current version. Returns the
/// version now in effect. Refuses a database written by a newer build.
pub fn migrate(conn: &mut Connection, migrations: &[Migration]) -> Result<i64> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let mut pending: Vec<&Migration> = migrations.iter().filter(|m| m.version > current).collect();
    pending.sort_by_key(|m| m.version);
    let latest = migrations.iter().map(|m| m.version).max().unwrap_or(0);
    if current > latest {
        bail!("database schema version {current} is newer than this build supports ({latest})");
    }
    for m in pending {
        let tx = conn.transaction()?;
        (m.apply)(&tx).with_context(|| format!("applying schema version {}", m.version))?;
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
    }
    Ok(latest)
}

/// Opens a connection with the pragmas every connection in this program uses.
pub fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("opening database {}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}

/// The database handle shared by the API. Reads and list edits go through
/// `with`/`with_mut`. The DNS log writer owns a separate connection (see
/// `recorder.rs`), so SQLite WAL keeps readers and the writer from blocking each other.
pub struct Db {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Db {
    /// Opens (creating if needed) the database and brings the schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating data directory {}", dir.display()))?;
        }
        let mut conn = connect(path)?;
        migrate(&mut conn, MIGRATIONS)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        })
    }

    /// A new connection to the same file, for the log writer. The schema must already be migrated.
    pub fn writer_connection(&self) -> Result<Connection> {
        connect(&self.path)
    }

    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.conn.lock().map_err(|_| anyhow!("database lock poisoned"))?;
        f(&guard)
    }

    pub fn with_mut<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().map_err(|_| anyhow!("database lock poisoned"))?;
        f(&mut guard)
    }

    /// Health check. Returns an error when the file cannot be queried.
    pub fn ping(&self) -> Result<()> {
        self.with(|c| {
            c.query_row("SELECT 1", [], |_| Ok(()))?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

/// One row of the `lists` table. A row is one source. Rows that share a
/// `kind` and `name` are merged into one runtime list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRecord {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub format: String,
    pub rules: i64,
    /// Unix milliseconds of the last load attempt.
    pub last_updated: Option<i64>,
    pub error: Option<String>,
}

/// Fields that may change on an existing list. `None` leaves a field as it is.
#[derive(Debug, Clone, Default)]
pub struct ListPatch {
    pub name: Option<String>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub format: Option<String>,
}

const LIST_COLUMNS: &str = "id, kind, name, url, enabled, format, rules, last_updated, error";

fn list_from_row(r: &rusqlite::Row) -> rusqlite::Result<ListRecord> {
    Ok(ListRecord {
        id: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        url: r.get(3)?,
        enabled: r.get::<_, i64>(4)? != 0,
        format: r.get(5)?,
        rules: r.get(6)?,
        last_updated: r.get(7)?,
        error: r.get(8)?,
    })
}

pub fn lists_all(conn: &Connection) -> Result<Vec<ListRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {LIST_COLUMNS} FROM lists ORDER BY kind, name, id"
    ))?;
    let rows = stmt
        .query_map([], list_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn lists_get(conn: &Connection, id: i64) -> Result<Option<ListRecord>> {
    conn.query_row(
        &format!("SELECT {LIST_COLUMNS} FROM lists WHERE id = ?1"),
        params![id],
        list_from_row,
    )
    .optional()
    .map_err(Into::into)
}

pub fn lists_insert(
    conn: &Connection,
    kind: &str,
    name: &str,
    url: &str,
    enabled: bool,
    format: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO lists (kind, name, url, enabled, format) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![kind, name, url, enabled as i64, format],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Applies a patch. Returns false when no list has that id.
pub fn lists_update(conn: &Connection, id: i64, patch: &ListPatch) -> Result<bool> {
    let n = conn.execute(
        "UPDATE lists SET
            name    = COALESCE(?1, name),
            url     = COALESCE(?2, url),
            enabled = COALESCE(?3, enabled),
            format  = COALESCE(?4, format)
         WHERE id = ?5",
        params![
            patch.name,
            patch.url,
            patch.enabled.map(|b| b as i64),
            patch.format,
            id
        ],
    )?;
    Ok(n > 0)
}

pub fn lists_set_status(
    conn: &Connection,
    id: i64,
    rules: i64,
    last_updated: i64,
    error: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE lists SET rules = ?1, last_updated = ?2, error = ?3 WHERE id = ?4",
        params![rules, last_updated, error, id],
    )?;
    Ok(())
}

pub fn lists_delete(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.execute("DELETE FROM lists WHERE id = ?1", params![id])? > 0)
}

pub fn format_name(format: ListFormat) -> &'static str {
    match format {
        ListFormat::Auto => "auto",
        ListFormat::Hosts => "hosts",
        ListFormat::Plain => "plain",
        ListFormat::Abp => "abp",
    }
}

pub fn parse_format(raw: &str) -> Option<ListFormat> {
    match raw {
        "auto" => Some(ListFormat::Auto),
        "hosts" => Some(ListFormat::Hosts),
        "plain" => Some(ListFormat::Plain),
        "abp" => Some(ListFormat::Abp),
        _ => None,
    }
}

/// One-time import of the config's `lists` (block) and `allow_lists` (allow)
/// into the lists table. Each source becomes one row. It runs only when the
/// import flag is unset and the table is empty, so lists the user deleted
/// are not brought back on the next start. Returns the number of rows inserted.
pub fn import_config_lists(conn: &mut Connection, config: &Config) -> Result<usize> {
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![LISTS_IMPORTED_KEY],
            |r| r.get(0),
        )
        .optional()?;
    if already.is_some() {
        return Ok(0);
    }

    let tx = conn.transaction()?;
    let existing: i64 = tx.query_row("SELECT COUNT(*) FROM lists", [], |r| r.get(0))?;
    let mut inserted = 0;
    if existing == 0 {
        let sets: [(&str, &BTreeMap<String, ListConfig>); 2] = [
            ("block", &config.lists),
            ("allow", &config.allow_lists),
        ];
        for (kind, lists) in sets {
            for (name, list) in lists {
                for source in &list.sources {
                    tx.execute(
                        "INSERT INTO lists (kind, name, url, enabled, format)
                         VALUES (?1, ?2, ?3, 1, ?4)",
                        params![kind, name, source.trim(), format_name(list.format)],
                    )?;
                    inserted += 1;
                }
            }
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')",
        params![LISTS_IMPORTED_KEY],
    )?;
    tx.commit()?;
    Ok(inserted)
}

// ---------------------------------------------------------------------------
// Query log
// ---------------------------------------------------------------------------

/// Optional filters for the query log. Every value is bound as a parameter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryFilter {
    /// Inclusive lower bound, unix ms.
    pub from_ms: Option<i64>,
    /// Inclusive upper bound, unix ms.
    pub to_ms: Option<i64>,
    /// Case-insensitive substring of the domain.
    pub domain: Option<String>,
    /// Exact match on any of these client names. An empty list matches nothing.
    pub clients: Option<Vec<String>>,
    /// Exact match on the result (`blocked`, `forwarded`, ...).
    pub result: Option<String>,
}

/// Builds the WHERE clause for a filter. The SQL text holds only fixed
/// fragments and `?` placeholders. Every user value goes into the returned
/// parameter list, so no input can change the statement.
pub fn query_where(f: &QueryFilter) -> (String, Vec<Value>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if let Some(from) = f.from_ms {
        clauses.push("time >= ?".into());
        params.push(Value::Integer(from));
    }
    if let Some(to) = f.to_ms {
        clauses.push("time <= ?".into());
        params.push(Value::Integer(to));
    }
    if let Some(d) = f.domain.as_deref().filter(|d| !d.is_empty()) {
        clauses.push("instr(lower(domain), lower(?)) > 0".into());
        params.push(Value::Text(d.to_string()));
    }
    if let Some(names) = &f.clients {
        if names.is_empty() {
            clauses.push("0".into());
        } else {
            let marks = vec!["?"; names.len()].join(", ");
            clauses.push(format!("client_name IN ({marks})"));
            params.extend(names.iter().map(|n| Value::Text(n.clone())));
        }
    }
    if let Some(r) = f.result.as_deref().filter(|r| !r.is_empty()) {
        clauses.push("result = ?".into());
        params.push(Value::Text(r.to_string()));
    }

    let sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    (sql, params)
}

/// One stored query, with time as unix ms.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryRecord {
    pub time_ms: i64,
    pub client_ip: String,
    pub client_name: String,
    pub domain: String,
    pub qtype: String,
    pub result: String,
    pub upstream: String,
    pub elapsed_ms: f64,
    pub rule: String,
}

/// One page of matching queries, newest first, plus the total number of matches.
pub fn query_page(
    conn: &Connection,
    f: &QueryFilter,
    limit: i64,
    offset: i64,
) -> Result<(Vec<QueryRecord>, i64)> {
    let (where_sql, params) = query_where(f);

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM queries{where_sql}"),
        params_from_iter(params.iter()),
        |r| r.get(0),
    )?;

    let mut page_params = params;
    page_params.push(Value::Integer(limit));
    page_params.push(Value::Integer(offset));
    let mut stmt = conn.prepare(&format!(
        "SELECT time, client_ip, client_name, domain, qtype, result, upstream, elapsed_ms, rule
         FROM queries{where_sql}
         ORDER BY time DESC, id DESC
         LIMIT ? OFFSET ?"
    ))?;
    let rows = stmt
        .query_map(params_from_iter(page_params.iter()), |r| {
            Ok(QueryRecord {
                time_ms: r.get(0)?,
                client_ip: r.get(1)?,
                client_name: r.get(2)?,
                domain: r.get(3)?,
                qtype: r.get(4)?,
                result: r.get(5)?,
                upstream: r.get(6)?,
                elapsed_ms: r.get(7)?,
                rule: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((rows, total))
}

/// Distinct client names seen in the retained history (capped).
pub fn distinct_client_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT client_name FROM queries ORDER BY client_name LIMIT 10000",
    )?;
    let names = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(names)
}

// ---------------------------------------------------------------------------
// Stats and history
// ---------------------------------------------------------------------------

/// Totals for minutes in `[from_s, to_s)`, unix seconds. The result is a
/// `Rollup`, so `avg_ms` gives the true average over the window.
pub fn minute_totals(conn: &Connection, from_s: i64, to_s: i64) -> Result<Rollup> {
    let t = conn.query_row(
        "SELECT COALESCE(SUM(total), 0), COALESCE(SUM(blocked), 0),
                COALESCE(SUM(cached), 0), COALESCE(SUM(forwarded), 0),
                COALESCE(SUM(sum_ms), 0.0), COALESCE(SUM(ms_count), 0)
         FROM minute_stats WHERE minute >= ?1 AND minute < ?2",
        params![from_s, to_s],
        |r| {
            Ok(Rollup {
                total: r.get(0)?,
                blocked: r.get(1)?,
                cached: r.get(2)?,
                forwarded: r.get(3)?,
                sum_ms: r.get(4)?,
                ms_count: r.get(5)?,
            })
        },
    )?;
    Ok(t)
}

/// Totals grouped into buckets of `bucket_s` seconds, starting at `from_s`.
/// Returns `(bucket index, total, blocked)` for buckets that have data.
pub fn minute_series(
    conn: &Connection,
    from_s: i64,
    to_s: i64,
    bucket_s: i64,
) -> Result<Vec<(i64, i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT (minute - ?1) / ?3 AS idx, SUM(total), SUM(blocked)
         FROM minute_stats WHERE minute >= ?1 AND minute < ?2
         GROUP BY idx ORDER BY idx",
    )?;
    let rows = stmt
        .query_map(params![from_s, to_s, bucket_s], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Most queried domains in `[from_ms, to_ms)`. With `blocked_only`, counts only blocked queries.
pub fn top_domains(
    conn: &Connection,
    from_ms: i64,
    to_ms: i64,
    blocked_only: bool,
    limit: i64,
) -> Result<Vec<(String, i64)>> {
    let extra = if blocked_only { " AND result = 'blocked'" } else { "" };
    let mut stmt = conn.prepare(&format!(
        "SELECT domain, COUNT(*) AS c FROM queries
         WHERE time >= ?1 AND time < ?2{extra}
         GROUP BY domain ORDER BY c DESC, domain ASC LIMIT ?3"
    ))?;
    let rows = stmt
        .query_map(params![from_ms, to_ms, limit], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Where a block rule came from, as a SQL expression over `queries.rule`.
/// `list:<name>:<domain>` gives the list name. Other block rules are grouped as
/// client or custom rules.
const BLOCK_SOURCE_SQL: &str = "CASE
    WHEN rule LIKE 'list:%' THEN substr(rule, 6, instr(substr(rule, 6), ':') - 1)
    WHEN rule LIKE 'block:client%' THEN 'Client rules'
    WHEN rule LIKE 'block:%' THEN 'Custom rules'
    ELSE 'Other'
END";

/// Every domain with its query count in the window, most queried first.
/// Used to build root-domain and company totals, which need all domains.
pub fn all_domain_counts(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT domain, COUNT(*) AS c FROM queries
         WHERE time >= ?1 AND time < ?2
         GROUP BY domain ORDER BY c DESC, domain ASC",
    )?;
    let rows = stmt
        .query_map(params![from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Blocked queries grouped by domain and block source. The caller picks the
/// main source for each domain.
pub fn blocked_by_domain_and_source(
    conn: &Connection,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<(String, String, i64)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT domain, {BLOCK_SOURCE_SQL} AS source, COUNT(*) AS c FROM queries
         WHERE time >= ?1 AND time < ?2 AND result = 'blocked'
         GROUP BY domain, source ORDER BY c DESC"
    ))?;
    let rows = stmt
        .query_map(params![from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Blocked queries per block source, largest first.
pub fn block_reason_counts(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {BLOCK_SOURCE_SQL} AS source, COUNT(*) AS c FROM queries
         WHERE time >= ?1 AND time < ?2 AND result = 'blocked'
         GROUP BY source ORDER BY c DESC, source ASC"
    ))?;
    let rows = stmt
        .query_map(params![from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Answer addresses with their query counts. Rows without an address are skipped.
pub fn answer_ip_counts(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT answer_ip, COUNT(*) AS c FROM queries
         WHERE time >= ?1 AND time < ?2 AND answer_ip IS NOT NULL
         GROUP BY answer_ip ORDER BY c DESC",
    )?;
    let rows = stmt
        .query_map(params![from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn active_clients(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(DISTINCT client_ip) FROM queries WHERE time >= ?1 AND time < ?2",
        params![from_ms, to_ms],
        |r| r.get(0),
    )?)
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// Deletes queries older than `query_days` and rollups older than `stats_days`,
/// measured from `now_ms`. Deletes in chunks so that the write lock is held
/// only briefly each time. Returns `(queries_deleted, minutes_deleted)`.
pub fn apply_retention(
    conn: &Connection,
    now_ms: i64,
    query_days: u32,
    stats_days: u32,
) -> Result<(usize, usize)> {
    let query_cut = now_ms - i64::from(query_days) * MS_PER_DAY;
    let stats_cut = now_ms.div_euclid(1000) - i64::from(stats_days) * SECS_PER_DAY;

    let mut queries = 0usize;
    loop {
        let n = conn.execute(
            "DELETE FROM queries WHERE id IN
                (SELECT id FROM queries WHERE time < ?1 LIMIT ?2)",
            params![query_cut, RETENTION_CHUNK],
        )?;
        queries += n;
        if (n as i64) < RETENTION_CHUNK {
            break;
        }
    }

    let mut minutes = 0usize;
    loop {
        let n = conn.execute(
            "DELETE FROM minute_stats WHERE minute IN
                (SELECT minute FROM minute_stats WHERE minute < ?1 LIMIT ?2)",
            params![stats_cut, RETENTION_CHUNK],
        )?;
        minutes += n;
        if (n as i64) < RETENTION_CHUNK {
            break;
        }
    }
    Ok((queries, minutes))
}

/// Rewrites the database file to give free pages back to the disk. Returns
/// `(bytes_before, bytes_after)` of the main file, measured after a WAL checkpoint.
/// Needs free disk space about the size of the database, and the write lock for
/// the duration, so run it with the server stopped.
pub fn vacuum(path: &Path) -> Result<(u64, u64)> {
    let conn = connect(path)?;
    let size = || -> Result<u64> {
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .context("checkpointing the WAL")?;
        Ok(std::fs::metadata(path)
            .with_context(|| format!("reading {}", path.display()))?
            .len())
    };
    let before = size()?;
    conn.execute_batch("VACUUM").context("running VACUUM")?;
    let after = size()?;
    Ok((before, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacuum_returns_deleted_space_to_the_disk() {
        let path = std::env::temp_dir().join(format!("dnsui-vacuum-{}.db", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        {
            let mut conn = connect(&path).unwrap();
            migrate(&mut conn, MIGRATIONS).unwrap();
            let tx = conn.transaction().unwrap();
            for i in 0..20_000 {
                tx.execute(
                    "INSERT INTO queries (time, client_ip, client_name, domain, qtype, result, upstream, elapsed_ms)
                     VALUES (?1, '10.0.0.1', 'pc', ?2, 'A', 'forwarded', 'u', 1.0)",
                    params![i, format!("host{i}.example.com")],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            conn.execute("DELETE FROM queries WHERE time >= 100", []).unwrap();
        }
        let (before, after) = vacuum(&path).unwrap();
        assert!(after < before / 4, "expected a large shrink, got {before} -> {after}");
        let conn = connect(&path).unwrap();
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM queries", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 100, "VACUUM must not lose the rows that remain");
        drop(conn);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    fn mem() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, MIGRATIONS).unwrap();
        conn
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    }

    fn lists_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM lists", [], |r| r.get(0)).unwrap()
    }

    fn config_with_lists() -> Config {
        Config::parse_yaml(
            r#"
bootstrap: ["9.9.9.10"]
upstream_groups:
  default: ["https://dns10.quad9.net/dns-query"]
lists:
  ads:
    sources: ["https://example.com/a.txt", "/etc/dnsui/b.txt"]
    format: abp
allow_lists:
  trusted:
    sources: ["https://example.com/allow.txt"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn fresh_database_gets_tables_and_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        let version = migrate(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let stored: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, SCHEMA_VERSION);
        for t in ["meta", "queries", "minute_stats", "lists"] {
            assert!(table_exists(&conn, t), "missing table {t}");
        }
    }

    #[test]
    fn migrate_is_idempotent_and_runs_new_hooks_in_order() {
        fn create_extra(tx: &Transaction) -> rusqlite::Result<()> {
            tx.execute_batch("CREATE TABLE extra_v2 (x INTEGER);")
        }
        let mut conn = mem();
        let mut all: Vec<Migration> = MIGRATIONS
            .iter()
            .map(|m| Migration { version: m.version, apply: m.apply })
            .collect();
        all.push(Migration { version: 3, apply: create_extra });
        // Reverse order on purpose: migrate must sort by version.
        all.reverse();
        assert_eq!(migrate(&mut conn, &all).unwrap(), 3);
        assert!(table_exists(&conn, "extra_v2"));
        // A second run applies nothing and does not fail on existing tables.
        assert_eq!(migrate(&mut conn, &all).unwrap(), 3);
    }

    #[test]
    fn refuses_database_from_newer_build() {
        let mut conn = mem();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(migrate(&mut conn, MIGRATIONS).is_err());
    }

    #[test]
    fn config_lists_import_once_into_block_and_allow_rows() {
        let mut conn = mem();
        let cfg = config_with_lists();
        let inserted = import_config_lists(&mut conn, &cfg).unwrap();
        assert_eq!(inserted, 3, "two sources for ads, one for trusted");

        let rows = lists_all(&conn).unwrap();
        let block: Vec<_> = rows.iter().filter(|r| r.kind == "block").collect();
        let allow: Vec<_> = rows.iter().filter(|r| r.kind == "allow").collect();
        assert_eq!(block.len(), 2);
        assert!(block.iter().all(|r| r.name == "ads" && r.format == "abp" && r.enabled));
        assert_eq!(allow.len(), 1);
        assert_eq!(allow[0].name, "trusted");

        // A second import is a no-op.
        assert_eq!(import_config_lists(&mut conn, &cfg).unwrap(), 0);
        assert_eq!(lists_count(&conn), 3);
    }

    #[test]
    fn deleted_lists_are_not_reimported() {
        let mut conn = mem();
        let cfg = config_with_lists();
        import_config_lists(&mut conn, &cfg).unwrap();
        for row in lists_all(&conn).unwrap() {
            lists_delete(&conn, row.id).unwrap();
        }
        // The table is empty again, but the import flag is set, so nothing comes back.
        assert_eq!(import_config_lists(&mut conn, &cfg).unwrap(), 0);
        assert_eq!(lists_count(&conn), 0);
    }

    #[test]
    fn list_crud_and_status() {
        let conn = mem();
        let id = lists_insert(&conn, "block", "ads", "https://x.test/h.txt", true, "auto").unwrap();
        lists_set_status(&conn, id, 42, 1_700_000_000_000, Some("download failed")).unwrap();
        let got = lists_get(&conn, id).unwrap().unwrap();
        assert_eq!(got.rules, 42);
        assert_eq!(got.last_updated, Some(1_700_000_000_000));
        assert_eq!(got.error.as_deref(), Some("download failed"));

        let patch = ListPatch {
            name: Some("ads2".into()),
            enabled: Some(false),
            ..Default::default()
        };
        assert!(lists_update(&conn, id, &patch).unwrap());
        let got = lists_get(&conn, id).unwrap().unwrap();
        assert_eq!(got.name, "ads2");
        assert!(!got.enabled);
        assert_eq!(got.url, "https://x.test/h.txt", "untouched fields stay");

        assert!(lists_delete(&conn, id).unwrap());
        assert!(!lists_delete(&conn, id).unwrap());
        assert!(lists_get(&conn, id).unwrap().is_none());
    }

    #[test]
    fn retention_deletes_only_rows_past_the_window() {
        let conn = mem();
        let now_ms: i64 = 2_000_000_000_000;
        let day = MS_PER_DAY;
        // One query 31 days old (deleted) and one 29 days old (kept).
        for (t, domain) in [(now_ms - 31 * day, "old.test"), (now_ms - 29 * day, "new.test")] {
            conn.execute(
                "INSERT INTO queries (time, client_ip, client_name, domain, qtype, result, upstream, elapsed_ms, rule)
                 VALUES (?1, '10.0.0.1', 'Laptop', ?2, 'A', 'forwarded', 'u', 1.0, '')",
                params![t, domain],
            )
            .unwrap();
        }
        // Minute rollups: 400 days old (deleted) and 300 days old (kept).
        let now_s = now_ms / 1000;
        for minute in [now_s - 400 * 86_400, now_s - 300 * 86_400] {
            conn.execute(
                "INSERT INTO minute_stats (minute, total) VALUES (?1, 1)",
                params![minute],
            )
            .unwrap();
        }

        let (q, m) = apply_retention(&conn, now_ms, 30, 365).unwrap();
        assert_eq!((q, m), (1, 1));
        let remaining: String = conn
            .query_row("SELECT domain FROM queries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, "new.test");
        assert_eq!(minute_totals(&conn, 0, i64::MAX).unwrap().total, 1);
    }

    #[test]
    fn filter_builder_binds_every_user_value() {
        let hostile = "x'; DROP TABLE queries; --";
        let f = QueryFilter {
            from_ms: Some(1),
            to_ms: Some(2),
            domain: Some(hostile.into()),
            clients: Some(vec![hostile.into(), "Laptop".into()]),
            result: Some(hostile.into()),
        };
        let (sql, params) = query_where(&f);
        assert!(!sql.contains("DROP"), "user input leaked into SQL: {sql}");
        assert!(!sql.contains("Laptop"));
        assert!(!sql.contains('\''));
        // from, to, domain, two client names, result.
        assert_eq!(params.len(), 6);
        assert!(params.contains(&Value::Text(hostile.into())));

        // The hostile text is treated as data: the table survives and matches nothing.
        let conn = mem();
        let (rows, total) = query_page(&conn, &f, 10, 0).unwrap();
        assert!(rows.is_empty());
        assert_eq!(total, 0);
        assert!(table_exists(&conn, "queries"));
    }

    #[test]
    fn filter_matches_domain_case_insensitively_and_by_client() {
        let conn = mem();
        for (domain, client, result) in [
            ("Ads.Example.com", "Laptop", "blocked"),
            ("news.test", "Laptop", "forwarded"),
            ("ads.other.test", "WiFi", "blocked"),
        ] {
            conn.execute(
                "INSERT INTO queries (time, client_ip, client_name, domain, qtype, result, upstream, elapsed_ms, rule)
                 VALUES (1000, '10.0.0.1', ?1, ?2, 'A', ?3, 'u', 1.0, '')",
                params![client, domain, result],
            )
            .unwrap();
        }
        let by_domain = QueryFilter { domain: Some("ADS.".into()), ..Default::default() };
        assert_eq!(query_page(&conn, &by_domain, 100, 0).unwrap().1, 2);

        let by_client = QueryFilter {
            clients: Some(vec!["Laptop".into()]),
            result: Some("blocked".into()),
            ..Default::default()
        };
        let (rows, total) = query_page(&conn, &by_client, 100, 0).unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].domain, "Ads.Example.com");
    }
}
