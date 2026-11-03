use std::sync::Arc;

use postgres_mcp::adapters::inbound::PostgresMcpHost;
use postgres_mcp::adapters::outbound::TokioPostgresGateway;
use postgres_mcp::application::{
    ChangeServer, DiagnoseSetup, DiscoverScope, InspectServer, RunQuery,
};
use postgres_mcp::domain::{ServerConfig, WriteLevel};
use postgres_mcp::ports::SqlGateway;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the gateway, the
/// use cases, the host, the server. A bad configuration still serves, so a
/// model can call `doctor` and read what is wrong.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = ServerConfig::from_env();
    let host = match &config {
        Ok(config) => {
            let gateway: Arc<dyn SqlGateway> =
                Arc::new(TokioPostgresGateway::new(config.connection().clone()));
            PostgresMcpHost::new(
                Some(RunQuery::new(gateway.clone())),
                Some(InspectServer::new(gateway.clone())),
                Some(ChangeServer::new(gateway.clone(), config.writes())),
                Some(DiscoverScope::new(
                    gateway.clone(),
                    config.writes(),
                    config.connection().display(),
                )),
                DiagnoseSetup::new(Ok(config.clone()), Some(gateway)),
                config.writes(),
            )
        }
        Err(error) => {
            eprintln!("postgres-mcp: {error}; only doctor will answer");
            PostgresMcpHost::new(
                None,
                None,
                None,
                None,
                DiagnoseSetup::new(config, None),
                WriteLevel::Off,
            )
        }
    };
    StdioServer::new(Arc::new(host)).run().await
}
