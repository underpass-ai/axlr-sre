use thiserror::Error;

/// What is wrong with a call's arguments or its statement, said before
/// anything reaches the database.
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
    #[error("one statement per call: the text holds {0}")]
    MultipleStatements(usize),
    #[error("the statement is not closed: an open {0}")]
    Unterminated(&'static str),
    #[error(
        "`{0}` is not allowed in a read: it acts on the server; terminate is the one door for backends"
    )]
    DeniedFunction(String),
    #[error(
        "`{0}` is not a read: query takes SELECT, WITH, TABLE, VALUES, EXPLAIN or SHOW; execute takes the rest"
    )]
    NotARead(String),
    #[error(
        "writes are off: register postgres-mcp with POSTGRES_WRITES=maintenance or all to publish execute and terminate"
    )]
    WritesDisabled,
    #[error(
        "`{kind}` needs POSTGRES_WRITES=all; maintenance allows VACUUM, ANALYZE, REINDEX, CREATE INDEX CONCURRENTLY and ALTER (not ALTER SYSTEM)"
    )]
    NeedsAll { kind: String },
    #[error(
        "CREATE INDEX at the maintenance level needs CONCURRENTLY: a plain CREATE INDEX locks the table"
    )]
    IndexNotConcurrent,
    #[error("`{0}` is a read: use query")]
    IsARead(String),
    #[error("unknown {what} `{value}`: one of {allowed}")]
    UnknownChoice {
        what: &'static str,
        value: String,
        allowed: &'static str,
    },
}

/// What is wrong with the environment the server started with.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("no connection: set DATABASE_URL, or PGHOST, PGDATABASE and PGUSER")]
    MissingConnection,
    #[error("DATABASE_URL is not a postgres URL: {0}")]
    InvalidUrl(String),
    #[error("{0} is not valid: {1}")]
    Invalid(&'static str, String),
    #[error("PGSSLMODE must be disable, prefer, require or verify-full")]
    InvalidSslMode,
    #[error("POSTGRES_WRITES must be off, maintenance or all")]
    InvalidWrites,
}
