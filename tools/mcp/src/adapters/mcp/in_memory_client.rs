use rmcp::ServiceExt;
use rmcp::model::ProtocolVersion;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use tokio::task::JoinHandle;

use super::tool_host_handler::ToolHostHandler;

const BUFFER_BYTES: usize = 1 << 16;

/// A client over an in-memory pipe, so a tool's contract is tested without a process.
pub(crate) struct InMemoryClient {
    writer: WriteHalf<DuplexStream>,
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    service: JoinHandle<()>,
}

impl InMemoryClient {
    pub(crate) fn connect(handler: ToolHostHandler) -> Self {
        let (client, server) = tokio::io::duplex(BUFFER_BYTES);
        let service = tokio::spawn(async move {
            if let Ok(running) = handler.serve(tokio::io::split(server)).await {
                let _ = running.waiting().await;
            }
        });
        let (reader, writer) = tokio::io::split(client);
        Self {
            writer,
            lines: BufReader::new(reader).lines(),
            service,
        }
    }

    pub(crate) fn initialize_params(params: Value) -> Value {
        let mut merged = json!({
            "protocolVersion": ProtocolVersion::LATEST_WITH_INITIALIZE,
            "capabilities": {},
            "clientInfo": {"name": "sre-mcp", "version": env!("CARGO_PKG_VERSION")},
        });
        if let (Some(target), Value::Object(given)) = (merged.as_object_mut(), params) {
            target.extend(given);
        }
        merged
    }

    pub(crate) async fn request(&mut self, id: Value, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        while let Ok(Some(line)) = self.lines.next_line().await {
            match serde_json::from_str::<Value>(&line) {
                Ok(message) if message.get("id") == Some(&id) => return message,
                _ => {}
            }
        }
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32603, "message": "the MCP server closed the connection without answering"},
        })
    }

    pub(crate) async fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}))
            .await;
    }

    async fn send(&mut self, message: &Value) {
        let mut line = message.to_string();
        line.push('\n');
        let _ = self.writer.write_all(line.as_bytes()).await;
    }
}

impl Drop for InMemoryClient {
    fn drop(&mut self) {
        self.service.abort();
    }
}
