use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::sync::Mutex;
use crate::models::{UsageRecord, QuotaSnapshot, DailySummary, RecordingSession, RecordingSessionStatus, RecordingInterval, RecordingSessionEvent, SessionPodSnapshot};

pub struct Database {
    conn: Mutex<Connection>,
}

unsafe impl Send for Database {}
unsafe impl Sync for Database {}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn initialize(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
                id          TEXT PRIMARY KEY,
                provider    TEXT NOT NULL,
                plan        TEXT,
                email       TEXT,
                display_name TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                metadata    TEXT
            );

            CREATE TABLE IF NOT EXISTS quota_snapshots (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id  TEXT NOT NULL REFERENCES accounts(id),
                timestamp   TEXT NOT NULL,
                credits_used    REAL,
                credits_total   REAL,
                reset_at        TEXT,
                metadata        TEXT,
                UNIQUE(account_id, timestamp)
            );

            CREATE TABLE IF NOT EXISTS usage_events (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id      TEXT NOT NULL,
                agent           TEXT,
                session_id      TEXT,
                timestamp       TEXT NOT NULL,
                provider        TEXT NOT NULL,
                model           TEXT,
                input_tokens    INTEGER,
                output_tokens   INTEGER,
                total_tokens    INTEGER,
                credits_consumed REAL,
                duration_ms     INTEGER,
                context_usage_pct REAL,
                metadata        TEXT
            );

            CREATE TABLE IF NOT EXISTS daily_summary (
                date        TEXT NOT NULL,
                account_id  TEXT NOT NULL,
                agent       TEXT NOT NULL,
                provider    TEXT NOT NULL,
                total_credits       REAL DEFAULT 0,
                total_input_tokens  INTEGER DEFAULT 0,
                total_output_tokens INTEGER DEFAULT 0,
                total_tokens        INTEGER DEFAULT 0,
                request_count       INTEGER DEFAULT 0,
                avg_duration_ms     INTEGER DEFAULT 0,
                PRIMARY KEY(date, account_id, agent)
            );

            CREATE TABLE IF NOT EXISTS collection_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                provider    TEXT NOT NULL,
                agent       TEXT,
                timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
                status      TEXT NOT NULL,
                message     TEXT,
                records_count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS recording_sessions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                status      TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                UNIQUE(name)
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_recording_one_open
            ON recording_sessions(CASE WHEN status IN ('active','paused') THEN 1 END)
            WHERE status IN ('active','paused');

            CREATE TABLE IF NOT EXISTS recording_intervals (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  INTEGER NOT NULL REFERENCES recording_sessions(id) ON DELETE CASCADE,
                started_at  TEXT NOT NULL,
                ended_at    TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_intervals_session ON recording_intervals(session_id);

            CREATE TABLE IF NOT EXISTS recording_session_events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  INTEGER NOT NULL REFERENCES recording_sessions(id) ON DELETE CASCADE,
                event_id    INTEGER NOT NULL REFERENCES usage_events(id) ON DELETE CASCADE,
                UNIQUE(session_id, event_id)
            );

            CREATE INDEX IF NOT EXISTS idx_session_events_session ON recording_session_events(session_id);
            CREATE INDEX IF NOT EXISTS idx_session_events_event ON recording_session_events(event_id);

            CREATE TABLE IF NOT EXISTS recording_session_pod_snapshots (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id          INTEGER NOT NULL REFERENCES recording_sessions(id) ON DELETE CASCADE,
                pod                 TEXT NOT NULL,
                provider            TEXT NOT NULL,
                deployment          TEXT NOT NULL,
                timestamp           TEXT NOT NULL,
                status              TEXT NOT NULL,
                uptime              TEXT,
                restarts            INTEGER DEFAULT 0,
                version             TEXT,
                last_active         TEXT,
                avg_response_ms     INTEGER DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_snapshots_session_pod ON recording_session_pod_snapshots(session_id, pod);
            CREATE INDEX IF NOT EXISTS idx_snapshots_timestamp ON recording_session_pod_snapshots(timestamp);

            CREATE INDEX IF NOT EXISTS idx_events_account_time ON usage_events(account_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_events_agent_time ON usage_events(agent, timestamp);
            CREATE INDEX IF NOT EXISTS idx_quota_account_time ON quota_snapshots(account_id, timestamp);"
        )?;
        Ok(())
    }

    pub fn insert_usage_event(&self, record: &UsageRecord, account_id: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        // Deduplicate: skip if same agent + timestamp already exists
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM usage_events WHERE agent = ?1 AND timestamp = ?2)",
            params![record.agent, record.timestamp.to_rfc3339()],
            |row| row.get(0),
        ).unwrap_or(false);
        if exists {
            return Ok(0);
        }
        conn.execute(
            "INSERT INTO usage_events (account_id, agent, session_id, timestamp, provider, model,
             input_tokens, output_tokens, total_tokens, credits_consumed, duration_ms,
             context_usage_pct, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                account_id,
                record.agent,
                record.session_id,
                record.timestamp.to_rfc3339(),
                record.provider,
                record.model,
                record.input_tokens,
                record.output_tokens,
                record.total_tokens,
                record.credits_consumed,
                record.duration_ms,
                record.context_usage_pct,
                record.metadata.as_ref().map(|m| m.to_string()),
            ],
        )?;
        let event_id = conn.last_insert_rowid();

        // Associate with an active recording session if the timestamp falls within an open interval
        let active: Option<(i64,)> = conn.query_row(
            "SELECT id FROM recording_sessions WHERE status = 'active' LIMIT 1",
            [],
            |row| Ok((row.get(0)?,)),
        ).ok();
        if let Some((session_id,)) = active {
            let in_interval: bool = conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM recording_intervals
                    WHERE session_id = ?1
                      AND started_at <= ?2
                      AND (ended_at IS NULL OR ended_at >= ?2)
                )",
                params![session_id, record.timestamp.to_rfc3339()],
                |row| row.get(0),
            ).unwrap_or(false);
            if in_interval {
                conn.execute(
                    "INSERT OR IGNORE INTO recording_session_events (session_id, event_id) VALUES (?1, ?2)",
                    params![session_id, event_id],
                )?;
            }
        }

        Ok(event_id)
    }

    pub fn insert_quota_snapshot(&self, snapshot: &QuotaSnapshot) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT OR REPLACE INTO quota_snapshots (account_id, timestamp, credits_used, credits_total, reset_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.account_id,
                snapshot.timestamp.to_rfc3339(),
                snapshot.credits_used,
                snapshot.credits_total,
                snapshot.reset_at.map(|t| t.to_rfc3339()),
                snapshot.metadata.as_ref().map(|m| m.to_string()),
            ],
        )?;
        Ok(())
    }

    pub fn log_collection(&self, provider: &str, agent: Option<&str>, status: &str, message: Option<&str>, count: i64) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO collection_log (provider, agent, status, message, records_count) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![provider, agent, status, message, count],
        )?;
        Ok(())
    }

    pub fn get_daily_summary(&self, days: i64) -> Result<Vec<DailySummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT date(timestamp) as d, account_id, COALESCE(agent,'unknown'),
                    COALESCE(provider,'unknown'), 0.0,
                    COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(total_tokens), 0), COUNT(*),
                    COALESCE(AVG(duration_ms), 0)
             FROM usage_events
             WHERE timestamp >= datetime('now', ?1)
             GROUP BY d, account_id, agent, provider
             ORDER BY d ASC, agent ASC"
        )?;

        let days_param = format!("-{} days", days);
        let rows = stmt.query_map(params![days_param], |row| {
            Ok(DailySummary {
                date: row.get(0)?,
                account_id: row.get(1)?,
                agent: row.get(2)?,
                provider: row.get(3)?,
                total_credits: row.get::<_, f64>(4).unwrap_or(0.0),
                total_input_tokens: row.get::<_, i64>(5).unwrap_or(0),
                total_output_tokens: row.get::<_, i64>(6).unwrap_or(0),
                total_tokens: row.get::<_, i64>(7).unwrap_or(0),
                request_count: row.get::<_, i64>(8).unwrap_or(0),
                avg_duration_ms: row.get::<_, i64>(9).unwrap_or(0),
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_daily_summary_for_session(&self, session_id: i64, days: i64) -> Result<Vec<DailySummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT date(ue.timestamp) as d, ue.account_id, COALESCE(ue.agent,'unknown'),
                    COALESCE(ue.provider,'unknown'), 0.0,
                    COALESCE(SUM(ue.input_tokens), 0), COALESCE(SUM(ue.output_tokens), 0),
                    COALESCE(SUM(ue.total_tokens), 0), COUNT(*),
                    COALESCE(AVG(ue.duration_ms), 0)
             FROM usage_events ue
             JOIN recording_session_events se ON ue.id = se.event_id
             WHERE se.session_id = ?1
               AND ue.timestamp >= datetime('now', ?2)
             GROUP BY d, ue.account_id, ue.agent, ue.provider
             ORDER BY d ASC, ue.agent ASC"
        )?;

        let days_param = format!("-{} days", days);
        let rows = stmt.query_map(params![session_id, days_param], |row| {
            Ok(DailySummary {
                date: row.get(0)?,
                account_id: row.get(1)?,
                agent: row.get(2)?,
                provider: row.get(3)?,
                total_credits: row.get::<_, f64>(4).unwrap_or(0.0),
                total_input_tokens: row.get::<_, i64>(5).unwrap_or(0),
                total_output_tokens: row.get::<_, i64>(6).unwrap_or(0),
                total_tokens: row.get::<_, i64>(7).unwrap_or(0),
                request_count: row.get::<_, i64>(8).unwrap_or(0),
                avg_duration_ms: row.get::<_, i64>(9).unwrap_or(0),
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_leaderboard_for_session(&self, session_id: i64) -> Result<Vec<(String, String, f64, i64, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT ue.agent, ue.provider,
                    COALESCE(SUM(ue.credits_consumed), 0),
                    COALESCE(SUM(ue.total_tokens), 0),
                    COUNT(*)
             FROM usage_events ue
             JOIN recording_session_events se ON ue.id = se.event_id
             WHERE se.session_id = ?1
             GROUP BY ue.agent, ue.provider"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, f64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_avg_duration_for_session(&self, session_id: i64, days: i64) -> Result<Vec<(String, String, f64)>> {
        let conn = self.conn.lock().unwrap();
        let days_param = format!("-{} days", days);
        let mut stmt = conn.prepare(
            "SELECT ue.agent, date(ue.timestamp) as d, AVG(ue.duration_ms)
             FROM usage_events ue
             JOIN recording_session_events se ON ue.id = se.event_id
             WHERE se.session_id = ?1
               AND ue.timestamp >= datetime('now', ?2)
               AND ue.duration_ms IS NOT NULL
             GROUP BY ue.agent, d
             ORDER BY d ASC"
        )?;
        let rows = stmt.query_map(params![session_id, days_param], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, f64>(2).unwrap_or(0.0)))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_daily_summary_for_session_unbounded(&self, session_id: i64) -> Result<Vec<DailySummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT date(ue.timestamp) as d, ue.account_id, COALESCE(ue.agent,'unknown'),
                    COALESCE(ue.provider,'unknown'), 0.0,
                    COALESCE(SUM(ue.input_tokens), 0), COALESCE(SUM(ue.output_tokens), 0),
                    COALESCE(SUM(ue.total_tokens), 0), COUNT(*),
                    COALESCE(AVG(ue.duration_ms), 0)
             FROM usage_events ue
             JOIN recording_session_events se ON ue.id = se.event_id
             WHERE se.session_id = ?1
             GROUP BY d, ue.account_id, ue.agent, ue.provider
             ORDER BY d ASC, ue.agent ASC"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(DailySummary {
                date: row.get(0)?,
                account_id: row.get(1)?,
                agent: row.get(2)?,
                provider: row.get(3)?,
                total_credits: row.get::<_, f64>(4).unwrap_or(0.0),
                total_input_tokens: row.get::<_, i64>(5).unwrap_or(0),
                total_output_tokens: row.get::<_, i64>(6).unwrap_or(0),
                total_tokens: row.get::<_, i64>(7).unwrap_or(0),
                request_count: row.get::<_, i64>(8).unwrap_or(0),
                avg_duration_ms: row.get::<_, i64>(9).unwrap_or(0),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_avg_duration_for_session_unbounded(&self, session_id: i64) -> Result<Vec<(String, String, f64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT ue.agent, date(ue.timestamp) as d, AVG(ue.duration_ms)
             FROM usage_events ue
             JOIN recording_session_events se ON ue.id = se.event_id
             WHERE se.session_id = ?1
               AND ue.duration_ms IS NOT NULL
             GROUP BY ue.agent, d
             ORDER BY d ASC"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, f64>(2).unwrap_or(0.0)))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_latest_quota(&self) -> Result<Vec<(String, Option<f64>, Option<f64>, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT account_id, credits_used, credits_total, reset_at
             FROM quota_snapshots
             WHERE id IN (SELECT MAX(id) FROM quota_snapshots GROUP BY account_id)"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn update_daily_summary(&self) -> Result<()> {
        self.conn.lock().unwrap().execute_batch(
            "INSERT OR REPLACE INTO daily_summary (date, account_id, agent, provider,
                total_credits, total_input_tokens, total_output_tokens, total_tokens,
                request_count, avg_duration_ms)
            SELECT
                date(timestamp) as date,
                account_id, agent, provider,
                COALESCE(SUM(credits_consumed), 0),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COUNT(*),
                COALESCE(AVG(duration_ms), 0)
            FROM usage_events
            WHERE date(timestamp) >= date('now', '-1 day')
            GROUP BY date(timestamp), account_id, agent, provider;"
        )?;
        Ok(())
    }

    pub fn get_last_collection(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT timestamp FROM collection_log WHERE status='success' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        );
        match result {
            Ok(ts) => Ok(Some(ts)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_provider_health(&self) -> Result<Vec<(String, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT provider, status, timestamp FROM collection_log
             WHERE id IN (SELECT MAX(id) FROM collection_log GROUP BY provider)"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_avg_duration_by_agent(&self, days: i64) -> Result<Vec<(String, String, f64)>> {
        let conn = self.conn.lock().unwrap();
        let days_param = format!("-{} days", days);
        let mut stmt = conn.prepare(
            "SELECT agent, date(timestamp) as d, AVG(duration_ms)
             FROM usage_events
             WHERE timestamp >= datetime('now', ?1) AND duration_ms IS NOT NULL
             GROUP BY agent, d
             ORDER BY d ASC"
        )?;
        let rows = stmt.query_map(params![days_param], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, f64>(2).unwrap_or(0.0)))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_last_active(&self, agent: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT timestamp FROM usage_events WHERE agent = ?1 ORDER BY id DESC LIMIT 1",
            params![agent],
            |row| row.get(0),
        );
        match result {
            Ok(ts) => Ok(Some(ts)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_avg_duration_for_agent(&self, agent: &str, days: i64) -> Result<f64> {
        let conn = self.conn.lock().unwrap();
        let days_param = format!("-{} days", days);
        let result = conn.query_row(
            "SELECT AVG(duration_ms) FROM usage_events
             WHERE agent = ?1 AND timestamp >= datetime('now', ?2) AND duration_ms IS NOT NULL",
            params![agent, days_param],
            |row| row.get::<_, f64>(0),
        );
        match result {
            Ok(v) => Ok(v),
            Err(_) => Ok(0.0),
        }
    }

    pub fn create_recording_session(&self, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() || name.len() > 80 {
            bail!("session name must be non-empty and at most 80 characters");
        }
        if self.get_open_recording_session()?.is_some() {
            bail!("an open recording session already exists");
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO recording_sessions (name, status, created_at, updated_at)
             VALUES (?1, 'active', ?2, ?2)",
            params![name, now],
        )?;
        let session_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO recording_intervals (session_id, started_at) VALUES (?1, ?2)",
            params![session_id, now],
        )?;
        tx.commit()?;
        Ok(session_id)
    }

    pub fn get_recording_session_by_id(&self, id: i64) -> Result<Option<RecordingSession>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, name, status, created_at, updated_at FROM recording_sessions WHERE id = ?1",
            params![id],
            |row| self::session_from_row(row),
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_open_recording_session(&self) -> Result<Option<RecordingSession>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, name, status, created_at, updated_at FROM recording_sessions
             WHERE status IN ('active','paused') LIMIT 1",
            [],
            |row| self::session_from_row(row),
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_active_recording_session(&self) -> Result<Option<RecordingSession>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, name, status, created_at, updated_at FROM recording_sessions
             WHERE status = 'active' LIMIT 1",
            [],
            |row| self::session_from_row(row),
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_recording_sessions(&self) -> Result<Vec<RecordingSession>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, status, created_at, updated_at FROM recording_sessions
             ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| self::session_from_row(row))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn pause_recording_session(&self, id: i64) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE recording_sessions
             SET status = 'paused', updated_at = ?1
             WHERE id = ?2 AND status = 'active'",
            params![now, id],
        )?;
        if changed == 0 {
            bail!("only an active session can be paused");
        }
        tx.execute(
            "UPDATE recording_intervals
             SET ended_at = ?1
             WHERE session_id = ?2 AND ended_at IS NULL",
            params![now, id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn resume_recording_session(&self, id: i64) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE recording_sessions
             SET status = 'active', updated_at = ?1
             WHERE id = ?2 AND status = 'paused'",
            params![now, id],
        )?;
        if changed == 0 {
            bail!("only a paused session can be resumed");
        }
        tx.execute(
            "INSERT INTO recording_intervals (session_id, started_at) VALUES (?1, ?2)",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn archive_recording_session(&self, id: i64) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE recording_sessions
             SET status = 'archived', updated_at = ?1
             WHERE id = ?2 AND status IN ('active','paused')",
            params![now, id],
        )?;
        if changed == 0 {
            bail!("only an active or paused session can be archived");
        }
        tx.execute(
            "UPDATE recording_intervals
             SET ended_at = ?1
             WHERE session_id = ?2 AND ended_at IS NULL",
            params![now, id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_recording_session(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "DELETE FROM recording_sessions WHERE id = ?1 AND status = 'archived'",
            params![id],
        )?;
        if changed == 0 {
            bail!("only an archived session can be deleted");
        }
        Ok(())
    }

    pub fn get_recording_intervals_for_session(&self, session_id: i64) -> Result<Vec<RecordingInterval>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, started_at, ended_at FROM recording_intervals
             WHERE session_id = ?1 ORDER BY started_at ASC"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(RecordingInterval {
                id: row.get(0)?,
                session_id: row.get(1)?,
                started_at: parse_datetime(2, &row.get::<_, String>(2)?)?,
                ended_at: match row.get::<_, Option<String>>(3)? {
                    Some(s) => Some(parse_datetime(3, &s)?),
                    None => None,
                },
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn is_timestamp_in_open_session(&self, session_id: i64, ts: &DateTime<Utc>) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM recording_intervals
                WHERE session_id = ?1
                  AND started_at <= ?2
                  AND (ended_at IS NULL OR ended_at >= ?2)
            )",
            params![session_id, ts.to_rfc3339()],
            |row| row.get(0),
        )?;
        Ok(result)
    }

    pub fn get_session_events(&self, session_id: i64) -> Result<Vec<RecordingSessionEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, event_id FROM recording_session_events
             WHERE session_id = ?1 ORDER BY id ASC"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(RecordingSessionEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                event_id: row.get(2)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn insert_pod_snapshot(&self, snapshot: &SessionPodSnapshot) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO recording_session_pod_snapshots
             (session_id, pod, provider, deployment, timestamp, status, uptime, restarts, version, last_active, avg_response_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                snapshot.session_id,
                snapshot.pod,
                snapshot.provider,
                snapshot.deployment,
                snapshot.timestamp.to_rfc3339(),
                snapshot.status,
                snapshot.uptime,
                snapshot.restarts,
                snapshot.version,
                snapshot.last_active,
                snapshot.avg_response_ms,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_pod_snapshots_for_session(&self, session_id: i64) -> Result<Vec<SessionPodSnapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, pod, provider, deployment, timestamp, status, uptime, restarts, version, last_active, avg_response_ms
             FROM recording_session_pod_snapshots
             WHERE id IN (
                 SELECT MAX(id) FROM recording_session_pod_snapshots
                 WHERE session_id = ?1 GROUP BY pod, provider
             )"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionPodSnapshot {
                id: row.get(0)?,
                session_id: row.get(1)?,
                pod: row.get(2)?,
                provider: row.get(3)?,
                deployment: row.get(4)?,
                timestamp: parse_datetime(5, &row.get::<_, String>(5)?)?,
                status: row.get(6)?,
                uptime: row.get::<_, Option<String>>(7)?,
                restarts: row.get::<_, i32>(8).unwrap_or(0),
                version: row.get::<_, Option<String>>(9)?,
                last_active: row.get::<_, Option<String>>(10)?,
                avg_response_ms: row.get::<_, i64>(11).unwrap_or(0),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn db_size_mb(&self) -> f64 {
        let page_count: i64 = self.conn.lock().unwrap().query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap_or(0);
        let page_size: i64 = self.conn.lock().unwrap().query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap_or(4096);
        (page_count * page_size) as f64 / 1_048_576.0
    }
}

fn parse_datetime(idx: usize, s: &str) -> rusqlite::Result<DateTime<Utc>> {
    s.parse().map_err(|e| rusqlite::Error::FromSqlConversionFailure(
        idx, rusqlite::types::Type::Text, Box::new(e),
    ))
}

fn session_from_row(row: &rusqlite::Row) -> rusqlite::Result<RecordingSession> {
    let status_str: String = row.get(2)?;
    let status = RecordingSessionStatus::from_str(&status_str)
        .ok_or_else(|| rusqlite::Error::InvalidColumnType(2, "status".into(), rusqlite::types::Type::Text))?;
    Ok(RecordingSession {
        id: row.get(0)?,
        name: row.get(1)?,
        status,
        created_at: parse_datetime(3, &row.get::<_, String>(3)?)?,
        updated_at: parse_datetime(4, &row.get::<_, String>(4)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_mem_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        db.initialize().unwrap();
        db
    }

    fn sample_event(ts: DateTime<Utc>, agent: &str) -> UsageRecord {
        UsageRecord {
            timestamp: ts,
            agent: agent.into(),
            provider: "kiro".into(),
            ..Default::default()
        }
    }

    #[test]
    fn lifecycle_and_interval_boundaries() {
        let db = in_mem_db();
        let id = db.create_recording_session("  test  ").unwrap();
        let session = db.get_open_recording_session().unwrap().unwrap();
        assert_eq!(session.id, id);
        assert_eq!(session.status, RecordingSessionStatus::Active);
        assert_eq!(session.name, "test");

        let intervals = db.get_recording_intervals_for_session(id).unwrap();
        assert_eq!(intervals.len(), 1);
        assert!(intervals[0].ended_at.is_none());

        db.pause_recording_session(id).unwrap();
        let session = db.get_recording_session_by_id(id).unwrap().unwrap();
        assert_eq!(session.status, RecordingSessionStatus::Paused);

        let intervals = db.get_recording_intervals_for_session(id).unwrap();
        assert_eq!(intervals.len(), 1);
        assert!(intervals[0].ended_at.is_some());

        std::thread::sleep(std::time::Duration::from_millis(10));
        db.resume_recording_session(id).unwrap();
        let session = db.get_recording_session_by_id(id).unwrap().unwrap();
        assert_eq!(session.status, RecordingSessionStatus::Active);

        let intervals = db.get_recording_intervals_for_session(id).unwrap();
        assert_eq!(intervals.len(), 2);
        assert!(intervals[1].ended_at.is_none());

        db.archive_recording_session(id).unwrap();
        let session = db.get_recording_session_by_id(id).unwrap().unwrap();
        assert_eq!(session.status, RecordingSessionStatus::Archived);
        let intervals = db.get_recording_intervals_for_session(id).unwrap();
        assert!(intervals.iter().all(|i| i.ended_at.is_some()));
    }

    #[test]
    fn one_open_session_enforcement() {
        let db = in_mem_db();
        let id = db.create_recording_session("first").unwrap();

        assert!(db.create_recording_session("second").is_err());

        db.pause_recording_session(id).unwrap();
        assert!(db.create_recording_session("third").is_err());

        db.archive_recording_session(id).unwrap();
        let _ = db.create_recording_session("fourth").unwrap();
    }

    #[test]
    fn event_association_without_baseline_loss() {
        let db = in_mem_db();
        let session_id = db.create_recording_session("recording").unwrap();

        let now = Utc::now();
        let in_interval = sample_event(now, "masami");
        db.insert_usage_event(&in_interval, "kiro:user").unwrap();

        let mut before = sample_event(now - chrono::Duration::hours(1), "chloe");
        before.agent = "chloe".into();
        db.insert_usage_event(&before, "kiro:user").unwrap();

        // baseline contains both events
        let conn = db.conn.lock().unwrap();
        let baseline_count: i64 = conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0)).unwrap();
        assert_eq!(baseline_count, 2);
        drop(conn);

        // only the in-interval event is associated
        let associated = db.get_session_events(session_id).unwrap();
        assert_eq!(associated.len(), 1);

        // pause and then insert another event - it should not associate
        db.pause_recording_session(session_id).unwrap();
        let mut after = sample_event(now + chrono::Duration::minutes(1), "misaki");
        after.agent = "misaki".into();
        db.insert_usage_event(&after, "kiro:user").unwrap();

        let associated = db.get_session_events(session_id).unwrap();
        assert_eq!(associated.len(), 1);
    }

    #[test]
    fn session_deletion_preserves_baseline_events() {
        let db = in_mem_db();
        let session_id = db.create_recording_session("recording").unwrap();

        let now = Utc::now();
        db.insert_usage_event(&sample_event(now, "masami"), "kiro:user").unwrap();

        let events = db.get_session_events(session_id).unwrap();
        assert_eq!(events.len(), 1);

        db.archive_recording_session(session_id).unwrap();
        db.delete_recording_session(session_id).unwrap();

        assert!(db.get_recording_session_by_id(session_id).unwrap().is_none());

        let conn = db.conn.lock().unwrap();
        let baseline_count: i64 = conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0)).unwrap();
        assert_eq!(baseline_count, 1);
    }

    #[test]
    fn frozen_archived_pod_snapshot_selection() {
        let db = in_mem_db();
        let session_id = db.create_recording_session("recording").unwrap();

        let snapshot = SessionPodSnapshot {
            id: 0,
            session_id,
            pod: "masami".into(),
            provider: "kiro".into(),
            deployment: "masami-deployment".into(),
            timestamp: Utc::now(),
            status: "enabled".into(),
            uptime: Some("2h".into()),
            restarts: 1,
            version: Some("v1.2.3".into()),
            last_active: Some("2026-08-23T12:00:00Z".into()),
            avg_response_ms: 120,
        };
        db.insert_pod_snapshot(&snapshot).unwrap();

        let snapshots = db.get_pod_snapshots_for_session(session_id).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].pod, "masami");
        assert_eq!(snapshots[0].uptime, Some("2h".to_string()));

        db.archive_recording_session(session_id).unwrap();
        let snapshots = db.get_pod_snapshots_for_session(session_id).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].uptime, Some("2h".to_string()));
    }

    #[test]
    fn pod_snapshots_grouped_by_pod_and_provider() {
        let db = in_mem_db();
        let session_id = db.create_recording_session("recording").unwrap();

        let older = Utc::now() - chrono::Duration::hours(1);
        let newer = Utc::now();

        let s1 = SessionPodSnapshot {
            id: 0,
            session_id,
            pod: "masami".into(),
            provider: "kiro".into(),
            deployment: "kiro-deployment".into(),
            timestamp: older,
            status: "enabled".into(),
            uptime: Some("1h".into()),
            restarts: 0,
            version: None,
            last_active: None,
            avg_response_ms: 100,
        };
        let s2 = SessionPodSnapshot {
            id: 0,
            session_id,
            pod: "masami".into(),
            provider: "chloe".into(),
            deployment: "chloe-deployment".into(),
            timestamp: older,
            status: "enabled".into(),
            uptime: Some("30m".into()),
            restarts: 0,
            version: None,
            last_active: None,
            avg_response_ms: 200,
        };
        let s3 = SessionPodSnapshot {
            id: 0,
            session_id,
            pod: "masami".into(),
            provider: "kiro".into(),
            deployment: "kiro-deployment".into(),
            timestamp: newer,
            status: "enabled".into(),
            uptime: Some("2h".into()),
            restarts: 0,
            version: None,
            last_active: None,
            avg_response_ms: 110,
        };

        db.insert_pod_snapshot(&s1).unwrap();
        db.insert_pod_snapshot(&s2).unwrap();
        db.insert_pod_snapshot(&s3).unwrap();

        let snapshots = db.get_pod_snapshots_for_session(session_id).unwrap();
        // same pod, two providers -> two rows, each returning the newest id
        assert_eq!(snapshots.len(), 2);

        let kiro = snapshots.iter().find(|s| s.provider == "kiro").unwrap();
        let chloe = snapshots.iter().find(|s| s.provider == "chloe").unwrap();
        assert_eq!(kiro.uptime, Some("2h".to_string()));
        assert_eq!(chloe.uptime, Some("30m".to_string()));
    }

    #[test]
    fn invalid_transitions_and_deletions_rejected() {
        let db = in_mem_db();
        let id = db.create_recording_session("test").unwrap();

        assert!(db.resume_recording_session(id).is_err());
        assert!(db.archive_recording_session(9999).is_err());
        assert!(db.delete_recording_session(id).is_err());

        db.archive_recording_session(id).unwrap();
        assert!(db.delete_recording_session(id).is_ok());
    }

    #[test]
    fn session_name_validation() {
        let db = in_mem_db();

        assert!(db.create_recording_session("").is_err());
        assert!(db.create_recording_session("   ").is_err());
        assert!(db.create_recording_session("a".repeat(81).as_str()).is_err());

        let _ = db.create_recording_session("unique").unwrap();
        assert!(db.create_recording_session("unique").is_err());
    }

    #[test]
    fn baseline_and_session_summary_differ() {
        let db = in_mem_db();

        // Baseline event before any session
        let before = sample_event(Utc::now() - chrono::Duration::hours(1), "pre");
        db.insert_usage_event(&before, "kiro:user").unwrap();

        let session_id = db.create_recording_session("recording").unwrap();
        let now = Utc::now();
        let in_session = sample_event(now, "in");
        db.insert_usage_event(&in_session, "kiro:user").unwrap();

        let baseline = db.get_daily_summary(1).unwrap();
        let session = db.get_daily_summary_for_session(session_id, 1).unwrap();

        // baseline contains both events; session read contains only the associated one
        assert_eq!(baseline.iter().map(|s| s.request_count).sum::<i64>(), 2);
        assert_eq!(session.iter().map(|s| s.request_count).sum::<i64>(), 1);
    }

    #[test]
    fn session_leaderboard_and_response_time_filter() {
        let db = in_mem_db();
        let session_id = db.create_recording_session("recording").unwrap();

        let now = Utc::now();
        let mut e1 = sample_event(now, "masami");
        e1.duration_ms = Some(100);
        let mut e2 = sample_event(now, "chloe");
        e2.duration_ms = Some(200);
        db.insert_usage_event(&e1, "kiro:user").unwrap();
        db.insert_usage_event(&e2, "kiro:user").unwrap();

        let leaderboard = db.get_leaderboard_for_session(session_id).unwrap();
        assert_eq!(leaderboard.len(), 2);
        assert_eq!(leaderboard.iter().map(|(_, _, _, _, r)| r).sum::<i64>(), 2);

        let avg = db.get_avg_duration_for_session(session_id, 1).unwrap();
        assert_eq!(avg.len(), 2);
        assert!(avg.iter().any(|(a, _, _)| a == "masami"));
    }

    #[test]
    fn unbounded_session_summary_and_response_time() {
        let db = in_mem_db();
        let session_id = db.create_recording_session("recording").unwrap();

        let mut e = sample_event(Utc::now(), "masami");
        e.duration_ms = Some(120);
        db.insert_usage_event(&e, "kiro:user").unwrap();

        // unbounded read returns the same in-session data as a bounded 1-day read
        let unbounded = db.get_daily_summary_for_session_unbounded(session_id).unwrap();
        let bounded = db.get_daily_summary_for_session(session_id, 1).unwrap();
        assert_eq!(unbounded.iter().map(|s| s.request_count).sum::<i64>(), 1);
        assert_eq!(bounded.iter().map(|s| s.request_count).sum::<i64>(), unbounded.iter().map(|s| s.request_count).sum::<i64>());

        let avg_unbounded = db.get_avg_duration_for_session_unbounded(session_id).unwrap();
        let avg_bounded = db.get_avg_duration_for_session(session_id, 1).unwrap();
        assert_eq!(avg_unbounded.len(), 1);
        assert_eq!(avg_bounded.len(), avg_unbounded.len());
    }
}
