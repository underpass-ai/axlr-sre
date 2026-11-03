//! The tokens a call sends, validated before any of them reaches `kubectl`.
//! None may start with `-`: kubectl has no reliable end of options, so a
//! token that looked like a flag would become one.

use super::InputError;

const NAME_MAX: usize = 253;
const LABEL_MAX: usize = 63;
const SELECTOR_MAX: usize = 512;
const DURATION_MAX: usize = 8;
const CONTEXT_MAX: usize = 253;

fn is_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LABEL_MAX
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

/// A namespace: one DNS label.
pub fn namespace(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    if is_label(value) {
        Ok(value.to_string())
    } else {
        Err(InputError::InvalidNamespace(value.to_string()))
    }
}

/// A resource name: DNS labels joined by dots.
pub fn name(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    if !value.is_empty() && value.len() <= NAME_MAX && value.split('.').all(is_label) {
        Ok(value.to_string())
    } else {
        Err(InputError::InvalidName(value.to_string()))
    }
}

/// A resource kind as kubectl takes it: `pods`, `deploy`, `deployments.apps`.
/// Secrets are refused whatever the spelling.
pub fn kind(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    let shape = !value.is_empty()
        && value.len() <= LABEL_MAX
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    if !shape {
        return Err(InputError::InvalidKind(value.to_string()));
    }
    let resource = value.split('.').next().unwrap_or_default();
    if resource == "secret" || resource == "secrets" {
        return Err(InputError::SecretsRefused);
    }
    Ok(value.to_string())
}

/// A label selector: `app=web,tier in (a,b),!canary`.
pub fn selector(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    let shape = !value.is_empty()
        && value.len() <= SELECTOR_MAX
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " =!,./_-()".contains(c));
    if shape {
        Ok(value.to_string())
    } else {
        Err(InputError::InvalidSelector(value.to_string()))
    }
}

/// A duration as kubectl takes it: `30s`, `5m`, `2h`.
pub fn duration(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    let digits = value.strip_suffix(['s', 'm', 'h']).unwrap_or_default();
    let shape = !digits.is_empty()
        && value.len() <= DURATION_MAX
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0');
    if shape {
        Ok(value.to_string())
    } else {
        Err(InputError::InvalidDuration(value.to_string()))
    }
}

/// A container name: one DNS label.
pub fn container(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    if is_label(value) {
        Ok(value.to_string())
    } else {
        Err(InputError::InvalidName(value.to_string()))
    }
}

/// A kubeconfig context name: what kubeconfig generators produce, such as
/// `arn:aws:eks:eu-west-1:123:cluster/x` or `kind-lab`.
pub fn context(value: &str) -> Option<String> {
    let value = value.trim();
    let shape = !value.is_empty()
        && value.len() <= CONTEXT_MAX
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._:/-".contains(c));
    shape.then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_and_containers_are_dns_labels() {
        assert_eq!(namespace(" prod-eu ").unwrap(), "prod-eu");
        for bad in ["", "Prod", "-a", "a-", "a.b", "a_b", &"a".repeat(64)] {
            assert!(namespace(bad).is_err(), "{bad}");
            assert!(container(bad).is_err(), "{bad}");
        }
        assert_eq!(container("web").unwrap(), "web");
    }

    #[test]
    fn names_are_dns_subdomains() {
        assert_eq!(name("web-7d9f.v2").unwrap(), "web-7d9f.v2");
        for bad in ["", "a..b", "-x", "x/y", "A", "../etc", &"a".repeat(254)] {
            assert!(name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn kinds_are_lowercase_and_never_secrets() {
        for ok in [
            "pods",
            "deploy",
            "deployments.apps",
            "hpa",
            "ingresses.networking.k8s.io",
        ] {
            assert_eq!(kind(ok).unwrap(), ok);
        }
        for bad in ["", "Pods", "-o", "pods/x", "1pods", "a b"] {
            assert!(
                matches!(kind(bad), Err(InputError::InvalidKind(_))),
                "{bad}"
            );
        }
        for secret in ["secret", "secrets", "secrets.core"] {
            assert_eq!(kind(secret), Err(InputError::SecretsRefused));
        }
    }

    #[test]
    fn selectors_keep_to_label_syntax() {
        assert_eq!(
            selector("app=web,tier in (a,b),!canary").unwrap(),
            "app=web,tier in (a,b),!canary"
        );
        for bad in ["", "-l x", "a;b", "a\nb", "a$b", &"a".repeat(513)] {
            assert!(selector(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn durations_are_a_number_and_a_unit() {
        for ok in ["30s", "5m", "2h"] {
            assert_eq!(duration(ok).unwrap(), ok);
        }
        for bad in ["", "s", "30", "05m", "1d", "30 s", "-5m", "100000000s"] {
            assert!(duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn contexts_accept_what_kubeconfig_generators_produce() {
        assert_eq!(
            context("arn:aws:eks:eu-west-1:1:cluster/x").unwrap(),
            "arn:aws:eks:eu-west-1:1:cluster/x"
        );
        assert_eq!(context(" kind-lab ").unwrap(), "kind-lab");
        for bad in ["", "-x", "a b", "a;b"] {
            assert!(context(bad).is_none(), "{bad}");
        }
    }
}
