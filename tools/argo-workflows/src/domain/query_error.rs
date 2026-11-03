#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QueryError {
    #[error(
        "{field} is not a Kubernetes name: lowercase letters, digits, '-' and '.', 253 characters at most"
    )]
    InvalidName { field: &'static str },
    #[error("limit must be between 1 and 100")]
    InvalidLimit,
    #[error("tail_lines must be between 1 and 200")]
    InvalidTailLines,
    #[error("continue is longer than 4096 characters")]
    LongCursor,
}
