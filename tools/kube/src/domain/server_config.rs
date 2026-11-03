use super::{ConfigError, Policy, tokens};

const DEFAULT_MAX_REPLICAS: u32 = 10;

/// What the environment says at start: the policy, the kubeconfig context
/// and where `kubectl` is. Nothing here is a credential: kubectl reads its
/// own kubeconfig or the ServiceAccount token of the pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    policy: Policy,
    context: Option<String>,
    kubectl: String,
}

impl ServerConfig {
    pub fn new(
        namespaces: Option<&str>,
        writes: Option<&str>,
        max_replicas: Option<&str>,
        context: Option<&str>,
        kubectl: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let namespaces: Vec<String> = namespaces
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|namespace| !namespace.is_empty())
            .map(str::to_string)
            .collect();
        let writes = match writes.map(str::trim).filter(|value| !value.is_empty()) {
            None => false,
            Some("true") => true,
            Some("false") => false,
            Some(other) => return Err(ConfigError::InvalidWrites(other.to_string())),
        };
        let max_replicas = match max_replicas
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            None => DEFAULT_MAX_REPLICAS,
            Some(value) => value
                .parse::<u32>()
                .ok()
                .filter(|value| (1..=1000).contains(value))
                .ok_or_else(|| ConfigError::InvalidMaxReplicas(value.to_string()))?,
        };
        let context = match context.map(str::trim).filter(|value| !value.is_empty()) {
            None => None,
            Some(value) => Some(
                tokens::context(value)
                    .ok_or_else(|| ConfigError::InvalidContext(value.to_string()))?,
            ),
        };
        let kubectl = match kubectl.map(str::trim) {
            None | Some("") => "kubectl".to_string(),
            Some(path) if path.starts_with('-') => return Err(ConfigError::InvalidKubectl),
            Some(path) => path.to_string(),
        };
        let policy = Policy::new(namespaces, writes, max_replicas)
            .map_err(|error| ConfigError::InvalidNamespaces(error.to_string()))?;
        Ok(Self {
            policy,
            context,
            kubectl,
        })
    }

    /// `KUBE_NAMESPACES`, `KUBE_WRITES`, `KUBE_MAX_REPLICAS`, `KUBE_CONTEXT`, `KUBECTL`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Self::new(
            get("KUBE_NAMESPACES").as_deref(),
            get("KUBE_WRITES").as_deref(),
            get("KUBE_MAX_REPLICAS").as_deref(),
            get("KUBE_CONTEXT").as_deref(),
            get("KUBECTL").as_deref(),
        )
    }

    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    #[must_use]
    pub fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }

    #[must_use]
    pub fn kubectl(&self) -> &str {
        &self.kubectl
    }

    /// One line for `doctor`: mode, namespaces, context.
    #[must_use]
    pub fn summary(&self) -> String {
        let namespaces = if self.policy.namespaces().is_empty() {
            "any namespace, named on every call".to_string()
        } else {
            format!("namespaces {}", self.policy.namespaces().join(", "))
        };
        format!(
            "{}, {}, context {}, kubectl {}",
            if self.policy.writes() {
                format!("read-write (replicas up to {})", self.policy.max_replicas())
            } else {
                "read-only".to_string()
            },
            namespaces,
            self.context.as_deref().unwrap_or("of the kubeconfig"),
            self.kubectl
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_environment_is_a_read_only_server_over_the_kubeconfig() {
        let config = ServerConfig::from_vars(|_| None).unwrap();
        assert!(!config.policy().writes());
        assert!(config.policy().namespaces().is_empty());
        assert_eq!(config.context(), None);
        assert_eq!(config.kubectl(), "kubectl");
        assert!(config.summary().starts_with("read-only, any namespace"));
    }

    #[test]
    fn the_variables_are_read_and_validated() {
        let config = ServerConfig::from_vars(|name| {
            Some(
                match name {
                    "KUBE_NAMESPACES" => "prod, staging,",
                    "KUBE_WRITES" => "true",
                    "KUBE_MAX_REPLICAS" => "20",
                    "KUBE_CONTEXT" => "kind-lab",
                    "KUBECTL" => "/usr/local/bin/kubectl",
                    _ => return None,
                }
                .to_string(),
            )
        })
        .unwrap();
        assert_eq!(config.policy().namespaces(), ["prod", "staging"]);
        assert!(config.policy().writes());
        assert_eq!(config.policy().max_replicas(), 20);
        assert_eq!(config.context(), Some("kind-lab"));
        assert_eq!(config.kubectl(), "/usr/local/bin/kubectl");
        assert!(config.summary().contains("read-write (replicas up to 20)"));
        assert!(config.summary().contains("context kind-lab"));
    }

    #[test]
    fn every_bad_variable_is_named() {
        let bad = |ns, w, m, c, k| ServerConfig::new(ns, w, m, c, k).unwrap_err();
        assert!(matches!(
            bad(Some("Bad"), None, None, None, None),
            ConfigError::InvalidNamespaces(_)
        ));
        assert_eq!(
            bad(None, Some("yes"), None, None, None),
            ConfigError::InvalidWrites("yes".into())
        );
        assert_eq!(
            bad(None, None, Some("0"), None, None),
            ConfigError::InvalidMaxReplicas("0".into())
        );
        assert_eq!(
            bad(None, None, Some("x"), None, None),
            ConfigError::InvalidMaxReplicas("x".into())
        );
        assert_eq!(
            bad(None, None, None, Some("a b"), None),
            ConfigError::InvalidContext("a b".into())
        );
        assert_eq!(
            bad(None, None, None, None, Some("--bad")),
            ConfigError::InvalidKubectl
        );
        assert!(ServerConfig::new(None, Some(" "), Some(""), Some(""), Some("")).is_ok());
    }
}
