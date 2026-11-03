use std::sync::Arc;

use redis_mcp::adapters::inbound::RedisMcpHost;
use redis_mcp::adapters::outbound::RedisCrateGateway;
use redis_mcp::application::{
    ChangeInstance, DiagnoseSetup, DiscoverScope, InspectInstance, InspectKeys,
};
use redis_mcp::domain::ServerConfig;
use redis_mcp::ports::RedisGateway;
use sre_mcp::StdioServer;

/// Composition: the configuration from the environment, the gateway, the
/// use cases, the host, the server. A bad configuration still serves, so a
/// model can call `doctor` and read what is wrong.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    // One crypto provider for the whole process, before any TLS client is built.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = ServerConfig::from_env();
    let host = match &config {
        Ok(config) => {
            let gateway: Arc<dyn RedisGateway> =
                Arc::new(RedisCrateGateway::new(config.connection().clone()));
            RedisMcpHost::new(
                Some(InspectInstance::new(gateway.clone())),
                Some(InspectKeys::new(gateway.clone())),
                Some(ChangeInstance::new(gateway.clone(), config.writes())),
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
            eprintln!("redis-mcp: {error}; only doctor will answer");
            RedisMcpHost::new(
                None,
                None,
                None,
                None,
                DiagnoseSetup::new(config, None),
                false,
            )
        }
    };
    StdioServer::new(Arc::new(host)).run().await
}
