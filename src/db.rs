use anyhow::Result;
use rusqlite::{Connection, params};
use std::sync::Mutex;
use crate::models::{UsageRecord, QuotaSnapshot, DailySummary};

pub struct Database {
    conn: Mutex<Connection>,
}

unsafe impl Send for Database {}
unsafe impl Sync for Database {}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA busy_timeout=5000;")?;
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

            CREATE INDEX IF NOT EXISTS idx_events_account_time ON usage_events(account_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_events_agent_time ON usage_events(agent, timestamp);
            CREATE INDEX IF NOT EXISTS idx_quota_account_time ON quota_snapshots(account_id, timestamp);"
        )?;
        Ok(())
    }

    pub fn insert_usage_event(&self, record: &UsageRecord, account_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // Deduplicate: skip if same agent + timestamp already exists
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM usage_events WHERE agent = ?1 AND timestamp = ?2)",
            params![record.agent, record.timestamp.to_rfc3339()],
            |row| row.get(0),
        ).unwrap_or(false);
        if exists {
            return Ok(());
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
        Ok(())
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

    pub fn db_size_mb(&self) -> f64 {
        let page_count: i64 = self.conn.lock().unwrap().query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap_or(0);
        let page_size: i64 = self.conn.lock().unwrap().query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap_or(4096);
        (page_count * page_size) as f64 / 1_048_576.0
    }
}
