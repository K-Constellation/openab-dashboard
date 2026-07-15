# Changelog

## 0.1.0 (2026-07-15)

Initial public release.

### Features

- Token-based usage monitoring (primary metric)
- Generic provider system — add new CLIs via config only
- Pod health monitoring (uptime, restarts, version, last active)
- Average response time tracking with line chart
- Daily usage stacked bar chart (Chart.js)
- Agent leaderboard with period filtering
- 5 built-in UI themes (Cyberpunk Neon, Minimal Light, Glassmorphism, Terminal Green, Deep Ocean)
- Event deduplication (prevents duplicate counting from overlapping collection windows)
- REST API with provider/agent filtering
- Auto-refresh every 60 seconds
- Single binary deployment with embedded web assets

### Supported Providers

- Kiro (kiro-cli)
- Antigravity (agy-acp)
- Any OpenAB-compatible CLI (extensible via config)
