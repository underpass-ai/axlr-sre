use std::sync::Arc;

use aws_mcp::adapters::outbound::ProcessRunner;
use aws_mcp::ports::AwsRunner;
use messaging_mcp::adapters::inbound::MessagingMcpHost;
use messaging_mcp::adapters::outbound::JsonlEvidence;
use messaging_mcp::application::{
    ChangeQueues, DiagnoseSetup, DiscoverScope, InspectQueues, QueueLookup,
};
use messaging_mcp::domain::MessagingConfig;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the process runner
/// of `aws-mcp`, the evidence store, the use cases, the host, the server. A
/// bad configuration still serves, so a model can call `doctor`.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = MessagingConfig::from_env();
    let host = match &config {
        Ok(config) => {
            let runner: Arc<dyn AwsRunner> = Arc::new(ProcessRunner::new(
                config.aws().cli(),
                config.aws().profile().map(str::to_string),
            ));
            let lookup = Arc::new(QueueLookup::new(
                runner.clone(),
                config.aws().profile().map(str::to_string),
            ));
            let regions = config.aws().regions().clone();
            MessagingMcpHost::new(
                Some(InspectQueues::new(lookup.clone(), regions.clone())),
                Some(ChangeQueues::new(
                    lookup.clone(),
                    regions,
                    config.writes(),
                    Arc::new(JsonlEvidence::new(config.snapshot_dir().clone())),
                )),
                Some(DiscoverScope::new(lookup, config.clone())),
                DiagnoseSetup::new(Ok(config.clone()), Some(runner)),
                config.writes(),
            )
        }
        Err(error) => {
            eprintln!("messaging-mcp: {error}; only doctor will answer");
            MessagingMcpHost::new(None, None, None, DiagnoseSetup::new(config, None), false)
        }
    };
    StdioServer::new(Arc::new(host)).run().await
}
