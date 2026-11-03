use thiserror::Error;

/// What is wrong with a call's arguments, said before any request.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error(
        "invalid time `{0}`: RFC 3339 such as 2026-11-03T10:00:00Z, or how long ago, such as 15m, 2h or 1d"
    )]
    InvalidTime(String),
    #[error("start must be before end")]
    WindowOrder,
    #[error("the window is longer than {0} days")]
    WindowTooLong(u32),
    #[error("{0} is required")]
    Empty(&'static str),
    #[error("{what} is longer than {max} characters")]
    TooLong { what: &'static str, max: usize },
    #[error("a LogQL query starts with a stream selector in braces, such as {{app=\"web\"}}")]
    NotAStreamSelector,
    #[error("invalid label name `{0}`")]
    InvalidLabel(String),
    #[error("invalid datasource uid `{0}`: letters, digits, `-` and `_`; discover lists the uids")]
    InvalidUid(String),
    #[error("{what} must be between {min} and {max}")]
    OutOfRange {
        what: &'static str,
        min: i64,
        max: i64,
    },
    #[error("no {kind} datasource: add one in Grafana, or set {variable} to its uid")]
    NoDatasource {
        kind: &'static str,
        variable: &'static str,
    },
    #[error(
        "more than one {kind} datasource ({names}): name one with `datasource`, or set {variable} to its uid"
    )]
    AmbiguousDatasource {
        kind: &'static str,
        names: String,
        variable: &'static str,
    },
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
    #[error("GRAFANA_URL is not set")]
    MissingUrl,
    #[error("GRAFANA_URL is not a URL: {0}")]
    InvalidUrl(String),
    #[error("GRAFANA_URL must be https unless it is localhost")]
    InsecureUrl,
    #[error("GRAFANA_TOKEN is not set")]
    MissingToken,
    #[error("{0} is not a datasource uid: letters, digits, `-` and `_`")]
    InvalidDatasource(&'static str),
}
