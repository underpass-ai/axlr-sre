use std::sync::Arc;

use aws_mcp::adapters::inbound::AwsMcpHost;
use aws_mcp::adapters::outbound::ProcessRunner;
use aws_mcp::application::{DiagnoseSetup, DiscoverScope, InspectLambda, QueryLogs, QueryMetrics};
use aws_mcp::domain::ServerConfig;
use aws_mcp::ports::AwsRunner;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the process runner,
/// the use cases, the host, the server. A bad configuration still serves,
/// so a model can call `doctor` and read what is wrong.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = ServerConfig::from_env();
    let host = match &config {
        Ok(config) => {
            let runner: Arc<dyn AwsRunner> = Arc::new(ProcessRunner::new(
                config.cli(),
                config.profile().map(str::to_string),
            ));
            let regions = config.regions().clone();
            let profile = config.profile().map(str::to_string);
            AwsMcpHost::new(
                Some(QueryMetrics::new(
                    runner.clone(),
                    regions.clone(),
                    profile.clone(),
                )),
                Some(InspectLambda::new(
                    runner.clone(),
                    regions.clone(),
                    profile.clone(),
                )),
                Some(QueryLogs::new(runner.clone(), regions, profile)),
                Some(DiscoverScope::new(runner.clone(), config.clone())),
                DiagnoseSetup::new(Ok(config.clone()), Some(runner)),
            )
        }
        Err(error) => {
            eprintln!("aws-mcp: {error}; only doctor will answer");
            AwsMcpHost::new(None, None, None, None, DiagnoseSetup::new(config, None))
        }
    };
    StdioServer::new(Arc::new(host)).run().await
}
