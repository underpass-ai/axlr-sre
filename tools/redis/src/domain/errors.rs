use thiserror::Error;

/// What is wrong with a call's arguments, said before anything reaches the
/// server.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("{0} is required")]
    Empty(&'static str),
    #[error("{what} must be between {min} and {max}")]
    OutOfRange {
        what: &'static str,
        min: i64,
        max: i64,
    },
    #[error("invalid {what} `{value}`: {rule}")]
    Invalid {
        what: &'static str,
        value: String,
        rule: &'static str,
    },
    #[error("pattern `{0}` selects every key: narrow it, or add a value filter")]
    PatternTooWide(String),
    #[error(
        "writes are off: register redis-mcp with REDIS_WRITES=true to publish expire, delete, config_set and client_kill"
    )]
    WritesDisabled,
    #[error("`{0}` is not a parameter config_set may change; the allowed ones: {1}")]
    ParameterNotAllowed(String, &'static str),
    #[error("unknown {what} `{value}`: one of {allowed}")]
    UnknownChoice {
        what: &'static str,
        value: String,
        allowed: &'static str,
    },
    #[error("the reply was not what the command answers: {0}")]
    UnexpectedReply(&'static str),
}

/// What is wrong with the environment the server started with.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("no connection: set REDIS_URL, or REDIS_HOST")]
    MissingConnection,
    #[error("REDIS_URL is not a redis URL: {0}")]
    InvalidUrl(String),
    #[error("{0} is not valid: {1}")]
    Invalid(&'static str, String),
    #[error("REDIS_TLS must be off, on or insecure")]
    InvalidTls,
    #[error("REDIS_WRITES must be true or false")]
    InvalidWrites,
}
