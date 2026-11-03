use anyhow::Result;
use async_trait::async_trait;

/// What one statement left: rows (every cell text), or a command tag with
/// its count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Option<String>>>,
    },
    Command {
        tag: String,
        affected: Option<u64>,
    },
}

/// One connection to the database. Each call opens one, runs what the use
/// case asks, statement by statement, and drops it: no pool, no state
/// between calls.
#[async_trait]
pub trait SqlSession: Send {
    /// Runs one statement with the simple protocol and gives back what it
    /// left. A statement that fails is an `Err` with the server's message.
    async fn run(&mut self, sql: &str) -> Result<Outcome>;
}

#[async_trait]
pub trait SqlGateway: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn SqlSession>>;
}
