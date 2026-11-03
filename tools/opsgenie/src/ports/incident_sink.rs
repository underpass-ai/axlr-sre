use anyhow::Result;
use async_trait::async_trait;

use crate::domain::IncidentDraft;

/// Where the mirror sends the incidents: axlr-bots. `open` answers the
/// incident id, new or the one already open for the group key; `resolve` is
/// fine with an incident that is already resolved.
#[async_trait]
pub trait IncidentSink: Send + Sync {
    async fn open(&self, draft: &IncidentDraft) -> Result<String>;
    async fn resolve(&self, incident_id: &str, reason: &str) -> Result<()>;
}
