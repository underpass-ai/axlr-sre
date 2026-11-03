//! Every string that reaches an `aws` argument is checked here first. No
//! token starts with `-`, so none can become a flag.

use super::InputError;

const NAME_MAX: usize = 256;

fn plain(
    what: &'static str,
    value: &str,
    max: usize,
    rule: &'static str,
    allowed: impl Fn(char) -> bool,
) -> Result<String, InputError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(InputError::Empty(what));
    }
    if value.starts_with('-') || value.chars().count() > max || !value.chars().all(allowed) {
        return Err(InputError::Invalid {
            what,
            value: value.chars().take(60).collect(),
            rule,
        });
    }
    Ok(value.to_string())
}

/// `eu-west-1`, `us-gov-west-1`, `cn-north-1`: two letters, words, a digit.
pub fn region(value: &str) -> Result<String, InputError> {
    let value = value.trim().to_ascii_lowercase();
    let parts: Vec<&str> = value.split('-').collect();
    let ok = parts.len() >= 3
        && parts[0].len() == 2
        && parts[..parts.len() - 1]
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase()))
        && parts[parts.len() - 1].chars().all(|c| c.is_ascii_digit())
        && !parts[parts.len() - 1].is_empty();
    if ok {
        Ok(value)
    } else {
        Err(InputError::InvalidRegion(value.chars().take(60).collect()))
    }
}

/// A Lambda function name, or its ARN.
pub fn function_name(value: &str) -> Result<String, InputError> {
    plain(
        "function",
        value,
        170,
        "a function name or ARN: letters, digits, `-`, `_`, `:` and `.`",
        |c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'),
    )
}

/// A CloudWatch Logs group name.
pub fn log_group(value: &str) -> Result<String, InputError> {
    plain(
        "log_group",
        value,
        512,
        "letters, digits, `/`, `.`, `-`, `_`, `#`",
        |c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '#'),
    )
}

/// A metric namespace, such as `AWS/Lambda`.
pub fn namespace(value: &str) -> Result<String, InputError> {
    plain(
        "namespace",
        value,
        NAME_MAX,
        "such as AWS/Lambda, AWS/SQS or a custom one",
        |c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '#' | ':'),
    )
}

pub fn metric_name(value: &str) -> Result<String, InputError> {
    plain(
        "metric",
        value,
        NAME_MAX,
        "such as Errors or CPUUtilization",
        |c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/'),
    )
}

/// A dimension name and value, both bounded and without a leading dash.
pub fn dimension(name: &str, value: &str) -> Result<(String, String), InputError> {
    let name = plain(
        "dimension name",
        name,
        NAME_MAX,
        "letters, digits, `.`, `-`, `_`, `:` and `/`",
        |c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/'),
    )?;
    let value = plain(
        "dimension value",
        value,
        1024,
        "printable text without a leading dash",
        |c| !c.is_control(),
    )?;
    Ok((name, value))
}

pub const STATISTICS: &str = "Sum, Average, Minimum, Maximum, SampleCount or p50..p99.9";

/// `Sum`, `Average`, `Minimum`, `Maximum`, `SampleCount` or a percentile `pNN`.
pub fn statistic(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    let known = ["Sum", "Average", "Minimum", "Maximum", "SampleCount"];
    if let Some(found) = known.iter().find(|k| k.eq_ignore_ascii_case(value)) {
        return Ok(found.to_string());
    }
    let percentile = value
        .strip_prefix('p')
        .filter(|rest| {
            !rest.is_empty()
                && rest.len() <= 5
                && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
                && rest
                    .parse::<f64>()
                    .is_ok_and(|p| (0.0..=100.0).contains(&p))
        })
        .is_some();
    if percentile {
        Ok(value.to_string())
    } else {
        Err(InputError::UnknownChoice {
            what: "stat",
            value: value.chars().take(20).collect(),
            allowed: STATISTICS,
        })
    }
}

/// Seconds per point: a multiple of 60, 60 to 86400.
pub fn period(value: i64) -> Result<i64, InputError> {
    if (60..=86_400).contains(&value) && value % 60 == 0 {
        Ok(value)
    } else {
        Err(InputError::InvalidPeriod)
    }
}

/// A prefix of alarm names.
pub fn alarm_prefix(value: &str) -> Result<String, InputError> {
    plain(
        "prefix",
        value,
        NAME_MAX,
        "printable text without a leading dash",
        |c| !c.is_control(),
    )
}

/// A CloudWatch Logs filter pattern: free text, bounded, no leading dash.
pub fn filter_pattern(value: &str) -> Result<String, InputError> {
    plain(
        "filter",
        value,
        1024,
        "a filter pattern such as ERROR or \"Task timed out\"",
        |c| !c.is_control(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_have_a_shape() {
        for ok in [
            "eu-west-1",
            " US-EAST-2 ",
            "us-gov-west-1",
            "cn-north-1",
            "ap-southeast-3",
        ] {
            assert!(region(ok).is_ok(), "{ok}");
        }
        assert_eq!(region(" US-EAST-2 ").unwrap(), "us-east-2");
        for bad in [
            "",
            "eu",
            "eu-west",
            "eu-west-",
            "europe-1",
            "eu-west-1; rm",
            "-eu-west-1",
        ] {
            assert!(
                matches!(region(bad), Err(InputError::InvalidRegion(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn names_refuse_a_leading_dash_and_control_characters() {
        assert_eq!(function_name(" my-fn ").unwrap(), "my-fn");
        assert!(function_name("arn:aws:lambda:eu-west-1:123456789012:function:f:live").is_ok());
        assert_eq!(
            function_name("").unwrap_err(),
            InputError::Empty("function")
        );
        assert!(function_name("--help").is_err());
        assert!(function_name("a b").is_err());
        assert!(log_group("/aws/lambda/my-fn").is_ok());
        assert!(log_group("/aws/lambda/my fn").is_err());
        assert!(namespace("AWS/Lambda").is_ok());
        assert!(namespace("Custom/My.App").is_ok());
        assert!(namespace("AWS Lambda").is_err());
        assert!(metric_name("Errors").is_ok());
        assert!(metric_name("-x").is_err());
        assert_eq!(
            dimension("FunctionName", "my-fn").unwrap(),
            ("FunctionName".into(), "my-fn".into())
        );
        assert!(dimension("Function Name", "x").is_err());
        assert!(dimension("FunctionName", "-x").is_err());
        assert!(dimension("FunctionName", "a\nb").is_err());
        assert!(alarm_prefix("prod ").is_ok());
        assert!(alarm_prefix("-x").is_err());
        assert!(filter_pattern("\"Task timed out\"").is_ok());
        assert!(filter_pattern(&"x".repeat(1025)).is_err());
    }

    #[test]
    fn statistics_and_periods_are_checked() {
        assert_eq!(statistic("sum").unwrap(), "Sum");
        assert_eq!(statistic("p99").unwrap(), "p99");
        assert_eq!(statistic("p99.9").unwrap(), "p99.9");
        for bad in ["p", "p101", "median", "pxx", "p1.2.3"] {
            assert!(statistic(bad).is_err(), "{bad}");
        }
        assert_eq!(period(300).unwrap(), 300);
        assert_eq!(period(86_400).unwrap(), 86_400);
        for bad in [0, 30, 61, 90_000] {
            assert_eq!(period(bad).unwrap_err(), InputError::InvalidPeriod);
        }
    }
}
