use thiserror::Error;

/// What is wrong with a call's arguments or with the queue it names, said
/// before anything runs or is written.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error(transparent)]
    Aws(#[from] aws_mcp::domain::InputError),
    #[error("{0} is required")]
    Empty(&'static str),
    #[error("invalid {what} `{value}`: {rule}")]
    Invalid {
        what: &'static str,
        value: String,
        rule: &'static str,
    },
    #[error("{what} must be between {min} and {max}")]
    OutOfRange {
        what: &'static str,
        min: i64,
        max: i64,
    },
    #[error("the queue URL is in {url_region} but the call runs in {region}")]
    RegionMismatch { url_region: String, region: String },
    #[error("writes are off: register messaging-mcp with MESSAGING_WRITES=true to publish them")]
    WritesDisabled,
    #[error(
        "queue {0} has a redrive policy: a receive counts as a delivery attempt and can move its messages to the dead-letter queue; look at the dead-letter queue instead"
    )]
    SourceQueuePeek(String),
    #[error("queue {0} is not a dead-letter queue: no queue sends its failed messages to it")]
    NotDeadLetter(String),
    #[error("{what}: a FIFO destination needs {field} on every message, or a default")]
    FifoNeeds {
        what: &'static str,
        field: &'static str,
    },
    #[error("{0}: a standard destination takes no message_group_id or message_deduplication_id")]
    NotFifo(&'static str),
    #[error("a message move task is already {status} on this queue: {handle}")]
    TaskRunning { status: String, handle: String },
}

/// What is wrong with the environment the server started with.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error(transparent)]
    Aws(#[from] aws_mcp::domain::ConfigError),
    #[error("MESSAGING_WRITES must be true or false")]
    InvalidWrites,
    #[error("MESSAGING_SNAPSHOT_DIR is not a path")]
    InvalidSnapshotDir,
}
