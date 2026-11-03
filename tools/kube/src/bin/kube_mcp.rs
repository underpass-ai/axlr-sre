use std::sync::Arc;

use kube_mcp::adapters::inbound::KubeMcpHost;
use kube_mcp::adapters::outbound::ProcessRunner;
use kube_mcp::application::{ChangeCluster, DiagnoseSetup, DiscoverScope, InspectCluster};
use kube_mcp::domain::ServerConfig;
use kube_mcp::ports::KubectlRunner;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the process runner,
/// the use cases, the host, the server. A bad configuration still serves, so
/// a model can call `doctor` and read what is wrong.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = ServerConfig::from_env();
    let host = match &config {
        Ok(config) => {
            let runner: Arc<dyn KubectlRunner> = Arc::new(ProcessRunner::new(
                config.kubectl(),
                config.context().map(str::to_string),
            ));
            let policy = config.policy().clone();
            KubeMcpHost::new(
                Some(InspectCluster::new(runner.clone())),
                policy
                    .writes()
                    .then(|| ChangeCluster::new(runner.clone(), policy.clone())),
                Some(DiscoverScope::new(runner.clone(), policy.clone())),
                DiagnoseSetup::new(Ok(config.clone()), Some(runner)),
                Some(policy),
            )
        }
        Err(error) => {
            eprintln!("kube-mcp: {error}; only doctor will answer");
            KubeMcpHost::new(None, None, None, DiagnoseSetup::new(config, None), None)
        }
    };
    StdioServer::new(Arc::new(host)).run().await
}
