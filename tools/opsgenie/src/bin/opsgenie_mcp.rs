use std::sync::Arc;

use opsgenie_mcp::adapters::inbound::OpsgenieMcpHost;
use opsgenie_mcp::adapters::outbound::HttpOpsgenieClient;
use opsgenie_mcp::application::{
    ChangeAlerts, DiagnoseSetup, DiscoverScope, InspectAlerts, ReadAlerts,
};
use opsgenie_mcp::domain::ServerConfig;
use opsgenie_mcp::ports::OpsgenieClient;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the HTTP client,
/// the use cases, the host, the server. A bad configuration still serves,
/// so a model can call `doctor` and read what is missing.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = ServerConfig::from_env();
    let host = match &config {
        Ok(config) => match HttpOpsgenieClient::new(config) {
            Ok(client) => {
                let client: Arc<dyn OpsgenieClient> = Arc::new(client);
                let read = Arc::new(ReadAlerts::new(client.clone(), config.scope().clone()));
                OpsgenieMcpHost::new(
                    Some(InspectAlerts::new(client.clone(), read.clone())),
                    Some(ChangeAlerts::new(
                        client.clone(),
                        read.clone(),
                        config.writes(),
                        config.user().map(str::to_string),
                    )),
                    Some(DiscoverScope::new(
                        read.clone(),
                        config.server_display(),
                        config.writes(),
                    )),
                    DiagnoseSetup::new(Ok(config.clone()), Some(client), Some(read)),
                    config.writes(),
                )
            }
            Err(error) => {
                eprintln!("opsgenie-mcp: HTTP client: {error:#}");
                OpsgenieMcpHost::new(
                    None,
                    None,
                    None,
                    DiagnoseSetup::new(Ok(config.clone()), None, None),
                    config.writes(),
                )
            }
        },
        Err(error) => {
            eprintln!("opsgenie-mcp: {error}; only doctor will answer");
            OpsgenieMcpHost::new(
                None,
                None,
                None,
                DiagnoseSetup::new(config, None, None),
                false,
            )
        }
    };
    StdioServer::new(Arc::new(host)).run().await
}
