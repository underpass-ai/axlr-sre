use super::InputError;

const KEY_MAX: usize = 512;

/// A key name: printable, 512 bytes at most, no leading dash.
pub fn key_name(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(InputError::Empty("key"));
    }
    if value.len() > KEY_MAX || value.chars().any(char::is_control) || value.starts_with('-') {
        return Err(InputError::Invalid {
            what: "key",
            value: value.chars().take(60).collect(),
            rule: "printable text, 512 bytes at most",
        });
    }
    Ok(value.to_string())
}

/// A SCAN MATCH pattern. Wildcards alone (`*`, `**`, `*:*`, `?*`) select
/// every key; such a pattern needs a value filter beside it.
pub fn key_pattern(value: Option<&str>, has_value_filter: bool) -> Result<String, InputError> {
    let pattern = value
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or("*");
    if pattern.len() > KEY_MAX || pattern.chars().any(char::is_control) || pattern.starts_with('-')
    {
        return Err(InputError::Invalid {
            what: "pattern",
            value: pattern.chars().take(60).collect(),
            rule: "a MATCH glob such as session:* or demo:lock:?",
        });
    }
    let only_wildcards = pattern
        .chars()
        .all(|c| matches!(c, '*' | '?' | ':' | '-' | '_' | '.'));
    if only_wildcards && !has_value_filter {
        return Err(InputError::PatternTooWide(pattern.to_string()));
    }
    Ok(pattern.to_string())
}

/// Keys are kept when their value, as previewed, contains the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueFilter {
    needle: String,
    ignore_case: bool,
}

impl ValueFilter {
    pub fn parse(contains: Option<&str>, ignore_case: bool) -> Result<Option<Self>, InputError> {
        match contains.map(str::trim).filter(|c| !c.is_empty()) {
            None => Ok(None),
            Some(needle) if needle.len() <= 1024 => Ok(Some(Self {
                needle: if ignore_case {
                    needle.to_lowercase()
                } else {
                    needle.to_string()
                },
                ignore_case,
            })),
            Some(needle) => Err(InputError::Invalid {
                what: "value_contains",
                value: needle.chars().take(60).collect(),
                rule: "1024 characters at most",
            }),
        }
    }

    #[must_use]
    pub fn matches(&self, value: &str) -> bool {
        if self.ignore_case {
            value.to_lowercase().contains(&self.needle)
        } else {
            value.contains(&self.needle)
        }
    }

    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "value contains `{}`{}",
            self.needle,
            if self.ignore_case {
                " (ignoring case)"
            } else {
                ""
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_patterns_are_checked() {
        assert_eq!(key_name(" demo:lock:1 ").unwrap(), "demo:lock:1");
        assert_eq!(key_name("").unwrap_err(), InputError::Empty("key"));
        assert!(key_name("-x").is_err());
        assert!(key_name("a\nb").is_err());
        assert!(key_name(&"k".repeat(513)).is_err());
        assert_eq!(key_pattern(Some("demo:*"), false).unwrap(), "demo:*");
        assert_eq!(key_pattern(None, true).unwrap(), "*");
        for wide in ["*", "**", "*:*", "?*", " "] {
            assert!(
                matches!(
                    key_pattern(Some(wide), false).unwrap_err(),
                    InputError::PatternTooWide(_)
                ),
                "{wide}"
            );
        }
        assert!(key_pattern(Some("-x"), false).is_err());
        assert!(key_pattern(Some("a\tb"), false).is_err());
    }

    #[test]
    fn the_value_filter_matches_with_or_without_case() {
        let filter = ValueFilter::parse(Some("Pending"), false).unwrap().unwrap();
        assert!(filter.matches("status=Pending"));
        assert!(!filter.matches("status=pending"));
        let loose = ValueFilter::parse(Some("Pending"), true).unwrap().unwrap();
        assert!(loose.matches("status=PENDING"));
        assert!(loose.describe().contains("ignoring case"));
        assert_eq!(ValueFilter::parse(Some(" "), false).unwrap(), None);
        assert!(ValueFilter::parse(Some(&"n".repeat(1025)), false).is_err());
    }
}
