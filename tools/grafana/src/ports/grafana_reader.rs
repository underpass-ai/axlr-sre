use anyhow::Result;
use async_trait::async_trait;

use crate::domain::Request;

/// The one thing the use cases need from Grafana: the JSON a GET answers.
/// A non-2xx status is an `Err` whose message a model can act on; a 401 or
/// 403 names the token without repeating the body.
#[async_trait]
pub trait GrafanaReader: Send + Sync {
    async fn get(&self, request: &Request) -> Result<serde_json::Value>;
}
