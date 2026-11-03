use url::Url;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error(
        "ARGO_SERVER is not set: the Argo Server address, such as https://argo.example.com or argo.example.com:2746"
    )]
    MissingServer,
    #[error("ARGO_SERVER is not a valid address: {0}")]
    InvalidServer(String),
    #[error("ARGO_SERVER must use https; http is accepted only for localhost")]
    InsecureServer,
    #[error(
        "ARGO_TOKEN is not set: the bearer token of a service account that may read the namespace"
    )]
    MissingToken,
    #[error("ARGO_NAMESPACE is not a Kubernetes name")]
    InvalidNamespace,
}

/// Where Argo Server is and how to speak to it. Nothing in it is a default of
/// any installation: every value comes from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    base_url: Url,
    namespace: String,
    token: String,
}

const DEFAULT_NAMESPACE: &str = "argo";

impl ServerConfig {
    /// `server` is a URL or `host[:port]`; `token` with or without the `Bearer ` prefix;
    /// `namespace` empty means Argo's own default, `argo`.
    pub fn new(
        server: Option<&str>,
        namespace: Option<&str>,
        token: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let server = server
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(ConfigError::MissingServer)?;
        let with_scheme = if server.contains("://") {
            server.to_string()
        } else {
            format!("https://{server}")
        };
        let mut base_url =
            Url::parse(&with_scheme).map_err(|e| ConfigError::InvalidServer(e.to_string()))?;
        let host = base_url
            .host_str()
            .ok_or_else(|| ConfigError::InvalidServer("no host".into()))?
            .to_string();
        let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
        match base_url.scheme() {
            "https" => {}
            "http" if loopback => {}
            "http" => return Err(ConfigError::InsecureServer),
            other => return Err(ConfigError::InvalidServer(format!("scheme {other}"))),
        }
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        base_url.set_query(None);
        base_url.set_fragment(None);
        let token = token
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or(ConfigError::MissingToken)?;
        let token = match token.strip_prefix("Bearer ") {
            Some(bare) => format!("Bearer {}", bare.trim()),
            None => format!("Bearer {token}"),
        };
        let namespace = namespace
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(DEFAULT_NAMESPACE);
        let valid = namespace.len() <= 63
            && namespace
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !namespace.starts_with('-')
            && !namespace.ends_with('-');
        if !valid {
            return Err(ConfigError::InvalidNamespace);
        }
        Ok(Self {
            base_url,
            namespace: namespace.to_string(),
            token,
        })
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        let read = |name: &str| std::env::var(name).ok();
        Self::new(
            read("ARGO_SERVER").as_deref(),
            read("ARGO_NAMESPACE").as_deref(),
            read("ARGO_TOKEN").as_deref(),
        )
    }

    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The `Authorization` header value. Never shown by `doctor`.
    #[must_use]
    pub fn authorization(&self) -> &str {
        &self.token
    }

    /// The server as `doctor` may show it: scheme, host and port, no credentials.
    #[must_use]
    pub fn server_display(&self) -> String {
        let mut shown = self.base_url.clone();
        let _ = shown.set_username("");
        let _ = shown.set_password(None);
        shown.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_becomes_https_with_a_trailing_slash() {
        let config = ServerConfig::new(Some("argo.example.com:2746"), None, Some("abc")).unwrap();
        assert_eq!(config.base_url().as_str(), "https://argo.example.com:2746/");
        assert_eq!(config.namespace(), "argo");
        assert_eq!(config.authorization(), "Bearer abc");
    }

    #[test]
    fn a_bearer_prefix_is_kept_once_and_a_path_is_honoured() {
        let config = ServerConfig::new(
            Some("https://ops.example.com/argo?x=1"),
            Some(" ci "),
            Some("Bearer  abc "),
        )
        .unwrap();
        assert_eq!(config.base_url().as_str(), "https://ops.example.com/argo/");
        assert_eq!(config.namespace(), "ci");
        assert_eq!(config.authorization(), "Bearer abc");
        assert_eq!(config.server_display(), "https://ops.example.com/argo/");
    }

    #[test]
    fn http_is_only_for_localhost() {
        assert!(ServerConfig::new(Some("http://localhost:2746"), None, Some("t")).is_ok());
        assert!(ServerConfig::new(Some("http://127.0.0.1:2746"), None, Some("t")).is_ok());
        assert_eq!(
            ServerConfig::new(Some("http://argo.example.com"), None, Some("t")),
            Err(ConfigError::InsecureServer)
        );
    }

    #[test]
    fn missing_or_bad_values_are_named() {
        assert_eq!(
            ServerConfig::new(None, None, Some("t")),
            Err(ConfigError::MissingServer)
        );
        assert_eq!(
            ServerConfig::new(Some(" "), None, Some("t")),
            Err(ConfigError::MissingServer)
        );
        assert_eq!(
            ServerConfig::new(Some("argo.example.com"), None, None),
            Err(ConfigError::MissingToken)
        );
        assert_eq!(
            ServerConfig::new(Some("argo.example.com"), Some("Bad_NS"), Some("t")),
            Err(ConfigError::InvalidNamespace)
        );
        assert!(matches!(
            ServerConfig::new(Some("ftp://argo.example.com"), None, Some("t")),
            Err(ConfigError::InvalidServer(_))
        ));
        assert!(matches!(
            ServerConfig::new(Some("https://"), None, Some("t")),
            Err(ConfigError::InvalidServer(_))
        ));
    }

    #[test]
    fn the_display_never_carries_userinfo() {
        let config = ServerConfig::new(
            Some("https://user:secret@argo.example.com"),
            None,
            Some("t"),
        )
        .unwrap();
        assert_eq!(config.server_display(), "https://argo.example.com/");
    }
}
