# OpenAB Dashboard — Database Schema

## Overview

Single SQLite database (`usage.db`) storing all usage metrics. Designed for:
- Fast reads for dashboard queries
- Extensible to new providers via JSON `metadata` fields
- Pre-aggregated daily summaries for quick chart rendering

## Schema

### `accounts`

Registered AI tool accounts being monitored.

```sql
CREATE TABLE accounts (
    id          TEXT PRIMARY KEY,    -- format: "{provider}:{identifier}"
                                    -- e.g. "kiro:user", "agy:user@example.com"
    provider    TEXT NOT NULL,       -- "kiro", "antigravity", "openai", "claude", ...
    plan        TEXT,                -- "free", "pro", "pro_plus", "enterprise"
    email       TEXT,
    display_name TEXT,               -- friendly name for dashboard
    created_at  TEXT NOT NULL,       -- ISO 8601
    metadata    TEXT                 -- JSON: provider-specific info
);
```

**Examples:**
```
id: "kiro:user"
provider: "kiro"
plan: "pro"
email: "jack@example.com"
display_name: "Kiro Pro (user)"
metadata: {"region": "us-east-1", "license": "pro"}
```

### `quota_snapshots`

Periodic snapshots of account-level quota/credit balance.

```sql
CREATE TABLE quota_snapshots (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      TEXT NOT NULL REFERENCES accounts(id),
    timestamp       TEXT NOT NULL,       -- ISO 8601 (collection time)
    credits_used    REAL,                -- credits consumed this period
    credits_total   REAL,                -- total credits for this period
    reset_at        TEXT,                -- when credits reset (ISO 8601)
    metadata        TEXT,                -- JSON: per-model breakdown, etc.
    UNIQUE(account_id, timestamp)
);
```

**Index:**
```sql
CREATE INDEX idx_quota_account_time ON quota_snapshots(account_id, timestamp);
```

### `usage_events`

Individual usage records (per turn / per request).

```sql
CREATE TABLE usage_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id          TEXT NOT NULL REFERENCES accounts(id),
    agent               TEXT,            -- "masami", "chloe", "misaki", "test"
    session_id          TEXT,
    timestamp           TEXT NOT NULL,   -- ISO 8601
    provider            TEXT NOT NULL,   -- "kiro", "antigravity"
    model               TEXT,            -- "claude-sonnet-4.6", "gemini-2.5-flash"
    input_tokens        INTEGER,
    output_tokens       INTEGER,
    total_tokens        INTEGER,         -- input + output (convenience)
    credits_consumed    REAL,
    duration_ms         INTEGER,         -- turn/request duration
    context_usage_pct   REAL,            -- context window usage percentage
    metadata            TEXT             -- JSON: tool_calls, error info, etc.
);
```

**Indexes:**
```sql
CREATE INDEX idx_events_account_time ON usage_events(account_id, timestamp);
CREATE INDEX idx_events_agent_time ON usage_events(agent, timestamp);
CREATE INDEX idx_events_provider_time ON usage_events(provider, timestamp);
```

### `daily_summary`

Pre-aggregated daily totals for fast dashboard rendering.

```sql
CREATE TABLE daily_summary (
    date                TEXT NOT NULL,       -- "2026-07-14"
    account_id          TEXT NOT NULL REFERENCES accounts(id),
    agent               TEXT NOT NULL,       -- "masami", "chloe", "misaki"
    provider            TEXT NOT NULL,
    total_credits       REAL DEFAULT 0,
    total_input_tokens  INTEGER DEFAULT 0,
    total_output_tokens INTEGER DEFAULT 0,
    total_tokens        INTEGER DEFAULT 0,
    request_count       INTEGER DEFAULT 0,
    avg_duration_ms     INTEGER DEFAULT 0,
    PRIMARY KEY(date, account_id, agent)
);
```

### `collection_log`

Tracks collector health — when each provider was last successfully polled.

```sql
CREATE TABLE collection_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider    TEXT NOT NULL,
    agent       TEXT,
    timestamp   TEXT NOT NULL,
    status      TEXT NOT NULL,       -- "success", "error", "timeout"
    message     TEXT,                -- error message if failed
    records_count INTEGER DEFAULT 0  -- how many records were collected
);
```

## Aggregation

Daily summary is updated after each collection run:

```sql
INSERT OR REPLACE INTO daily_summary (date, account_id, agent, provider,
    total_credits, total_input_tokens, total_output_tokens, total_tokens,
    request_count, avg_duration_ms)
SELECT
    date(timestamp) as date,
    account_id,
    agent,
    provider,
    COALESCE(SUM(credits_consumed), 0),
    COALESCE(SUM(input_tokens), 0),
    COALESCE(SUM(output_tokens), 0),
    COALESCE(SUM(total_tokens), 0),
    COUNT(*),
    COALESCE(AVG(duration_ms), 0)
FROM usage_events
WHERE date(timestamp) = date('now')
GROUP BY date(timestamp), account_id, agent, provider;
```

## Retention Policy

| Table | Retention | Reason |
|-------|-----------|--------|
| `usage_events` | 90 days | Detailed records, can get large |
| `quota_snapshots` | 180 days | Lightweight, useful for trends |
| `daily_summary` | Forever | Small, valuable for historical charts |
| `collection_log` | 30 days | Debugging only |

Cleanup runs daily:
```sql
DELETE FROM usage_events WHERE timestamp < datetime('now', '-90 days');
DELETE FROM quota_snapshots WHERE timestamp < datetime('now', '-180 days');
DELETE FROM collection_log WHERE timestamp < datetime('now', '-30 days');
```
