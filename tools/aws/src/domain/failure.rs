use serde_json::{Value, json};

const DETAIL_MAX: usize = 300;

/// What `aws` said when it failed, turned into a short reason and the action
/// that fixes it. The CLI's stderr has two shapes: a service error,
/// `An error occurred (Code) when calling the Op operation: text`, and prose
/// about credentials and connectivity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub detail: String,
    pub hint: Option<String>,
    pub code: Option<String>,
}

impl Failure {
    #[must_use]
    pub fn diagnose(stderr: &str, profile: Option<&str>) -> Self {
        let text = stderr.trim();
        let (code, operation, message) = service_error(text);
        let has = |needle: &str| text.contains(needle);
        let login = || match profile {
            Some(profile) => format!("aws sso login --profile {profile}"),
            None => "aws sso login, or give the node fresh credentials".to_string(),
        };
        let (detail, hint) = if has("could not be found") && has("config profile") {
            (
                format!(
                    "the profile `{}` is not in the AWS config",
                    profile.unwrap_or("?")
                ),
                Some("set AWS_MCP_PROFILE to a profile of the AWS config, or unset it".into()),
            )
        } else if has("Unable to locate credentials") || has("NoCredentialProviders") {
            (
                "no AWS credentials".to_string(),
                Some("set AWS_MCP_PROFILE to a profile, or give the node a role (IRSA)".into()),
            )
        } else if has("Token has expired")
            || has("has expired")
            || has("ExpiredToken")
            || has("Error when retrieving token from sso")
            || has("The SSO session")
        {
            ("the session has expired".to_string(), Some(login()))
        } else if has("Could not connect to the endpoint URL") || has("Connect timeout") {
            (
                "the endpoint could not be reached".to_string(),
                Some("check the region and the network of this host".into()),
            )
        } else if let Some(code) = &code {
            let detail = format!(
                "{code} on {}: {}",
                operation.as_deref().unwrap_or("?"),
                bounded(message.as_deref().unwrap_or(""))
            );
            let hint = if code.starts_with("AccessDenied")
                || code == "UnauthorizedOperation"
                || code.starts_with("Unauthorized")
                || code == "UnrecognizedClientException"
                || code == "InvalidClientTokenId"
            {
                Some("the identity lacks this permission: call doctor to see who the CLI is".into())
            } else if code.starts_with("Throttling")
                || code.starts_with("TooManyRequests")
                || code == "RequestLimitExceeded"
                || code == "LimitExceededException"
            {
                Some("rate limited: ask for less, or wait and call again".into())
            } else if code.starts_with("ResourceNotFound")
                || code == "NoSuchEntity"
                || code.ends_with("NotFound")
                || code.ends_with("NotFoundException")
            {
                Some("check the exact name and the region".into())
            } else if code.starts_with("InvalidParameter") || code == "ValidationError" {
                Some("check the arguments; the message says which".into())
            } else {
                None
            };
            (detail, hint)
        } else {
            (bounded(text), None)
        };
        Self { detail, hint, code }
    }

    /// `{"error", "hint"}` for an answer.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"error": self.detail, "hint": self.hint})
    }
}

fn bounded(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= DETAIL_MAX {
        flat
    } else {
        let mut cut: String = flat.chars().take(DETAIL_MAX - 1).collect();
        cut.push('…');
        cut
    }
}

/// `(code, operation, message)` of a service error line, when there is one.
fn service_error(text: &str) -> (Option<String>, Option<String>, Option<String>) {
    const MARK: &str = "An error occurred (";
    let Some(start) = text.find(MARK) else {
        return (None, None, None);
    };
    let rest = &text[start + MARK.len()..];
    let Some(close) = rest.find(')') else {
        return (None, None, None);
    };
    let code = rest[..close].trim().to_string();
    let after = &rest[close + 1..];
    let operation = after
        .split_once("when calling the ")
        .and_then(|(_, tail)| tail.split_once(" operation"))
        .map(|(op, _)| op.trim().to_string());
    let message = after
        .split_once("operation:")
        .map(|(_, msg)| msg.trim().to_string())
        .filter(|m| !m.is_empty());
    (Some(code), operation, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPIRED: &str =
        "Error when retrieving token from sso: Token has expired and refresh failed";
    const NO_PROFILE: &str = "The config profile (demo) could not be found";
    const NO_CREDS: &str =
        "Unable to locate credentials. You can configure credentials by running \"aws configure\".";
    const DENIED: &str = "An error occurred (AccessDeniedException) when calling the ListFunctions operation: User: arn:aws:sts::123456789012:assumed-role/demo-ro/x is not authorized to perform: lambda:ListFunctions on resource: *";
    const THROTTLED: &str = "An error occurred (ThrottlingException) when calling the GetMetricData operation (reached max retries: 2): Rate exceeded";
    const MISSING: &str = "\nAn error occurred (ResourceNotFoundException) when calling the GetFunctionConfiguration operation: Function not found: arn:aws:lambda:eu-west-1:123456789012:function:ghost\n";
    const ENDPOINT: &str =
        "Could not connect to the endpoint URL: \"https://lambda.eu-west-9.amazonaws.com/\"";

    #[test]
    fn credentials_and_sessions_are_told_apart() {
        let expired = Failure::diagnose(EXPIRED, Some("demo"));
        assert_eq!(expired.detail, "the session has expired");
        assert_eq!(
            expired.hint.as_deref(),
            Some("aws sso login --profile demo")
        );
        assert!(
            Failure::diagnose(EXPIRED, None)
                .hint
                .unwrap()
                .contains("fresh credentials")
        );
        let no_profile = Failure::diagnose(NO_PROFILE, Some("demo"));
        assert!(
            no_profile
                .detail
                .contains("`demo` is not in the AWS config")
        );
        assert!(no_profile.hint.unwrap().contains("AWS_MCP_PROFILE"));
        let no_creds = Failure::diagnose(NO_CREDS, None);
        assert_eq!(no_creds.detail, "no AWS credentials");
        assert!(no_creds.hint.unwrap().contains("IRSA"));
        assert_eq!(no_creds.code, None);
    }

    #[test]
    fn service_errors_carry_code_operation_and_a_bounded_message() {
        let denied = Failure::diagnose(DENIED, None);
        assert_eq!(denied.code.as_deref(), Some("AccessDeniedException"));
        assert!(
            denied
                .detail
                .starts_with("AccessDeniedException on ListFunctions: User:")
        );
        assert!(denied.hint.unwrap().contains("doctor"));
        let throttled = Failure::diagnose(THROTTLED, None);
        assert!(throttled.detail.contains("GetMetricData"));
        assert!(throttled.hint.unwrap().contains("rate limited"));
        let missing = Failure::diagnose(MISSING, None);
        assert!(missing.detail.contains("Function not found"));
        assert!(missing.hint.unwrap().contains("exact name"));
        let endpoint = Failure::diagnose(ENDPOINT, None);
        assert!(endpoint.hint.unwrap().contains("region"));
        let invalid = Failure::diagnose(
            "An error occurred (InvalidParameterValue) when calling the GetMetricData operation: bad period",
            None,
        );
        assert!(invalid.hint.unwrap().contains("arguments"));
        let other = Failure::diagnose(
            "An error occurred (InternalFailure) when calling the X operation",
            None,
        );
        assert_eq!(other.hint, None);
        assert_eq!(other.detail, "InternalFailure on X: ");
        assert_eq!(other.to_json()["error"], "InternalFailure on X: ");
    }

    #[test]
    fn unknown_text_is_bounded_and_without_a_hint() {
        let long = Failure::diagnose(&"x ".repeat(500), None);
        assert!(long.detail.chars().count() <= DETAIL_MAX);
        assert!(long.detail.ends_with('…'));
        assert_eq!(long.hint, None);
        let odd = Failure::diagnose("An error occurred (broken", None);
        assert_eq!(odd.code, None);
        assert_eq!(Failure::diagnose("", None).detail, "");
    }
}
