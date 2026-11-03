use thiserror::Error;

/// What is wrong with a call's arguments, said before `aws` runs.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("{0} is required")]
    Empty(&'static str),
    #[error("invalid region `{0}`: such as eu-west-1 or us-east-2")]
    InvalidRegion(String),
    #[error("region `{0}` is not in AWS_MCP_REGIONS: discover lists the regions in scope")]
    RegionNotAllowed(String),
    #[error("region is required: AWS_MCP_REGIONS is not set, so every call names its region")]
    RegionRequired,
    #[error("invalid {what} `{value}`: {rule}")]
    Invalid {
        what: &'static str,
        value: String,
        rule: &'static str,
    },
    #[error(
        "invalid time `{0}`: RFC 3339 such as 2026-11-03T10:00:00Z, or how long ago, such as 15m, 3h or 1d"
    )]
    InvalidTime(String),
    #[error("start must be before end")]
    WindowOrder,
    #[error("the window is longer than {0} days")]
    WindowTooLong(u32),
    #[error("{what} must be between {min} and {max}")]
    OutOfRange {
        what: &'static str,
        min: i64,
        max: i64,
    },
    #[error("period must be a multiple of 60 seconds, 60 to 86400")]
    InvalidPeriod,
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
    #[error("AWS_MCP_REGIONS has an entry that is not a region: {0}")]
    InvalidRegions(String),
    #[error("AWS_MCP_PROFILE is not a profile name: letters, digits, `.`, `_` and `-`")]
    InvalidProfile,
    #[error("AWS_CLI is empty")]
    InvalidCli,
}
