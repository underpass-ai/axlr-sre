use anyhow::Result;
use async_trait::async_trait;

use crate::domain::Request;

/// The one thing the use cases need from Opsgenie: the JSON a request
/// answers. A non-2xx status is an `Err` whose message a model can act on;
/// a 401 or 403 names the key without repeating the body; a 429 says to
/// wait. The use cases only send a write when the caller confirmed it.
#[async_trait]
pub trait OpsgenieClient: Send + Sync {
    async fn send(&self, request: &Request) -> Result<serde_json::Value>;
}
