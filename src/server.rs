use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::{delete, get, post},
};
use rust_embed::Embed;
use std::sync::Arc;
use crate::AppState;
use crate::is_deployment_pod_name;
use crate::models::*;
use serde::Deserialize;

#[derive(Embed)]
#[folder = "static/"]
struct StaticAssets;

pub fn build_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/summary", get(api_summary))
        .route("/api/quota", get(api_quota))
        .route("/api/leaderboard", get(api_leaderboard))
        .route("/api/health", get(api_health))
        .route("/api/response-time", get(api_response_time))
        .route("/api/pods", get(api_pods))
        .route("/api/recording-sessions", get(list_sessions).post(create_session))
        .route("/api/recording-sessions/open", get(get_open_session))
        .route("/api/recording-sessions/:id/pause", post(pause_session))
        .route("/api/recording-sessions/:id/resume", post(resume_session))
        .route("/api/recording-sessions/:id/archive", post(archive_session))
        .route("/api/recording-sessions/:id", delete(delete_session))
        .route("/static/*path", get(static_handler))
        .with_state(state)
}

pub async fn start(state: Arc<AppState>, port: u16) -> anyhow::Result<()> {
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    match StaticAssets::get("index.html") {
        Some(content) => Html(String::from_utf8_lossy(content.data.as_ref()).to_string()).into_response(),
        None => (StatusCode::NOT_FOUND, "index.html not found").into_response(),
    }
}

async fn static_handler(axum::extract::Path(path): axum::extract::Path<String>) -> impl IntoResponse {
    match StaticAssets::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                [(axum::http::header::CONTENT_TYPE, mime.to_string())],
                content.data.to_vec(),
            ).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[derive(Deserialize)]
struct SummaryParams {
    days: Option<i64>,
    agent: Option<String>,
    provider: Option<String>,
    session_id: Option<i64>,
}

async fn api_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryParams>,
) -> Json<SummaryResponse> {
    let db = &state.db;

    let summaries = if let Some(session_id) = params.session_id {
        db.get_daily_summary_for_session_unbounded(session_id).unwrap_or_default()
    } else {
        db.get_daily_summary(params.days.unwrap_or(14)).unwrap_or_default()
    };

    // Group by date
    let mut daily_map: std::collections::BTreeMap<String, Vec<AgentDailyData>> = std::collections::BTreeMap::new();
    for s in summaries {
        if let Some(ref agent_filter) = params.agent {
            if &s.agent != agent_filter { continue; }
        }
        if let Some(ref provider_filter) = params.provider {
            if &s.provider != provider_filter { continue; }
        }
        daily_map.entry(s.date.clone()).or_default().push(AgentDailyData {
            agent: s.agent,
            provider: s.provider,
            credits: if s.total_credits > 0.0 { Some(s.total_credits) } else { None },
            tokens: s.total_tokens,
            requests: s.request_count,
        });
    }

    let daily: Vec<DailyData> = daily_map.into_iter()
        .map(|(date, agents)| DailyData { date, agents })
        .collect();

    let end = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let start = if let Some(_session_id) = params.session_id {
        daily.first().map(|d| d.date.clone()).unwrap_or(end.clone())
    } else {
        let days = params.days.unwrap_or(14);
        (chrono::Utc::now() - chrono::Duration::days(days)).format("%Y-%m-%d").to_string()
    };

    Json(SummaryResponse {
        period: Period { start, end },
        daily,
    })
}

async fn api_quota(State(state): State<Arc<AppState>>) -> Json<QuotaResponse> {
    let db = &state.db;
    let quotas = db.get_latest_quota().unwrap_or_default();

    let accounts: Vec<AccountQuota> = quotas.into_iter().map(|(id, used, total, reset)| {
        let usage_percent = match (used, total) {
            (Some(u), Some(t)) if t > 0.0 => Some(u / t * 100.0),
            _ => None,
        };
        AccountQuota {
            provider: id.split(':').next().unwrap_or("").to_string(),
            id,
            plan: None,
            display_name: None,
            credits_used: used,
            credits_total: total,
            usage_percent,
            reset_at: reset,
            days_remaining: None,
            last_updated: None,
        }
    }).collect();

    Json(QuotaResponse { accounts })
}

#[derive(Deserialize)]
struct LeaderboardParams {
    period: Option<String>,
    metric: Option<String>,
    session_id: Option<i64>,
}

async fn api_leaderboard(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LeaderboardParams>,
) -> Json<LeaderboardResponse> {
    let period = params.period.unwrap_or_else(|| "month".into());
    let metric = params.metric.unwrap_or_else(|| "tokens".into());

    let db = &state.db;

    let mut ranking: Vec<RankEntry> = if let Some(session_id) = params.session_id {
        let rows = db.get_leaderboard_for_session(session_id).unwrap_or_default();
        rows.into_iter()
            .map(|(agent, provider, credits, tokens, requests)| {
                let value = match metric.as_str() {
                    "credits" => if credits > 0.0 { Some(credits) } else { None },
                    "requests" => Some(requests as f64),
                    _ => Some(tokens as f64),
                };
                RankEntry { rank: 0, agent, provider, value, requests, tokens }
            })
            .collect()
    } else {
        let days: i64 = match period.as_str() {
            "today" => 1,
            "week" => 7,
            _ => 30,
        };
        let summaries = db.get_daily_summary(days).unwrap_or_default();

        // Aggregate by agent
        let mut agent_totals: std::collections::HashMap<String, (String, f64, i64, i64)> = std::collections::HashMap::new();
        for s in summaries {
            let entry = agent_totals.entry(s.agent.clone()).or_insert((s.provider.clone(), 0.0, 0, 0));
            entry.1 += s.total_credits;
            entry.2 += s.total_tokens;
            entry.3 += s.request_count;
        }

        agent_totals.into_iter()
            .map(|(agent, (provider, credits, tokens, requests))| {
                let value = match metric.as_str() {
                    "credits" => if credits > 0.0 { Some(credits) } else { None },
                    "requests" => Some(requests as f64),
                    _ => Some(tokens as f64),
                };
                RankEntry { rank: 0, agent, provider, value, requests, tokens }
            })
            .collect()
    };

    // Sort by value descending
    ranking.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
    for (i, entry) in ranking.iter_mut().enumerate() {
        entry.rank = i + 1;
    }

    Json(LeaderboardResponse { period, metric, ranking })
}

async fn api_health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let db = &state.db;
    let size = db.db_size_mb();
    let last_collection = db.get_last_collection().unwrap_or(None);

    let mut providers = std::collections::HashMap::new();
    if let Ok(health_data) = db.get_provider_health() {
        for (provider, status, timestamp) in health_data {
            providers.insert(provider, ProviderHealth {
                status,
                last_success: timestamp,
            });
        }
    }

    Json(HealthResponse {
        status: "healthy".into(),
        last_collection,
        providers,
        db_size_mb: size,
        uptime_seconds: 0,
    })
}

#[derive(Deserialize)]
struct ResponseTimeParams {
    days: Option<i64>,
    session_id: Option<i64>,
}

async fn api_response_time(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ResponseTimeParams>,
) -> Json<serde_json::Value> {
    let db = &state.db;

    let data = if let Some(session_id) = params.session_id {
        db.get_avg_duration_for_session_unbounded(session_id).unwrap_or_default()
    } else {
        db.get_avg_duration_by_agent(params.days.unwrap_or(14)).unwrap_or_default()
    };

    // Group by date, with agents as series
    let mut daily_map: std::collections::BTreeMap<String, std::collections::HashMap<String, f64>> = std::collections::BTreeMap::new();
    let mut agents_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (agent, date, avg_ms) in &data {
        agents_set.insert(agent.clone());
        daily_map.entry(date.clone()).or_default().insert(agent.clone(), *avg_ms / 1000.0); // convert to seconds
    }

    let agents: Vec<String> = agents_set.into_iter().collect();
    let dates: Vec<String> = daily_map.keys().cloned().collect();
    let series: Vec<serde_json::Value> = agents.iter().map(|agent| {
        let values: Vec<f64> = dates.iter().map(|d| {
            daily_map.get(d).and_then(|m| m.get(agent)).copied().unwrap_or(0.0)
        }).collect();
        serde_json::json!({ "agent": agent, "data": values })
    }).collect();

    Json(serde_json::json!({
        "dates": dates,
        "series": series
    }))
}

#[derive(Deserialize)]
struct PodsParams {
    session_id: Option<i64>,
}

async fn api_pods(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PodsParams>,
) -> Json<serde_json::Value> {
    if let Some(session_id) = params.session_id {
        return Json(session_pods(&state.db, session_id));
    }

    let config = &state.config;
    let mut pods = vec![];

    // Try to get live pod info from kubectl
    let live_pods = get_live_pod_info(&config.collector.kubectl_path).await;

    for (provider_name, provider_config) in &config.providers.providers {
        for pod in &provider_config.pods {
            // Kubernetes pod names follow `<deployment>-<rs-hash>-<pod-hash>`,
            // while `pod.name` is a free-form display label and is not reliable for this lookup.
            let live = live_pods.iter().find(|lp| is_deployment_pod_name(&lp.pod_name, &pod.deployment));

            // Get last active time from DB
            let last_active = state.db.get_last_active(&pod.name).unwrap_or(None);

            // Get avg response time from DB (last 24h)
            let avg_ms = state.db.get_avg_duration_for_agent(&pod.name, 1).unwrap_or(0.0);

            pods.push(serde_json::json!({
                "name": pod.name,
                "provider": provider_name,
                "deployment": pod.deployment,
                "status": if provider_config.enabled { "enabled" } else { "disabled" },
                "uptime": live.map(|l| l.age.clone()),
                "restarts": live.map(|l| l.restarts),
                "version": live.map(|l| l.version.clone()),
                "last_active": last_active,
                "avg_response_ms": avg_ms as i64,
            }));
        }
    }

    Json(serde_json::json!({ "pods": pods }))
}

fn session_pods(db: &crate::db::Database, session_id: i64) -> serde_json::Value {
    let snapshots = db.get_pod_snapshots_for_session(session_id).unwrap_or_default();
    let pods: Vec<serde_json::Value> = snapshots.into_iter().map(|s| {
        serde_json::json!({
            "name": s.pod,
            "provider": s.provider,
            "deployment": s.deployment,
            "status": s.status,
            "uptime": s.uptime,
            "restarts": s.restarts,
            "version": s.version,
            "last_active": s.last_active,
            "avg_response_ms": s.avg_response_ms,
        })
    }).collect();
    serde_json::json!({ "pods": pods })
}

async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<Vec<RecordingSession>> {
    Json(state.db.list_recording_sessions().unwrap_or_default())
}

async fn get_open_session(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.db.get_open_recording_session() {
        Ok(Some(s)) => Json(s).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
struct CreateSession {
    name: String,
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSession>,
) -> Result<(StatusCode, Json<RecordingSession>), (StatusCode, String)> {
    let name = payload.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err((StatusCode::BAD_REQUEST, "session name must be non-empty and at most 80 characters".into()));
    }
    let id = state.db.create_recording_session(&payload.name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let session = state.db.get_recording_session_by_id(id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "session not found".into()))?;
    Ok((StatusCode::CREATED, Json(session)))
}

fn require_json_content_type(headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    match headers.get("content-type").and_then(|v| v.to_str().ok()) {
        Some(ct) if ct.split(';').next().map(|s| s.trim() == "application/json").unwrap_or(false) => Ok(()),
        _ => Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "Content-Type must be application/json".into())),
    }
}

async fn pause_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<RecordingSession>, (StatusCode, String)> {
    require_json_content_type(&headers)?;
    state.db.pause_recording_session(id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let session = state.db.get_recording_session_by_id(id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "session not found".into()))?;
    Ok(Json(session))
}

async fn resume_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<RecordingSession>, (StatusCode, String)> {
    require_json_content_type(&headers)?;
    state.db.resume_recording_session(id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let session = state.db.get_recording_session_by_id(id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "session not found".into()))?;
    Ok(Json(session))
}

async fn archive_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<RecordingSession>, (StatusCode, String)> {
    require_json_content_type(&headers)?;
    state.db.archive_recording_session(id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let session = state.db.get_recording_session_by_id(id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "session not found".into()))?;
    Ok(Json(session))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    require_json_content_type(&headers)?;
    state.db.delete_recording_session(id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

struct LivePodInfo {
    pod_name: String,
    age: String,
    restarts: i32,
    version: String,
}

async fn get_live_pod_info(kubectl: &str) -> Vec<LivePodInfo> {
    let output = tokio::process::Command::new(kubectl)
        .args(["get", "pods", "-o", "jsonpath={range .items[*]}{.metadata.name}|{.metadata.creationTimestamp}|{.status.containerStatuses[0].restartCount}|{.spec.containers[0].image}{\"\\n\"}{end}"])
        .output()
        .await;

    let Ok(output) = output else { return vec![] };
    let stdout = String::from_utf8_lossy(&output.stdout);

    stdout.lines().filter_map(|line| {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 4 { return None; }
        let name = parts[0].to_string();
        let created = parts[1];
        let restarts: i32 = parts[2].parse().unwrap_or(0);
        let image = parts[3];

        // Calculate uptime
        let age = if let Ok(ts) = created.parse::<chrono::DateTime<chrono::Utc>>() {
            let duration = chrono::Utc::now() - ts;
            if duration.num_days() > 0 {
                format!("{}d", duration.num_days())
            } else if duration.num_hours() > 0 {
                format!("{}h", duration.num_hours())
            } else {
                format!("{}m", duration.num_minutes())
            }
        } else {
            "?".to_string()
        };

        // Extract version from image tag
        let version = image.rsplit(':').next().unwrap_or("unknown").to_string();

        Some(LivePodInfo { pod_name: name, age, restarts, version })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DatabaseConfig, ServerConfig, CollectorConfig, ProvidersConfig};
    use crate::db::Database;
    use crate::models::UsageRecord;
    use chrono::Utc;

    async fn test_server() -> (u16, reqwest::Client) {
        let db = Database::new(":memory:").unwrap();
        db.initialize().unwrap();

        let config = Config {
            database: DatabaseConfig { path: ":memory:".into() },
            server: ServerConfig { host: "127.0.0.1".into(), port: 0 },
            collector: CollectorConfig {
                interval_seconds: 300,
                kubectl_path: "/bin/false".into(),
                devin_session_db_mode: crate::config::DevinSessionDbMode::Auto,
            },
            providers: ProvidersConfig { providers: std::collections::HashMap::new() },
        };
        let state = Arc::new(AppState { db, config });
        let app = build_app(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        (port, client)
    }

    #[tokio::test]
    async fn api_summary_baseline_vs_session() {
        // Baseline event, not associated with any session
        let baseline_record = UsageRecord {
            timestamp: Utc::now() - chrono::Duration::hours(1),
            agent: "baseline".into(),
            provider: "kiro".into(),
            ..Default::default()
        };

        // We need a state handle to insert data, but the db moved into AppState.
        // Seed data before building the app.
        let (port2, client2, session_id) = {
            let db = Database::new(":memory:").unwrap();
            db.initialize().unwrap();

            db.insert_usage_event(&baseline_record, "kiro:user").unwrap();

            let session_id = db.create_recording_session("rec").unwrap();
            let in_session = UsageRecord {
                timestamp: Utc::now(),
                agent: "insession".into(),
                provider: "kiro".into(),
                ..Default::default()
            };
            db.insert_usage_event(&in_session, "kiro:user").unwrap();

            let config = Config {
                database: DatabaseConfig { path: ":memory:".into() },
                server: ServerConfig { host: "127.0.0.1".into(), port: 0 },
                collector: CollectorConfig {
                    interval_seconds: 300,
                    kubectl_path: "/bin/false".into(),
                    devin_session_db_mode: crate::config::DevinSessionDbMode::Auto,
                },
                providers: ProvidersConfig { providers: std::collections::HashMap::new() },
            };
            let state = Arc::new(AppState { db, config });
            let app = build_app(state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            (port, reqwest::Client::new(), session_id)
        };

        let base: serde_json::Value = client2
            .get(format!("http://127.0.0.1:{}/api/summary?days=1", port2))
            .send().await.unwrap()
            .json().await.unwrap();
        let base_requests: i64 = base["daily"].as_array().unwrap().iter()
            .flat_map(|d| d["agents"].as_array().into_iter().flatten())
            .map(|a| a["requests"].as_i64().unwrap())
            .sum();
        assert_eq!(base_requests, 2);

        let session: serde_json::Value = client2
            .get(format!("http://127.0.0.1:{}/api/summary?days=1&session_id={}", port2, session_id))
            .send().await.unwrap()
            .json().await.unwrap();
        let session_requests: i64 = session["daily"].as_array().unwrap().iter()
            .flat_map(|d| d["agents"].as_array().into_iter().flatten())
            .map(|a| a["requests"].as_i64().unwrap())
            .sum();
        assert_eq!(session_requests, 1);
    }

    #[tokio::test]
    async fn recording_session_lifecycle_boundary() {
        let (port, client) = test_server().await;

        // Empty name -> 400
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions", port))
            .json(&serde_json::json!({ "name": "" }))
            .send().await.unwrap();
        assert_eq!(r.status(), 400);

        // Valid name -> 201
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions", port))
            .json(&serde_json::json!({ "name": "good" }))
            .send().await.unwrap();
        assert_eq!(r.status(), 201);

        // Duplicate / open session exists -> 400
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions", port))
            .json(&serde_json::json!({ "name": "good" }))
            .send().await.unwrap();
        assert_eq!(r.status(), 400);

        // Delete unarchived -> 400
        let r = client
            .delete(format!("http://127.0.0.1:{}/api/recording-sessions/1", port))
            .header("content-type", "application/json")
            .send().await.unwrap();
        assert_eq!(r.status(), 400);
    }

    #[tokio::test]
    async fn recording_session_lifecycle_requires_json_content_type() {
        let (port, client) = test_server().await;

        // Create a session with JSON content type
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions", port))
            .json(&serde_json::json!({ "name": "csrf-test" }))
            .send().await.unwrap();
        assert_eq!(r.status(), 201);
        let session: RecordingSession = r.json().await.unwrap();
        assert_eq!(session.status, RecordingSessionStatus::Active);

        // Pause without Content-Type -> 415, status unchanged
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions/{}/pause", port, session.id))
            .send().await.unwrap();
        assert_eq!(r.status(), 415);
        let current: RecordingSession = client
            .get(format!("http://127.0.0.1:{}/api/recording-sessions/open", port))
            .send().await.unwrap()
            .json().await.unwrap();
        assert_eq!(current.status, RecordingSessionStatus::Active);

        // Pause with form content type -> 415, status unchanged
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions/{}/pause", port, session.id))
            .header("content-type", "application/x-www-form-urlencoded")
            .send().await.unwrap();
        assert_eq!(r.status(), 415);

        // Pause with JSON; charset -> 200 and status paused
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions/{}/pause", port, session.id))
            .header("content-type", "application/json; charset=utf-8")
            .send().await.unwrap();
        assert_eq!(r.status(), 200);
        let current: RecordingSession = client
            .get(format!("http://127.0.0.1:{}/api/recording-sessions/open", port))
            .send().await.unwrap()
            .json().await.unwrap();
        assert_eq!(current.status, RecordingSessionStatus::Paused);

        // Archive with JSON -> 200 and archived
        let r = client
            .post(format!("http://127.0.0.1:{}/api/recording-sessions/{}/archive", port, session.id))
            .header("content-type", "application/json")
            .send().await.unwrap();
        assert_eq!(r.status(), 200);
        let archived: RecordingSession = r.json().await.unwrap();
        assert_eq!(archived.status, RecordingSessionStatus::Archived);

        // Delete with form content type -> 415
        let r = client
            .delete(format!("http://127.0.0.1:{}/api/recording-sessions/{}", port, session.id))
            .header("content-type", "multipart/form-data")
            .send().await.unwrap();
        assert_eq!(r.status(), 415);

        // Delete with JSON -> 204
        let r = client
            .delete(format!("http://127.0.0.1:{}/api/recording-sessions/{}", port, session.id))
            .header("content-type", "application/json")
            .send().await.unwrap();
        assert_eq!(r.status(), 204);
    }

    #[tokio::test]
    async fn api_summary_ignores_days_when_session_id_present() {
        let (port2, client2, session_id) = {
            let db = Database::new(":memory:").unwrap();
            db.initialize().unwrap();

            let session_id = db.create_recording_session("prod-reg").unwrap();

            // A single in-session event right now
            let event = UsageRecord {
                timestamp: Utc::now(),
                agent: "reg-agent".into(),
                provider: "kiro".into(),
                ..Default::default()
            };
            db.insert_usage_event(&event, "kiro:user").unwrap();

            let config = Config {
                database: DatabaseConfig { path: ":memory:".into() },
                server: ServerConfig { host: "127.0.0.1".into(), port: 0 },
                collector: CollectorConfig {
                    interval_seconds: 300,
                    kubectl_path: "/bin/false".into(),
                    devin_session_db_mode: crate::config::DevinSessionDbMode::Auto,
                },
                providers: ProvidersConfig { providers: std::collections::HashMap::new() },
            };
            let state = Arc::new(AppState { db, config });
            let app = build_app(state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            (port, reqwest::Client::new(), session_id)
        };

        // days=0 would exclude the event if the days filter were applied to the session.
        let summary: serde_json::Value = client2
            .get(format!("http://127.0.0.1:{}/api/summary?days=0&session_id={}", port2, session_id))
            .send().await.unwrap()
            .json().await.unwrap();

        let total: i64 = summary["daily"].as_array().unwrap().iter()
            .flat_map(|d| d["agents"].as_array().into_iter().flatten())
            .map(|a| a["requests"].as_i64().unwrap())
            .sum();
        // Session-scoped query must ignore the days parameter and return all session data.
        assert_eq!(total, 1);
    }
}
