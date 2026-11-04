//! Query recording for the database. The DNS handler calls `LogSink::record`,
//! which does one non-blocking `try_send` onto a bounded channel. A single
//! writer thread drains the channel and inserts rows in batches, one
//! transaction per batch. A batch is flushed after `BATCH_MAX_ROWS` rows or
//! `FLUSH_INTERVAL`, whichever comes first. When the channel is full, the row
//! is dropped and a counter goes up. A DNS answer never waits on the database.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use tracing::warn;

/// Rows buffered between the DNS path and the writer.
pub const CHANNEL_CAPACITY: usize = 10_000;
/// Most rows written in one transaction.
pub const BATCH_MAX_ROWS: usize = 500;
/// Longest time a row waits before its batch is written.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// One answered query, ready to store.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryRow {
    /// Unix milliseconds.
    pub time_ms: i64,
    pub client_ip: String,
    /// Configured client name, or `unknown`.
    pub client_name: String,
    pub domain: String,
    pub qtype: String,
    pub result: String,
    pub upstream: String,
    pub elapsed_ms: f64,
    pub rule: String,
    /// The first A or AAAA address in the answer, if there was one.
    pub answer_ip: Option<String>,
}

/// What the writer receives. `Flush` asks it to write the pending batch now and
/// then acknowledge, so that shutdown can wait for queued rows to reach disk.
enum Msg {
    Row(QueryRow),
    Flush(SyncSender<()>),
}

/// Counters shared by the sink and the writer.
#[derive(Debug, Default)]
pub struct SinkCounters {
    /// Rows dropped because the channel was full or the writer was gone.
    pub dropped: AtomicU64,
    /// Rows written to the database.
    pub written: AtomicU64,
    /// Rows lost because their batch failed to write.
    pub write_errors: AtomicU64,
}

/// Cheap, cloneable handle used by the DNS path.
#[derive(Clone)]
pub struct LogSink {
    tx: SyncSender<Msg>,
    counters: Arc<SinkCounters>,
}

impl LogSink {
    /// A sink and its receiving end, with no writer thread. Tests use this to
    /// fill the channel. `spawn` builds on it.
    fn channel(capacity: usize) -> (Self, Receiver<Msg>) {
        let (tx, rx) = mpsc::sync_channel(capacity.max(1));
        (
            Self {
                tx,
                counters: Arc::new(SinkCounters::default()),
            },
            rx,
        )
    }

    /// Starts the writer thread on its own connection and returns the sink.
    /// The schema must already be migrated.
    pub fn spawn(conn: Connection, capacity: usize) -> Result<Self> {
        let (sink, rx) = Self::channel(capacity);
        let counters = sink.counters.clone();
        thread::Builder::new()
            .name("dnsui-querylog".into())
            .spawn(move || run_writer(conn, rx, counters))
            .context("starting the query log writer thread")?;
        Ok(sink)
    }

    /// Queues a row without waiting. Returns false when the row was dropped.
    pub fn record(&self, row: QueryRow) -> bool {
        match self.tx.try_send(Msg::Row(row)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Writes every row queued so far, then returns. Used at shutdown. Returns
    /// false if the writer did not confirm within `timeout`.
    pub fn flush(&self, timeout: Duration) -> bool {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        if self.tx.send(Msg::Flush(ack_tx)).is_err() {
            return false;
        }
        ack_rx.recv_timeout(timeout).is_ok()
    }

    pub fn counters(&self) -> &SinkCounters {
        &self.counters
    }
}

fn run_writer(mut conn: Connection, rx: Receiver<Msg>, counters: Arc<SinkCounters>) {
    let mut batch: Vec<QueryRow> = Vec::with_capacity(BATCH_MAX_ROWS);
    loop {
        // Block for the first message. A closed channel means every sink is gone.
        let Ok(first) = rx.recv() else {
            return;
        };

        let deadline = Instant::now() + FLUSH_INTERVAL;
        let mut acks: Vec<SyncSender<()>> = Vec::new();
        let mut closed = false;
        let mut next = Some(first);
        loop {
            match next.take() {
                Some(Msg::Row(row)) => batch.push(row),
                Some(Msg::Flush(ack)) => {
                    acks.push(ack);
                    break;
                }
                None => {}
            }
            if batch.len() >= BATCH_MAX_ROWS {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(msg) => next = Some(msg),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }

        flush(&mut conn, &batch, &counters);
        batch.clear();
        for ack in acks {
            // The requester may have timed out and gone away. That is fine.
            let _ = ack.send(());
        }
        if closed {
            return;
        }
    }
}

fn flush(conn: &mut Connection, rows: &[QueryRow], counters: &SinkCounters) {
    match insert_batch(conn, rows) {
        Ok(()) => {
            counters.written.fetch_add(rows.len() as u64, Ordering::Relaxed);
        }
        Err(e) => {
            counters
                .write_errors
                .fetch_add(rows.len() as u64, Ordering::Relaxed);
            warn!(error = %format!("{e:#}"), rows = rows.len(), "query log batch was not written");
        }
    }
}

/// Inserts rows and updates the per-minute rollups in one transaction.
pub fn insert_batch(conn: &mut Connection, rows: &[QueryRow]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut rollups: BTreeMap<i64, Rollup> = BTreeMap::new();
    let tx = conn.transaction()?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO queries
                (time, client_ip, client_name, domain, qtype, result, upstream, elapsed_ms, rule, answer_ip)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for row in rows {
            insert.execute(params![
                row.time_ms,
                row.client_ip,
                row.client_name,
                row.domain,
                row.qtype,
                row.result,
                row.upstream,
                row.elapsed_ms,
                row.rule,
                row.answer_ip,
            ])?;
            let minute = row.time_ms.div_euclid(60_000) * 60;
            rollups
                .entry(minute)
                .or_default()
                .add(&row.result, row.elapsed_ms);
        }
    }
    {
        let mut upsert = tx.prepare_cached(
            "INSERT INTO minute_stats (minute, total, blocked, cached, forwarded, sum_ms, ms_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (minute) DO UPDATE SET
                total     = total + excluded.total,
                blocked   = blocked + excluded.blocked,
                cached    = cached + excluded.cached,
                forwarded = forwarded + excluded.forwarded,
                sum_ms    = sum_ms + excluded.sum_ms,
                ms_count  = ms_count + excluded.ms_count",
        )?;
        for (minute, r) in &rollups {
            upsert.execute(params![
                minute,
                r.total,
                r.blocked,
                r.cached,
                r.forwarded,
                r.sum_ms,
                r.ms_count
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Counts for one minute. Averages are kept as a sum and a count, so that
/// merging minutes gives the true average.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rollup {
    pub total: i64,
    pub blocked: i64,
    pub cached: i64,
    pub forwarded: i64,
    pub sum_ms: f64,
    pub ms_count: i64,
}

impl Rollup {
    /// Adds one query. Results match the in-memory telemetry: `blocked`,
    /// `cached` and `forwarded` have their own counters, and other results
    /// (`allowed`, `rewritten`, `error`) count only toward the total.
    pub fn add(&mut self, result: &str, elapsed_ms: f64) {
        self.total += 1;
        self.sum_ms += elapsed_ms;
        self.ms_count += 1;
        match result {
            "blocked" => self.blocked += 1,
            "cached" => self.cached += 1,
            "forwarded" => self.forwarded += 1,
            _ => {}
        }
    }

    /// Mean latency in ms, or `None` with no timed queries.
    pub fn avg_ms(&self) -> Option<f64> {
        (self.ms_count > 0).then(|| self.sum_ms / self.ms_count as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, minute_totals};
    use std::path::PathBuf;

    fn row(time_ms: i64, result: &str, elapsed_ms: f64) -> QueryRow {
        QueryRow {
            time_ms,
            client_ip: "192.0.2.10".into(),
            client_name: "Laptop".into(),
            domain: "example.test".into(),
            qtype: "A".into(),
            result: result.into(),
            upstream: "u".into(),
            elapsed_ms,
            rule: String::new(),
            answer_ip: None,
        }
    }

    fn temp_db(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dnsui-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (dir.clone(), dir.join("dnsui.db"))
    }

    #[test]
    fn full_channel_drops_and_never_blocks() {
        let (sink, rx) = LogSink::channel(2);
        assert!(sink.record(row(1, "forwarded", 1.0)));
        assert!(sink.record(row(2, "forwarded", 1.0)));

        // Nothing drains the channel, so every further send must be dropped.
        // Each call must return at once, with no waiting on the full channel.
        let started = Instant::now();
        let mut accepted = 0;
        for i in 0..10_000 {
            if sink.record(row(i, "blocked", 0.5)) {
                accepted += 1;
            }
        }
        let elapsed = started.elapsed();

        assert_eq!(accepted, 0);
        assert_eq!(sink.counters().dropped.load(Ordering::Relaxed), 10_000);
        assert!(
            elapsed < Duration::from_secs(1),
            "10k sends on a full channel took {elapsed:?}"
        );
        drop(rx);
    }

    #[test]
    fn dropped_counter_counts_writer_gone() {
        let (sink, rx) = LogSink::channel(4);
        drop(rx);
        assert!(!sink.record(row(1, "forwarded", 1.0)));
        assert_eq!(sink.counters().dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn batch_insert_writes_rows_and_rollups() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&mut conn, crate::db::MIGRATIONS).unwrap();
        let base = 1_700_000_000_000_i64; // a whole minute boundary plus 0 ms
        let base = base - base.rem_euclid(60_000);

        let batch = vec![
            row(base + 1_000, "blocked", 1.0),
            row(base + 2_000, "blocked", 2.0),
            row(base + 3_000, "forwarded", 6.0),
            row(base + 61_000, "cached", 4.0), // next minute
        ];
        insert_batch(&mut conn, &batch).unwrap();

        let n: i64 = conn.query_row("SELECT COUNT(*) FROM queries", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 4);

        let minute = base / 1000;
        let (total, blocked, forwarded, sum, count): (i64, i64, i64, f64, i64) = conn
            .query_row(
                "SELECT total, blocked, forwarded, sum_ms, ms_count FROM minute_stats WHERE minute = ?1",
                params![minute],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!((total, blocked, forwarded, count), (3, 2, 1, 3));
        assert!((sum - 9.0).abs() < 1e-9);

        // A later batch adds into the existing minute instead of replacing it.
        insert_batch(&mut conn, &[row(base + 4_000, "blocked", 3.0)]).unwrap();
        let (total, blocked, sum, count): (i64, i64, f64, i64) = conn
            .query_row(
                "SELECT total, blocked, sum_ms, ms_count FROM minute_stats WHERE minute = ?1",
                params![minute],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((total, blocked, count), (4, 3, 4));
        // Average over the minute is the sum over the count, not an average of averages.
        assert!((sum / count as f64 - 3.0).abs() < 1e-9);

        let totals = minute_totals(&conn, minute, minute + 120).unwrap();
        assert_eq!(totals.total, 5);
        assert_eq!(totals.blocked, 3);
        assert_eq!(totals.cached, 1);
        // Two minutes in the window: (9 + 3 + 4) ms over 5 timed queries = 3.2 ms.
        assert!((totals.avg_ms().unwrap() - 3.2).abs() < 1e-9);
    }

    #[test]
    fn rollup_average_is_sum_over_count() {
        // Averaging the two per-minute averages would give (20 + 2) / 2 = 11.
        // The true average over four samples is (60 + 2) / 4 = 15.5.
        let mut a = Rollup::default();
        for ms in [10.0, 20.0, 30.0] {
            a.add("blocked", ms);
        }
        let mut b = Rollup::default();
        b.add("forwarded", 2.0);
        assert!((a.avg_ms().unwrap() - 20.0).abs() < 1e-9);
        assert!((b.avg_ms().unwrap() - 2.0).abs() < 1e-9);

        // The same two minutes, through the database, give the true average.
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&mut conn, crate::db::MIGRATIONS).unwrap();
        let base = 1_700_000_040_000_i64; // a minute boundary
        let mut rows = vec![
            row(base, "blocked", 10.0),
            row(base + 1, "blocked", 20.0),
            row(base + 2, "blocked", 30.0),
        ];
        rows.push(row(base + 60_000, "forwarded", 2.0));
        insert_batch(&mut conn, &rows).unwrap();
        let totals = minute_totals(&conn, base / 1000, base / 1000 + 120).unwrap();
        assert_eq!(totals.total, 4);
        assert!((totals.avg_ms().unwrap() - 15.5).abs() < 1e-9);
        assert_eq!(Rollup::default().avg_ms(), None);
    }

    #[test]
    fn flush_writes_pending_rows_before_the_interval() {
        let (dir, path) = temp_db("flush");
        let db = Db::open(&path).unwrap();
        let sink = LogSink::spawn(db.writer_connection().unwrap(), CHANNEL_CAPACITY).unwrap();
        for i in 0..7 {
            assert!(sink.record(row(1_700_000_200_000 + i, "forwarded", 1.0)));
        }
        // Well under the batch size and the interval, so only flush can write these now.
        let started = Instant::now();
        assert!(sink.flush(Duration::from_secs(5)));
        assert!(started.elapsed() < FLUSH_INTERVAL, "flush waited for the interval");
        assert_eq!(count_queries(&db), 7);
        assert_eq!(sink.counters().written.load(Ordering::Relaxed), 7);
        drop(sink);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn writer_thread_batches_and_flushes_on_interval() {
        let (dir, path) = temp_db("writer");
        let db = Db::open(&path).unwrap();
        let sink = LogSink::spawn(db.writer_connection().unwrap(), CHANNEL_CAPACITY).unwrap();

        // A small burst is below the batch limit, so only the interval can flush it.
        for i in 0..3 {
            assert!(sink.record(row(1_700_000_000_000 + i, "forwarded", 1.0)));
        }
        wait_for(|| sink.counters().written.load(Ordering::Relaxed) == 3, Duration::from_secs(5));
        assert_eq!(count_queries(&db), 3);

        // A burst well above the batch limit is written in several transactions.
        let total = BATCH_MAX_ROWS * 3 + 17;
        for i in 0..total {
            assert!(sink.record(row(1_700_000_100_000 + i as i64, "blocked", 1.0)));
        }
        wait_for(
            || sink.counters().written.load(Ordering::Relaxed) == (3 + total) as u64,
            Duration::from_secs(10),
        );
        assert_eq!(count_queries(&db), (3 + total) as i64);
        assert_eq!(sink.counters().write_errors.load(Ordering::Relaxed), 0);

        drop(sink);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn count_queries(db: &Db) -> i64 {
        db.with(|c| Ok(c.query_row("SELECT COUNT(*) FROM queries", [], |r| r.get(0))?))
            .unwrap()
    }

    fn wait_for(cond: impl Fn() -> bool, timeout: Duration) {
        let started = Instant::now();
        while !cond() && started.elapsed() < timeout {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(cond(), "condition not met within {timeout:?}");
    }
}
