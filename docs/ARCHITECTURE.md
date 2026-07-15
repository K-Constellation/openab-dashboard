# OpenAB Dashboard — Architecture

## Overview

A single-binary monitoring dashboard built in Rust. Collects AI tool usage data from OpenAB pods and presents it in a web interface.

**Key properties:**
- Single static binary (~8 MB), no runtime dependencies
- Embedded web assets (HTML/JS/CSS baked into binary)
- SQLite database (single file)
- ~5-10 MB memory footprint
- Deploy = `scp` one file

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         your-server (K8s host)                           │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │  openab-dashboard (single binary)                             │  │
│  │                                                               │  │
│  │  ┌─────────────┐    ┌──────────┐    ┌──────────────────────┐ │  │
│  │  │  Collector   │───▶│ SQLite   │◀───│  Web Server (axum)   │ │  │
│  │  │  (5min tick) │    │ usage.db │    │  :8080               │ │  │
│  │  └─────────────┘    └──────────┘    └──────────────────────┘ │  │
│  │        │                                      │               │  │
│  │        │ kubectl exec                         │ HTTP           │  │
│  │        ▼                                      ▼               │  │
│  └───────────────────────────────────────────────────────────────┘  │
│        │                                         │                  │
│        ▼                                         ▼                  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐   Browser (dashboard)      │
│  │ Masami   │ │ Chloe    │ │ Misaki   │                            │
│  │ pod      │ │ pod      │ │ pod      │                            │
│  └──────────┘ └──────────┘ └──────────┘                            │
└─────────────────────────────────────────────────────────────────────┘
```

## Tech Stack

| Component | Technology | Reason |
|-----------|-----------|--------|
| Language | Rust | Single binary, minimal memory, fast |
| Web framework | axum | Lightweight async HTTP server |
| Database | rusqlite | Embedded SQLite, zero infra |
| HTTP client | reqwest | Call external APIs (Google quota) |
| Scheduler | tokio-cron-scheduler | In-process periodic tasks |
| Serialization | serde + serde_json + serde_yaml | Config + API responses |
| CLI | clap | Command-line arguments |
| Frontend | HTMX + Chart.js + Tailwind CSS (CDN) | No build step, interactive |
| Asset embedding | rust-embed | Bake static files into binary |

## Components

### 1. Collector (`src/collector/`)

Runs every 5 minutes inside the same binary process:
- Executes `kubectl exec` / `kubectl logs` to query pods
- Calls Google quota API using OAuth token from pod
- Writes normalized records to SQLite

**Design pattern:** Provider trait — each AI tool implements `collect()`.

### 2. SQLite Database (`usage.db`)

Single-file database:
- Account info
- Quota snapshots (periodic credit balance)
- Usage events (per-turn token counts)
- Daily summaries (pre-aggregated)

### 3. Web Server (`src/server.rs`)

axum HTTP server:
- `GET /` — serves dashboard HTML (embedded in binary)
- `GET /api/*` — JSON API for frontend
- Static assets (JS, CSS) embedded via `rust-embed`

### 4. Frontend (`static/`)

Single-page dashboard:
- **HTMX** — dynamic updates without full SPA (auto-refresh, filter controls)
- **Chart.js** — bar charts, progress bars (via CDN)
- **Tailwind CSS** — dark theme styling (via CDN)
- No build step, no npm, no webpack

## Frontend Design

```
┌────────────────────────────────────────────────────────────────┐
│  ⏱ Time Range: [This Month ▾] [Refresh]        Last: 10:05 AM │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│  ┌─────────┐  ┌─────────┐  ┌─────────────┐  ┌─────────────┐  │
│  │ Credits │  │ Today   │  │ Top Agent   │  │ Days Left   │  │
│  │ 342.5   │  │  45.2   │  │ masami      │  │    18       │  │
│  └─────────┘  └─────────┘  └─────────────┘  └─────────────┘  │
│                                                                │
│  📊 Daily Usage (stacked bar chart by agent)                   │
│  ████████████████████████████████████████████████████████████  │
│                                                                │
│  🏆 Agent Leaderboard                                          │
│  1. masami ████████████████████████░░░░░ 65%  ▶520  🔧245     │
│  2. chloe  ████████████████░░░░░░░░░░░░ 41%  ▶410  🔧198     │
│  3. misaki ██████████░░░░░░░░░░░░░░░░░░ 28%  ▶280  🔧89      │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

## Data Flow

```
Pod (logs / SQLite DB / OAuth token)
    │
    │  kubectl exec/logs (subprocess)
    ▼
Collector (parse + normalize)
    │
    │  rusqlite INSERT
    ▼
SQLite (usage.db)
    │
    │  rusqlite SELECT
    ▼
axum handler → JSON response
    │
    │  fetch('/api/...')
    ▼
HTMX + Chart.js renders in browser
```

## Deployment

### Development (local)
```bash
cargo run -- --config config.yaml
```

### Production (your-server)
```bash
# Cross-compile (if building on different machine)
cargo build --release

# Deploy
scp target/release/openab-dashboard your-server:~/openab-dashboard/
scp config.yaml your-server:~/openab-dashboard/

# Run
ssh your-server "~/openab-dashboard/openab-dashboard --config config.yaml"
```

### As launchd service
```xml
<!-- ~/Library/LaunchAgents/com.openab.dashboard.plist -->
<plist version="1.0">
<dict>
    <key>Label</key><string>com.openab.dashboard</string>
    <key>ProgramArguments</key>
    <array>
        <string>/path/to/openab-dashboard/openab-dashboard</string>
        <string>--config</string>
        <string>/path/to/openab-dashboard/config.yaml</string>
    </array>
    <key>KeepAlive</key><true/>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
```

### Future: K8s Pod
```bash
# Build container (multi-stage, final image ~15 MB)
docker build -t openab-dashboard .
```
