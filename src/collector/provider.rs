// Provider trait kept for reference / future use.
// Currently all providers use the shared dispatch log parser in mod.rs.
// If a provider needs custom parsing beyond dispatch logs, implement this trait.

use async_trait::async_trait;
use crate::models::{UsageRecord, QuotaSnapshot};

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn collect_events(&self) -> anyhow::Result<Vec<UsageRecord>>;
    async fn collect_quota(&self) -> anyhow::Result<Option<QuotaSnapshot>> {
        Ok(None)
    }
}
