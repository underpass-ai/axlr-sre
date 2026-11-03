use anyhow::Result;
use async_trait::async_trait;

use crate::domain::Reply;

/// One connection to the instance. Each call opens one, runs what the use
/// case asks and drops it: no pool, no state between calls.
#[async_trait]
pub trait RedisSession: Send {
    /// One command, its arguments already validated. An error reply is an
    /// `Err` with the server's message.
    async fn command(&mut self, args: &[String]) -> Result<Reply>;

    /// Several commands in one round trip; one reply each, in order.
    async fn pipeline(&mut self, commands: &[Vec<String>]) -> Result<Vec<Reply>>;
}

#[async_trait]
pub trait RedisGateway: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn RedisSession>>;
}
