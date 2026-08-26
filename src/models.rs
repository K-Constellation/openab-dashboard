use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageRecord {
    pub timestamp: DateTime<Utc>,
    pub agent: String,
    pub provider: String,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub credits_consumed: Option<f64>,
    pub duration_ms: Option<i64>,
    pub context_usage_pct: Option<f64>,
    pub session_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    pub timestamp: DateTime<Utc>,
    pub account_id: String,
    pub credits_used: Option<f64>,
    pub credits_total: Option<f64>,
    pub reset_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySummary {
    pub date: String,
    pub account_id: String,
    pub agent: String,
    pub provider: String,
    pub total_credits: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub request_count: i64,
    pub avg_duration_ms: i64,
}

// API response types
#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    pub period: Period,
    pub daily: Vec<DailyData>,
}

#[derive(Debug, Serialize)]
pub struct Period {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Serialize)]
pub struct DailyData {
    pub date: String,
    pub agents: Vec<AgentDailyData>,
}

#[derive(Debug, Serialize)]
pub struct AgentDailyData {
    pub agent: String,
    pub provider: String,
    pub credits: Option<f64>,
    pub tokens: i64,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct QuotaResponse {
    pub accounts: Vec<AccountQuota>,
}

#[derive(Debug, Serialize)]
pub struct AccountQuota {
    pub id: String,
    pub provider: String,
    pub plan: Option<String>,
    pub display_name: Option<String>,
    pub credits_used: Option<f64>,
    pub credits_total: Option<f64>,
    pub usage_percent: Option<f64>,
    pub reset_at: Option<String>,
    pub days_remaining: Option<i64>,
    pub last_updated: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LeaderboardResponse {
    pub period: String,
    pub metric: String,
    pub ranking: Vec<RankEntry>,
}

#[derive(Debug, Serialize)]
pub struct RankEntry {
    pub rank: usize,
    pub agent: String,
    pub provider: String,
    pub value: Option<f64>,
    pub requests: i64,
    pub tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub last_collection: Option<String>,
    pub providers: std::collections::HashMap<String, ProviderHealth>,
    pub db_size_mb: f64,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct ProviderHealth {
    pub status: String,
    pub last_success: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecordingSessionStatus {
    #[default]
    Active,
    Paused,
    Archived,
}

impl RecordingSessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Archived => "archived",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSession {
    pub id: i64,
    pub name: String,
    pub status: RecordingSessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingInterval {
    pub id: i64,
    pub session_id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSessionEvent {
    pub id: i64,
    pub session_id: i64,
    pub event_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPodSnapshot {
    pub id: i64,
    pub session_id: i64,
    pub pod: String,
    pub provider: String,
    pub deployment: String,
    pub timestamp: DateTime<Utc>,
    pub status: String,
    pub uptime: Option<String>,
    pub restarts: i32,
    pub version: Option<String>,
    pub last_active: Option<String>,
    pub avg_response_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serde_lowercase() {
        let active = serde_json::to_string(&RecordingSessionStatus::Active).unwrap();
        assert_eq!(active, "\"active\"");
        let paused = serde_json::to_string(&RecordingSessionStatus::Paused).unwrap();
        assert_eq!(paused, "\"paused\"");
        let archived = serde_json::to_string(&RecordingSessionStatus::Archived).unwrap();
        assert_eq!(archived, "\"archived\"");

        let parsed: RecordingSessionStatus = serde_json::from_str("\"active\"").unwrap();
        assert_eq!(parsed, RecordingSessionStatus::Active);
    }
}
