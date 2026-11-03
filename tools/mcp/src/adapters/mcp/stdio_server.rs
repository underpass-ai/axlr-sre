use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use serde_json::{Value, json};

use super::in_memory_client::InMemoryClient;
use super::tool_host_handler::ToolHostHandler;
use crate::ports::ToolHost;

const HANDSHAKE_ID: &str = "sre-mcp-handshake";

/// Serves a [`ToolHost`] over stdio. `handle` answers one request in memory,
/// for a tool's contract tests.
pub struct StdioServer {
    handler: ToolHostHandler,
}

impl StdioServer {
    #[must_use]
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self {
            handler: ToolHostHandler::new(host),
        }
    }

    /// Serves until the client closes stdin. stdout is the protocol's: nothing else writes to it.
    pub async fn run(self) -> std::io::Result<()> {
        let service = self
            .handler
            .serve(stdio())
            .await
            .map_err(std::io::Error::other)?;
        service.waiting().await.map_err(std::io::Error::other)?;
        Ok(())
    }

    /// One request against a fresh in-memory connection, initialized first unless it is the `initialize`.
    pub async fn handle(&self, id: Value, method: &str, params: Value) -> Value {
        let mut client = InMemoryClient::connect(self.handler.clone());
        if method == "initialize" {
            return client
                .request(id, method, InMemoryClient::initialize_params(params))
                .await;
        }
        let handshake = InMemoryClient::initialize_params(json!({}));
        client
            .request(json!(HANDSHAKE_ID), "initialize", handshake)
            .await;
        client.notify("notifications/initialized").await;
        client.request(id, method, params).await
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::ProtocolVersion;

    use super::*;
    use crate::ports::FakeToolHost;

    fn server() -> StdioServer {
        StdioServer::new(Arc::new(FakeToolHost))
    }

    #[tokio::test]
    async fn initialize_with_empty_params_introduces_the_host() {
        let response = server().handle(json!(1), "initialize", json!({})).await;
        let info = &response["result"];
        assert_eq!(info["serverInfo"]["name"], "test-tool");
        assert_eq!(info["serverInfo"]["version"], "1.2.3");
        assert_eq!(info["instructions"], "for testing");
        assert_eq!(
            info["protocolVersion"],
            json!(ProtocolVersion::LATEST_WITH_INITIALIZE)
        );
        assert!(info["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn tools_list_publishes_the_host_catalogue() {
        let response = server().handle(json!(2), "tools/list", json!({})).await;
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "greet");
        assert_eq!(tools[0]["inputSchema"]["properties"]["a"]["type"], "string");
    }

    #[tokio::test]
    async fn a_good_call_comes_back_as_text_without_error_flag() {
        let params = json!({"name": "greet", "arguments": {"a": 1}});
        let response = server().handle(json!(3), "tools/call", params).await;
        assert_eq!(response["id"], 3);
        assert_eq!(response["result"]["isError"], false);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"hello\": 1")
        );
    }

    #[tokio::test]
    async fn a_tool_failure_is_not_a_json_rpc_error() {
        let params = json!({"name": "other", "arguments": {}});
        let response = server().handle(json!(4), "tools/call", params).await;
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("unknown tool: other"));
        assert!(text.contains("see tools/list"));
    }

    #[tokio::test]
    async fn a_call_without_arguments_does_not_blow_up() {
        let response = server()
            .handle(json!(5), "tools/call", json!({"name": "greet"}))
            .await;
        assert_eq!(response["result"]["isError"], false);
    }

    #[tokio::test]
    async fn ping_is_answered_empty() {
        let response = server().handle(json!(6), "ping", json!({})).await;
        assert_eq!(response["result"], json!({}));
    }

    #[tokio::test]
    async fn an_unknown_method_is_a_json_rpc_error() {
        let response = server().handle(json!(7), "sre/unknown", json!({})).await;
        assert_eq!(response["error"]["code"], -32601);
        assert!(response.get("result").is_none());
    }

    #[tokio::test]
    async fn initialize_announces_prompts_and_prompts_list_publishes_them() {
        let init = server().handle(json!(8), "initialize", json!({})).await;
        assert!(init["result"]["capabilities"]["prompts"].is_object());
        let list = server().handle(json!(9), "prompts/list", json!({})).await;
        assert_eq!(list["result"]["prompts"][0]["name"], "hello");
    }

    #[tokio::test]
    async fn prompts_get_renders_the_user_message() {
        let params = json!({"name": "hello", "arguments": {"who": "ops"}});
        let response = server().handle(json!(10), "prompts/get", params).await;
        assert_eq!(
            response["result"]["messages"][0]["content"]["text"],
            "hello ops"
        );
    }

    #[tokio::test]
    async fn an_unknown_prompt_is_an_invalid_params_error() {
        let response = server()
            .handle(json!(11), "prompts/get", json!({"name": "nope"}))
            .await;
        assert_eq!(response["error"]["code"], -32602);
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("unknown prompt: nope")
        );
    }
}
