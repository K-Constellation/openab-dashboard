# OpenAB Dashboard — Deployment Guide

## Phase 1: your-server Virtualenv (Current)

### Prerequisites

- Python 3.11+ on your-server
- `kubectl` configured with cluster access
- Network access to pods (same machine)

### Setup

```bash
ssh your-server
cd ~/openab-dashboard

# Create virtualenv
python3 -m venv .venv
source .venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Initialize database
python -m collector.db init

# Run collector once (test)
python -m collector.main --once

# Start web server (development)
python -m server.main
```

### Running as Background Service

```bash
# macOS launchd plist for collector
# Location: ~/Library/LaunchAgents/com.openab.dashboard.plist
launchctl load ~/Library/LaunchAgents/com.openab.dashboard.plist
```

### Directory Structure on your-server

```
~/openab-dashboard/
├── .venv/                  # Python virtualenv
├── usage.db                # SQLite database (gitignored)
├── config.yaml             # Active configuration
├── collector/              # Data collection code
├── server/                 # Web server code
├── templates/              # Dashboard HTML
├── static/                 # CSS, JS, images
└── docs/                   # Design documents
```

---

## Phase 2: Docker Container (Future)

### Dockerfile

```dockerfile
FROM python:3.11-slim

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY . .

# SQLite DB on volume mount
VOLUME ["/data"]
ENV DB_PATH=/data/usage.db

EXPOSE 8080

CMD ["python", "-m", "server.main"]
```

### Docker Compose (your-server)

```yaml
version: "3.8"
services:
  dashboard:
    build: .
    ports:
      - "8080:8080"
    volumes:
      - dashboard-data:/data
      - ~/.kube/config:/root/.kube/config:ro  # kubectl access
    environment:
      - KUBECTL_PATH=/usr/local/bin/kubectl
      - DB_PATH=/data/usage.db
    restart: unless-stopped

volumes:
  dashboard-data:
```

---

## Phase 3: K8s Pod (Future)

### Helm Values (sub-chart or standalone)

```yaml
dashboard:
  enabled: true
  image: ghcr.io/openabdev/openab-dashboard:latest
  port: 8080
  persistence:
    enabled: true
    size: 1Gi
  serviceAccount:
    create: true
    # Needs: pods/exec, pods/log (read-only)
  config:
    interval_seconds: 300
    providers:
      kiro:
        enabled: true
        pods: [openab-masami-kiro, openab-chloe-kiro]
      antigravity:
        enabled: true
        pods: [openab-misaki-gemini]
```

### RBAC Requirements

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
rules:
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["get", "list"]
  - apiGroups: [""]
    resources: ["pods/exec"]
    verbs: ["create"]
  - apiGroups: [""]
    resources: ["pods/log"]
    verbs: ["get"]
```

---

## Configuration

### config.yaml

```yaml
# Database
database:
  path: "./usage.db"       # Phase 1: relative path
  # path: "/data/usage.db" # Phase 2/3: volume mount

# Web server
server:
  host: "0.0.0.0"
  port: 8080

# Collector
collector:
  interval_seconds: 300    # 5 minutes
  kubectl_path: "/usr/local/bin/kubectl"

# Providers
providers:
  kiro:
    enabled: true
    pods:
      - name: masami
        deployment: openab-masami-kiro
        account_id: "kiro:user"
      - name: chloe
        deployment: openab-chloe-kiro
        account_id: "kiro:user"

  antigravity:
    enabled: true
    pods:
      - name: misaki
        deployment: openab-misaki-gemini
        account_id: "agy:user@example.com"
        oauth_token_path: "/home/agent/.gemini/antigravity-cli/antigravity-oauth-token"
```

---

## Monitoring the Dashboard Itself

- **Health endpoint:** `GET /api/health`
- **Logs:** stdout (or `dashboard.log` in Phase 1)
- **Alerts:** If `last_collection` > 15 minutes old, something is wrong
