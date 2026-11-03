use async_trait::async_trait;
use serde_json::Value;

use crate::domain::{OutputFormat, ToolOutcome};

/// What a tool gives the server: who it is, what it offers and how to call it.
#[async_trait]
pub trait ToolHost: Send + Sync {
    fn name(&self) -> &'static str;

    fn version(&self) -> &'static str;

    /// What `initialize` tells the model about this server.
    fn instructions(&self) -> &'static str;

    /// The catalogue, one [`crate::ToolDefinition::to_json`] per tool.
    fn tools(&self) -> Vec<Value>;

    async fn call(&self, name: &str, arguments: Value) -> ToolOutcome;

    fn output_format(&self) -> OutputFormat {
        OutputFormat::default()
    }

    fn prompts(&self) -> Vec<Value> {
        Vec::new()
    }

    fn prompt(&self, _name: &str, _arguments: &Value) -> Option<Value> {
        None
    }
}
