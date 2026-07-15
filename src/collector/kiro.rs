use chrono::{DateTime, Utc};
use crate::models::UsageRecord;

/// Parse Kiro-specific `_kiro.dev/metadata` lines.
/// These contain contextUsagePercentage, turnDurationMs, and optional meteringUsage (credits).
/// Note: these do NOT contain token counts — tokens come from dispatch logs (shared parser).
pub fn parse_metadata_logs(logs: &str, agent: &str) -> Vec<UsageRecord> {
    let mut records = vec![];

    for line in logs.lines() {
        let clean = super::strip_ansi(line);
        if clean.contains("_kiro.dev/metadata") {
            if let Some(record) = parse_metadata_line(&clean, agent) {
                records.push(record);
            }
        }
    }

    records
}

fn parse_metadata_line(line: &str, agent: &str) -> Option<UsageRecord> {
    // Extract JSON from log line: line="{ ... }"
    let json_start = line.find("line=\"")? + 6;
    let json_end = line.rfind('"')?;
    if json_start >= json_end {
        return None;
    }
    let json_str = &line[json_start..json_end].replace("\\\"", "\"");

    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let params = parsed.get("params")?;

    let session_id = params.get("sessionId")?.as_str()?.to_string();
    let context_pct = params.get("contextUsagePercentage")?.as_f64();
    let duration_ms = params.get("turnDurationMs").and_then(|v| v.as_i64());

    // Extract credits if available (only when Kiro credits are limited)
    let credits = params.get("meteringUsage")
        .and_then(|u| u.as_array())
        .and_then(|arr| arr.first())
        .and_then(|m| m.get("value"))
        .and_then(|v| v.as_f64());

    let timestamp = parse_log_timestamp(line).unwrap_or_else(Utc::now);

    Some(UsageRecord {
        timestamp,
        agent: agent.to_string(),
        provider: "kiro".to_string(),
        session_id: Some(session_id),
        context_usage_pct: context_pct,
        duration_ms,
        credits_consumed: credits,
        // No total_tokens here — tokens come from dispatch logs
        ..Default::default()
    })
}

fn parse_log_timestamp(line: &str) -> Option<DateTime<Utc>> {
    let ts_str = line.split_whitespace().next()?;
    ts_str.parse::<DateTime<Utc>>().ok()
}
