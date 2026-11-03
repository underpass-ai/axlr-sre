use std::sync::Arc;

use grafana_mcp::adapters::inbound::GrafanaMcpHost;
use grafana_mcp::adapters::outbound::HttpGrafanaReader;
use grafana_mcp::application::{
    Datasources, DiagnoseSetup, DiscoverScope, InspectGrafana, QueryLogs, QueryMetrics,
};
use grafana_mcp::domain::ServerConfig;
use grafana_mcp::ports::GrafanaReader;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the HTTP reader, the
/// use cases, the host, the server. A bad configuration still serves, so a
/// model can call `doctor` and read what is missing.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = ServerConfig::from_env();
    let host = match &config {
        Ok(config) => match HttpGrafanaReader::new(config) {
            Ok(reader) => {
                let reader: Arc<dyn GrafanaReader> = Arc::new(reader);
                let datasources = Arc::new(Datasources::new(
                    reader.clone(),
                    config.loki().map(str::to_string),
                    config.prometheus().map(str::to_string),
                ));
                GrafanaMcpHost::new(
                    Some(QueryLogs::new(reader.clone(), datasources.clone())),
                    Some(QueryMetrics::new(reader.clone(), datasources.clone())),
                    Some(InspectGrafana::new(reader.clone())),
                    Some(DiscoverScope::new(reader.clone(), datasources)),
                    DiagnoseSetup::new(Ok(config.clone()), Some(reader)),
                )
            }
            Err(error) => {
                eprintln!("grafana-mcp: HTTP client: {error:#}");
                GrafanaMcpHost::new(
                    None,
                    None,
                    None,
                    None,
                    DiagnoseSetup::new(Ok(config.clone()), None),
                )
            }
        },
        Err(error) => {
            eprintln!("grafana-mcp: {error}; only doctor will answer");
            GrafanaMcpHost::new(None, None, None, None, DiagnoseSetup::new(config, None))
        }
    };
    StdioServer::new(Arc::new(host)).run().await
}
