use async_trait::async_trait;
use serde_json::{Value, json};

use crate::domain::{ToolDefinition, ToolOutcome};
use crate::ports::ToolHost;

pub(crate) struct FakeToolHost;

#[async_trait]
impl ToolHost for FakeToolHost {
    fn name(&self) -> &'static str {
        "test-tool"
    }

    fn version(&self) -> &'static str {
        "1.2.3"
    }

    fn instructions(&self) -> &'static str {
        "for testing"
    }

    fn tools(&self) -> Vec<Value> {
        vec![
            ToolDefinition::new("greet", "greets")
                .text("a", "who")
                .to_json(),
        ]
    }

    async fn call(&self, name: &str, arguments: Value) -> ToolOutcome {
        match name {
            "greet" => ToolOutcome::Ok(json!({"hello": arguments.get("a")})),
            _ => ToolOutcome::failed(
                format!("unknown tool: {name}"),
                Some("see tools/list".into()),
            ),
        }
    }

    fn prompts(&self) -> Vec<Value> {
        vec![json!({
            "name": "hello",
            "description": "greets someone",
            "arguments": [{"name": "who", "description": "who to greet", "required": false}],
        })]
    }

    fn prompt(&self, name: &str, arguments: &Value) -> Option<Value> {
        (name == "hello").then(|| {
            let who = arguments.get("who").and_then(Value::as_str).unwrap_or("world");
            json!({
                "description": "greets someone",
                "messages": [{"role": "user", "content": {"type": "text", "text": format!("hello {who}")}}],
            })
        })
    }
}
