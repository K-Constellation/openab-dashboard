# OpenAB Dashboard

[繁體中文版 README](README.zh-TW.md)

A lightweight, single-binary monitoring dashboard for [OpenAB](https://github.com/openabdev/openab) agent pods. Track token usage, response times, and pod health across all your AI coding agents.

![Rust](https://img.shields.io/badge/rust-1.75%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Token-based metrics** — Track actual LLM token consumption per agent per day
- **Multi-provider support** — Kiro, Antigravity, OpenCode, or any CLI agent
- **Pod health monitoring** — Uptime, restarts, version, last active time
- **Response time tracking** — Average dispatch latency per agent over time
- **Leaderboard** — Ranked agent usage by tokens, with period filtering
- **5 built-in themes** — Cyberpunk Neon, Minimal Light, Glassmorphism, Terminal Green, Deep Ocean
- **Zero infrastructure** — SQLite database, embedded web assets, single binary
- **Auto-refresh** — Dashboard updates every 60 seconds

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  openab-dashboard (single binary, ~8 MB)                │
│                                                         │
│  ┌─────────────┐    ┌──────────┐    ┌───────────────┐  │
│  │  Collector   │───▶│ SQLite   │◀───│ Web Server    │  │
│  │  (5min tick) │    │ usage.db │    │ (axum :8080)  │  │
│  └─────────────┘    └──────────┘    └───────────────┘  │
│        │                                     │          │
│        │ kubectl logs                        │ HTTP     │
│        ▼                                     ▼          │
│  ┌──────────────────────┐              Browser          │
│  │ Pod A │ Pod B │ ...  │              (Dashboard)      │
│  └──────────────────────┘                               │
└─────────────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- Rust 1.75+ (for building)
- `kubectl` configured with access to your OpenAB pods

### Build & Run

```bash
# Clone
git clone https://github.com/masami-agent/openab-dashboard.git
cd openab-dashboard

# Build
cargo build --release

# Configure (edit config.yaml with your pods)
cp config.example.yaml config.yaml

# Run
./target/release/openab-dashboard --config config.yaml

# Open http://localhost:8080
```

### Run collector once (test mode)

```bash
./target/release/openab-dashboard --config config.yaml --once
```

## Configuration

```yaml
database:
  path: "./usage.db"

server:
  host: "0.0.0.0"
  port: 8080

collector:
  interval_seconds: 300        # Collect every 5 minutes
  kubectl_path: "/usr/local/bin/kubectl"

# Add any number of providers — just list their pods
providers:
  kiro:
    enabled: true
    pods:
      - name: masami
        deployment: openab-masami-kiro
        account_id: "kiro:user@example.com"

  antigravity:
    enabled: true
    pods:
      - name: misaki
        deployment: openab-misaki-gemini
        account_id: "agy:user@example.com"

  # Add more providers as needed:
  # opencode:
  #   enabled: true
  #   pods:
  #     - name: my-opencode
  #       deployment: openab-opencode
  #       account_id: "opencode:free"
```

### Adding a new provider

Just add a section under `providers:` in config.yaml. No code changes needed. The dashboard automatically collects `tokens_per_event` and `agent_dispatch_ms` from any pod's dispatch logs.

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /` | Dashboard UI |
| `GET /api/summary?days=14&provider=kiro` | Daily token usage (filterable) |
| `GET /api/leaderboard?period=week&metric=tokens` | Agent ranking |
| `GET /api/pods` | Pod status with uptime, version, last active |
| `GET /api/response-time?days=14` | Average response time per agent |
| `GET /api/quota` | Credit quota snapshots (when available) |
| `GET /api/health` | System health + last collection time |

## Deployment

### As a systemd/launchd service

```bash
# macOS (launchd)
cp com.openab.dashboard.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.openab.dashboard.plist

# Linux (systemd)
# Create a service file pointing to the binary
```

### Deploy = scp

```bash
scp target/release/openab-dashboard server:~/openab-dashboard/
scp config.yaml server:~/openab-dashboard/
ssh server "~/openab-dashboard/openab-dashboard --config ~/openab-dashboard/config.yaml"
```

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust |
| Web framework | axum 0.7 |
| Database | SQLite (rusqlite, embedded) |
| Charts | Chart.js 4.x (CDN) |
| Styling | Tailwind CSS (CDN) |
| Interactivity | HTMX 2.x |
| Asset embedding | rust-embed |

## How Token Collection Works

The collector runs `kubectl logs` against each configured pod every 5 minutes. It parses OpenAB's standardized dispatch log format:

```
2026-07-15T09:52:08Z INFO dispatch{...}: batch dispatched ... tokens_per_event=[23] ... agent_dispatch_ms=72728
```

- `tokens_per_event=[N,N,...]` — summed as total tokens for that event
- `agent_dispatch_ms=N` — response latency in milliseconds

This format is consistent across all OpenAB-supported CLIs (kiro-cli, agy-acp, opencode, etc.), making the dashboard provider-agnostic.

## Contributing

Contributions welcome! Please open an issue or PR.

## License

[MIT](LICENSE)
