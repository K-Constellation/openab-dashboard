pub mod provider;
pub mod kiro;
pub mod antigravity;

use std::sync::Arc;
use chrono::Utc;
use crate::AppState;
use crate::config::ProviderConfig;
use crate::models::SessionPodSnapshot;
use tokio::time::{self, Duration};

pub async fn collect_once(state: Arc<AppState>) -> anyhow::Result<()> {
    let config = &state.config;

    for (provider_name, provider_config) in &config.providers.providers {
        if !provider_config.enabled {
            continue;
        }

        match collect_provider(provider_name, provider_config, &config.collector.kubectl_path, &state).await {
            Ok(count) => {
                let _ = state.db.log_collection(provider_name, None, "success", None, count);
                tracing::info!("{}: collected {} events", provider_name, count);
            }
            Err(e) => {
                let _ = state.db.log_collection(provider_name, None, "error", Some(&e.to_string()), 0);
                tracing::error!("{} collection failed: {}", provider_name, e);
            }
        }
    }

    // Update daily summary
    let _ = state.db.update_daily_summary();

    // Capture configured-pod snapshots only while a session is active.
    // Errors here are logged, not propagated, so they cannot transition session state.
    if let Ok(Some(session)) = state.db.get_active_recording_session() {
        let live_pods = get_live_pod_info(&config.collector.kubectl_path).await;

        for (provider_name, provider_config) in &config.providers.providers {
            for pod in &provider_config.pods {
                let live = live_pods.iter().find(|lp| lp.deployment.contains(&pod.name));
                let last_active = state.db.get_last_active_for_session_agent(session.id, &pod.name, provider_name).unwrap_or(None);
                let avg_ms = state.db.get_avg_duration_for_session_agent(session.id, &pod.name, provider_name).unwrap_or(0.0) as i64;

                let snapshot = SessionPodSnapshot {
                    id: 0,
                    session_id: session.id,
                    pod: pod.name.clone(),
                    provider: provider_name.clone(),
                    deployment: pod.deployment.clone(),
                    timestamp: Utc::now(),
                    status: if provider_config.enabled { "enabled".into() } else { "disabled".into() },
                    uptime: live.map(|l| l.age.clone()),
                    restarts: live.map(|l| l.restarts).unwrap_or(0),
                    version: live.map(|l| l.version.clone()),
                    last_active,
                    avg_response_ms: avg_ms,
                };
                if let Err(e) = state.db.insert_pod_snapshot(&snapshot) {
                    tracing::warn!("failed to capture pod snapshot for {}: {}", pod.name, e);
                }
            }
        }
    }

    Ok(())
}

async fn collect_provider(
    provider_name: &str,
    provider_config: &ProviderConfig,
    kubectl: &str,
    state: &Arc<AppState>,
) -> anyhow::Result<i64> {
    let mut total_count: i64 = 0;

    for pod in &provider_config.pods {
        let logs = get_logs(kubectl, &pod.deployment, "60m").await?;
        let records = parse_dispatch_logs(&logs, &pod.name, provider_name);
        let count = records.len() as i64;

        for record in &records {
            // Baseline events always persist. Active-session association happens
            // inside the database if the timestamp falls within an open interval.
            let _ = state.db.insert_usage_event(record, &pod.account_id);
        }
        total_count += count;

        // Provider-specific: Kiro metadata parsing
        if provider_name == "kiro" {
            let metadata_records = kiro::parse_metadata_logs(&logs, &pod.name);
            for record in &metadata_records {
                let _ = state.db.insert_usage_event(record, &pod.account_id);
            }
        }
    }

    Ok(total_count)
}

/// Shared dispatch log parser — works for ALL providers
/// Parses lines containing `batch dispatched` with `tokens_per_event=[N,N,...]` and `agent_dispatch_ms=N`
pub fn parse_dispatch_logs(logs: &str, agent: &str, provider: &str) -> Vec<crate::models::UsageRecord> {
    let mut records = vec![];

    for line in logs.lines() {
        let clean = strip_ansi(line);
        if clean.contains("batch dispatched") {
            if let Some(record) = parse_dispatch_line(&clean, agent, provider) {
                records.push(record);
            }
        }
    }

    records
}

fn parse_dispatch_line(line: &str, agent: &str, provider: &str) -> Option<crate::models::UsageRecord> {
    let tokens: i64 = line.split("tokens_per_event=[")
        .nth(1)?
        .split(']')
        .next()?
        .split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .sum();

    let duration_ms: Option<i64> = line.split("agent_dispatch_ms=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok());

    let timestamp = line.split_whitespace().next()
        .and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok())
        .unwrap_or_else(chrono::Utc::now);

    Some(crate::models::UsageRecord {
        timestamp,
        agent: agent.to_string(),
        provider: provider.to_string(),
        total_tokens: Some(tokens),
        duration_ms,
        ..Default::default()
    })
}

async fn get_logs(kubectl: &str, deployment: &str, since: &str) -> anyhow::Result<String> {
    let output = tokio::process::Command::new(kubectl)
        .args(["logs", &format!("deployment/{}", deployment), &format!("--since={}", since)])
        .output()
        .await?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() && !stderr.contains("not found") {
        tracing::warn!("kubectl logs {} stderr: {}", deployment, stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Strip ANSI escape codes from a string
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            while let Some(&next) = chars.peek() {
                chars.next();
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

struct LivePodInfo {
    deployment: String,
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

        Some(LivePodInfo { deployment: name, age, restarts, version })
    }).collect()
}

pub async fn run_scheduler(state: Arc<AppState>) {
    let interval = state.config.collector.interval_seconds;
    tracing::info!("collector scheduler started (every {}s)", interval);

    let mut ticker = time::interval(Duration::from_secs(interval));
    loop {
        ticker.tick().await;
        if let Err(e) = collect_once(state.clone()).await {
            tracing::error!("collection cycle failed: {}", e);
        }
    }
}
