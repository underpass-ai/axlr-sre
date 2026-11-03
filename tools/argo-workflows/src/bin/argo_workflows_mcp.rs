use std::sync::Arc;

use argo_workflows_mcp::adapters::inbound::ArgoMcpHost;
use argo_workflows_mcp::adapters::outbound::HttpArgoReader;
use argo_workflows_mcp::application::{DiagnoseSetup, DiscoverScope, InspectArgo};
use argo_workflows_mcp::domain::ServerConfig;
use argo_workflows_mcp::ports::ArgoReader;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the HTTP reader, the
/// use cases, the host, the server. A bad configuration still serves, so a
/// model can call `doctor` and read what is missing.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = ServerConfig::from_env();
    let reader: Option<Arc<dyn ArgoReader>> = match &config {
        Ok(config) => match HttpArgoReader::new(config) {
            Ok(reader) => Some(Arc::new(reader)),
            Err(error) => {
                eprintln!("argo-workflows-mcp: HTTP client: {error:#}");
                None
            }
        },
        Err(error) => {
            eprintln!("argo-workflows-mcp: {error}; only doctor will answer");
            None
        }
    };
    let namespace = config.as_ref().ok().map(|c| c.namespace().to_string());
    let host = ArgoMcpHost::new(
        reader.clone().map(InspectArgo::new),
        reader
            .clone()
            .zip(namespace.clone())
            .map(|(reader, namespace)| DiscoverScope::new(reader, namespace)),
        DiagnoseSetup::new(config, reader),
        namespace,
    );
    StdioServer::new(Arc::new(host)).run().await
}
