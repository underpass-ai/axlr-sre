use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::domain::ArgoQuery;

/// What the use cases need from Argo: the JSON an API GET answers.
#[async_trait]
pub trait ArgoReader: Send + Sync {
    async fn read(&self, query: &ArgoQuery) -> Result<Value>;
}
