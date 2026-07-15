pub mod provider;
pub mod kiro;
pub mod antigravity;

use std::sync::Arc;
use crate::AppState;
use crate::config::ProviderConfig;
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
