# OpenAB Dashboard

[English README](README.md)

一個輕量級、單一二進位檔的 [OpenAB](https://github.com/openabdev/openab) agent pod 監控儀表板。追蹤各 AI coding agent 的 token 用量、回應時間和 pod 健康狀態。

![Rust](https://img.shields.io/badge/rust-1.75%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## 功能特色

- **以 Token 為主要指標** — 追蹤每個 agent 每天的實際 LLM token 消耗
- **多 Provider 支援** — Kiro、Antigravity、OpenCode，或任何 CLI agent
- **Pod 健康監控** — 運行時間、重啟次數、版本、最後活躍時間
- **回應時間追蹤** — 各 agent 的平均 dispatch 延遲趨勢
- **排行榜** — 依 token 排名，支援時間區間過濾
- **5 種內建主題** — Cyberpunk Neon、Minimal Light、Glassmorphism、Terminal Green、Deep Ocean
- **零基礎設施** — SQLite 資料庫、嵌入式網頁資源、單一二進位檔
- **自動更新** — 儀表板每 60 秒自動刷新

## 架構

```
┌─────────────────────────────────────────────────────────┐
│  openab-dashboard（單一二進位檔，約 8 MB）                 │
│                                                         │
│  ┌─────────────┐    ┌──────────┐    ┌───────────────┐  │
│  │  Collector   │───▶│ SQLite   │◀───│ Web Server    │  │
│  │ （每 5 分鐘）│    │ usage.db │    │ (axum :8080)  │  │
│  └─────────────┘    └──────────┘    └───────────────┘  │
│        │                                     │          │
│        │ kubectl logs                        │ HTTP     │
│        ▼                                     ▼          │
│  ┌──────────────────────┐              瀏覽器           │
│  │ Pod A │ Pod B │ ...  │            （儀表板）          │
│  └──────────────────────┘                               │
└─────────────────────────────────────────────────────────┘
```

## 快速開始

### 前置需求

- Rust 1.75+（用於編譯）
- `kubectl` 已設定並可存取你的 OpenAB pods

### 建置與執行

```bash
# 複製
git clone https://github.com/masami-agent/openab-dashboard.git
cd openab-dashboard

# 編譯
cargo build --release

# 設定（編輯 config.yaml 填入你的 pods）
cp config.example.yaml config.yaml

# 執行
./target/release/openab-dashboard --config config.yaml

# 打開 http://localhost:8080
```

### 單次收集模式（測試用）

```bash
./target/release/openab-dashboard --config config.yaml --once
```

### 使用 Docker 執行

Dashboard 的收集器會呼叫本機的 `kubectl`，所以容器內需要有主機的
kubeconfig 副本，以及可用的 `kubectl`（已內建於映像檔）。
`docker-entrypoint.sh` 會複製掛載進來的 kubeconfig，並將其中的
`127.0.0.1`/`localhost` API server 位址改寫成能連回主機的名稱
（OrbStack 用 `k8s.orb.local`，其他情況用 `host.docker.internal`）。

```bash
cp config.example.yaml config.yaml
# 編輯 config.yaml，然後：
docker compose up -d --build
```

這會掛載 `~/.kube/config`（可用 `KUBECONFIG=/path/to/config` 覆寫），
唯讀掛載進容器，並將 dashboard 對外開在 `http://localhost:8080`。
容器工作目錄為 `/app/data`，因此 `config.example.yaml` 中的
`database.path: "./usage.db"` 會解析為 `/app/data/usage.db`，並透過
`openab-data` volume 持久保留。

不使用 Compose 的話：

```bash
docker build -t openab-dashboard .
docker run -d --name openab-dashboard \
  -p 127.0.0.1:8080:8080 \
  -v $(pwd)/config.yaml:/app/config.yaml:ro \
  -v ~/.kube/config:/host-kube/config:ro \
  -v openab-data:/app/data \
  --add-host host.docker.internal:host-gateway \
  openab-dashboard
```

注意事項：
- 預設 Compose 只將 dashboard 綁在 `127.0.0.1:8080`（localhost）。
  若要開放給 LAN 或遠端存取，請用 `BIND_ADDR=0.0.0.0 docker compose up -d --build`。
  Dashboard 沒有內建認證，開放所有 interface 時務必搭配 reverse proxy 或其他存取控制。
- 上方 non-Compose `docker run` 範例也綁在 `127.0.0.1:8080`。若需要 LAN
  或遠端存取，請將 `-p 127.0.0.1:8080:8080` 改為 `-p 0.0.0.0:8080:8080`，
  並在 front 加上 reverse proxy 或其他存取控制。預設僅綁 localhost 是為了
  避免無認證的 dashboard 意外暴露。
- 容器以 non-root `appuser`（UID 1000）執行。`/app/data` 與複製到
  `/home/appuser/.kube/config` 的 kubeconfig 對該使用者可寫入。
- `~/.kube/config` 必須是單一絕對路徑；多個以冒號分隔的檔案，或開頭為 `~` 的路徑不支援 volume mount。
- `--add-host host.docker.internal:host-gateway` 只有 Linux 需要；
  Docker Desktop 和 OrbStack 已內建提供 `host.docker.internal`。
- 若你的叢集無法透過 `host.docker.internal` / `k8s.orb.local` 連線
  （例如遠端叢集），只要讓 `KUBECONFIG`／掛載的 kubeconfig 指向容器
  已經能連上的位址即可，不需要改寫。
- 隨時可用 `docker exec openab-dashboard kubectl get pods -A` 驗證連線。
- 掛載 kubeconfig 等同於把叢集的 live credential 交給容器；請確保該檔案與執行主機的安全。
- Dashboard 沒有內建認證；在沒有 reverse proxy 或其他存取控制的情況下，
  請勿將 `8080` port 暴露到不可信的網路。
- `Dockerfile` 會將下載的 `kubectl` 與 `dl.k8s.io` 公布的 SHA256 checksum 進行驗證。
  若你變更 `KUBECTL_VERSION` 或在離線環境建置，請提供對應的 checksum。

## 設定檔

```yaml
database:
  path: "./usage.db"

server:
  host: "0.0.0.0"
  port: 8080

collector:
  interval_seconds: 300        # 每 5 分鐘收集一次
  kubectl_path: "/usr/local/bin/kubectl"

# 新增任意數量的 provider — 只需列出它們的 pods
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

  # 需要時新增更多 provider：
  # opencode:
  #   enabled: true
  #   pods:
  #     - name: my-opencode
  #       deployment: openab-opencode
  #       account_id: "opencode:free"
```

### 新增 Provider

只需在 config.yaml 的 `providers:` 底下加一個 section。不需要修改程式碼。Dashboard 會自動從任何 pod 的 dispatch logs 收集 `tokens_per_event` 和 `agent_dispatch_ms`。

### 本機叢集上的 Devin ACP

使用 `devin` provider 可從各 pod 的 Devin CLI session DB 匯入每一則模型回應所回報的用量。設定 `devin_session_db_mode: auto` 時，collector 只會在 OrbStack、Docker Desktop、minikube、kind 或 k3d 等可辨識的本機叢集執行；雲端和無法辨識的叢集會略過，雲端收集暫不包含在此版本。

第一次執行會匯入舊 session。後續執行會以 Devin 的 session ID 與 message ID 去重，不會重複計算。`total_tokens` 為 input 加 output；cache token 數則保留在 event metadata。

## API 端點

| 端點 | 說明 |
|------|------|
| `GET /` | 儀表板 UI |
| `GET /api/summary?days=14&provider=kiro` | 每日 token 用量（可過濾） |
| `GET /api/leaderboard?period=week&metric=tokens` | Agent 排名 |
| `GET /api/pods` | Pod 狀態：運行時間、版本、最後活躍時間 |
| `GET /api/response-time?days=14` | 各 agent 平均回應時間 |
| `GET /api/quota` | Credit 配額快照（有資料時顯示） |
| `GET /api/health` | 系統健康狀態 + 最後收集時間 |

## 部署方式

### 作為系統服務

```bash
# macOS (launchd)
cp com.openab.dashboard.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.openab.dashboard.plist

# Linux (systemd)
# 建立 service file 指向二進位檔
```

### 部署 = scp

```bash
scp target/release/openab-dashboard server:~/openab-dashboard/
scp config.yaml server:~/openab-dashboard/
ssh server "~/openab-dashboard/openab-dashboard --config ~/openab-dashboard/config.yaml"
```

## 技術棧

| 元件 | 技術 |
|------|------|
| 語言 | Rust |
| Web 框架 | axum 0.7 |
| 資料庫 | SQLite（rusqlite，嵌入式） |
| 圖表 | Chart.js 4.x（CDN） |
| 樣式 | Tailwind CSS（CDN） |
| 互動 | HTMX 2.x |
| 資源嵌入 | rust-embed |

## Token 收集原理

Collector 每 5 分鐘對每個設定的 pod 執行 `kubectl logs`。它解析 OpenAB 標準化的 dispatch log 格式：

```
2026-07-15T09:52:08Z INFO dispatch{...}: batch dispatched ... tokens_per_event=[23] ... agent_dispatch_ms=72728
```

- `tokens_per_event=[N,N,...]` — 加總作為該 event 的 total tokens
- `agent_dispatch_ms=N` — 回應延遲（毫秒）

此格式在所有 OpenAB 支援的 CLI（kiro-cli、agy-acp、opencode 等）中一致，讓 dashboard 不受特定 provider 限制。

## 貢獻

歡迎開 issue 或送 PR！

## 授權

[MIT](LICENSE)
