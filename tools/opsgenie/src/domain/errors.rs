use thiserror::Error;

/// What is wrong with a call's arguments or with the alert it names, said
/// before anything is written.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("{0} is required")]
    Empty(&'static str),
    #[error("{what} is longer than {max} characters")]
    TooLong { what: &'static str, max: usize },
    #[error("invalid alert `{0}`: a tiny id such as 123 or #123, or the alert id")]
    InvalidAlert(String),
    #[error("{what} must be between {min} and {max}")]
    OutOfRange {
        what: &'static str,
        min: i64,
        max: i64,
    },
    #[error("alert #{0} is not in the scope: discover shows the teams and escalations")]
    OutOfScope(String),
    #[error("alert #{alert} is {status}, not open")]
    NotOpen { alert: String, status: String },
    #[error("writes are off: register opsgenie-mcp with OPSGENIE_WRITES=true to publish them")]
    WritesDisabled,
    #[error("the scope names {0} that this Opsgenie does not have: check the spelling, or use ids")]
    UnknownScope(String),
}

/// What is wrong with the environment the server started with.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("OPSGENIE_URL is not a URL: {0}")]
    InvalidUrl(String),
    #[error("OPSGENIE_URL must be https unless it is localhost")]
    InsecureUrl,
    #[error("OPSGENIE_API_KEY is not set")]
    MissingApiKey,
    #[error(
        "no scope: set OPSGENIE_TEAMS, OPSGENIE_ESCALATIONS or both; without one every alert of the account would be in"
    )]
    EmptyScope,
    #[error("{0} has an entry longer than 100 characters")]
    InvalidScope(&'static str),
    #[error("OPSGENIE_WRITES must be true or false")]
    InvalidWrites,
    #[error("OPSGENIE_USER is longer than 100 characters")]
    InvalidUser,
}

/// What is wrong with the mirror daemon's own variables.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MirrorConfigError {
    #[error("{0} is not set")]
    Missing(&'static str),
    #[error("{0} is not an identifier: letters, digits, `.`, `_` and `-`, 64 at most")]
    InvalidId(&'static str),
    #[error("OPSGENIE_MIRROR_INTERVAL must be a number of seconds between {min} and {max}")]
    InvalidInterval { min: u64, max: u64 },
    #[error("OPSGENIE_MIRROR_STATE is not a path")]
    InvalidState,
}
