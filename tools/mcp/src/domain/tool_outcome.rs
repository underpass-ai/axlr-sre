use serde_json::Value;

/// What a tool call came back with.
///
/// A failure is not a JSON-RPC error: the server answers it as a result with
/// `isError`, so the model reads the error and the hint and can go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutcome {
    Ok(Value),
    Failed { error: String, hint: Option<String> },
}

impl ToolOutcome {
    #[must_use]
    pub fn failed(error: impl Into<String>, hint: Option<String>) -> Self {
        Self::Failed {
            error: error.into(),
            hint,
        }
    }

    #[must_use]
    pub fn failed_with_hint(error: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
            hint: Some(hint.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_keeps_the_error_and_the_hint() {
        match ToolOutcome::failed("broken", Some("retry".to_string())) {
            ToolOutcome::Failed { error, hint } => {
                assert_eq!(error, "broken");
                assert_eq!(hint.as_deref(), Some("retry"));
            }
            ToolOutcome::Ok(_) => panic!("should be a failure"),
        }
    }

    #[test]
    fn a_failure_with_hint_needs_no_option() {
        assert_eq!(
            ToolOutcome::failed_with_hint("broken", "retry"),
            ToolOutcome::failed("broken", Some("retry".into()))
        );
    }
}
