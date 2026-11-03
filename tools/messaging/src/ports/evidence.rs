use std::path::Path;

use anyhow::Result;
use serde_json::Value;

/// Where a drain keeps what it deletes: a file opened once per drain, with
/// a header line and one line per message, durable on disk before the
/// batch it holds is deleted from the queue.
pub trait EvidenceStore: Send + Sync {
    fn start(&self, queue_name: &str, header: &Value) -> Result<Box<dyn EvidenceSink>>;
}

pub trait EvidenceSink: Send {
    /// Appends the lines and makes them durable.
    fn append(&mut self, lines: &[Value]) -> Result<()>;
    fn path(&self) -> &Path;
}
