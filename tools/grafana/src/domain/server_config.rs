use url::Url;

use super::{ConfigError, validate_uid};

/// What the environment says at start: where Grafana is, the service account
/// token, and which Loki and Prometheus datasources to use when a call names
/// none. The token is read once and never written anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    base_url: Url,
    authorization: String,
    loki: Option<String>,
    prometheus: Option<String>,
}

impl ServerConfig {
    pub fn new(
        url: Option<&str>,
        token: Option<&str>,
        loki: Option<&str>,
        prometheus: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let url = url
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::MissingUrl)?;
        let with_scheme = if url.contains("://") {
            url.to_string()
        } else {
            format!("https://{url}")
        };
        let mut base_url =
            Url::parse(&with_scheme).map_err(|error| ConfigError::InvalidUrl(error.to_string()))?;
        let host = base_url
            .host_str()
            .ok_or_else(|| ConfigError::InvalidUrl("no host".into()))?
            .to_string();
        let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
        if base_url.scheme() != "https" && !(base_url.scheme() == "http" && local) {
            return Err(ConfigError::InsecureUrl);
        }
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        base_url.set_query(None);
        base_url.set_fragment(None);
        let token = token
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::MissingToken)?;
        let token = token.strip_prefix("Bearer ").unwrap_or(token).trim();
        let datasource = |value: Option<&str>, name: &'static str| match value
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => Ok(None),
            Some(uid) => validate_uid(uid)
                .map(Some)
                .map_err(|_| ConfigError::InvalidDatasource(name)),
        };
        Ok(Self {
            base_url,
            authorization: format!("Bearer {token}"),
            loki: datasource(loki, "GRAFANA_LOKI_DATASOURCE")?,
            prometheus: datasource(prometheus, "GRAFANA_PROMETHEUS_DATASOURCE")?,
        })
    }

    /// `GRAFANA_URL`, `GRAFANA_TOKEN`, `GRAFANA_LOKI_DATASOURCE`, `GRAFANA_PROMETHEUS_DATASOURCE`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Self::new(
            get("GRAFANA_URL").as_deref(),
            get("GRAFANA_TOKEN").as_deref(),
            get("GRAFANA_LOKI_DATASOURCE").as_deref(),
            get("GRAFANA_PROMETHEUS_DATASOURCE").as_deref(),
        )
    }

    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    #[must_use]
    pub fn authorization(&self) -> &str {
        &self.authorization
    }

    #[must_use]
    pub fn loki(&self) -> Option<&str> {
        self.loki.as_deref()
    }

    #[must_use]
    pub fn prometheus(&self) -> Option<&str> {
        self.prometheus.as_deref()
    }

    /// The server without its token, for `doctor`.
    #[must_use]
    pub fn server_display(&self) -> String {
        self.base_url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_becomes_https_with_a_trailing_slash_and_the_token_a_bearer() {
        let config = ServerConfig::new(
            Some(" grafana.example.com/grafana "),
            Some("abc"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            config.base_url().as_str(),
            "https://grafana.example.com/grafana/"
        );
        assert_eq!(config.authorization(), "Bearer abc");
        assert_eq!(config.loki(), None);
        assert_eq!(
            config.server_display(),
            "https://grafana.example.com/grafana/"
        );
        let bearer = ServerConfig::new(
            Some("https://g.example.com?x=1#f"),
            Some("Bearer abc "),
            Some("loki-1"),
            Some("prom_1"),
        )
        .unwrap();
        assert_eq!(bearer.authorization(), "Bearer abc");
        assert_eq!(bearer.base_url().as_str(), "https://g.example.com/");
        assert_eq!(bearer.loki(), Some("loki-1"));
        assert_eq!(bearer.prometheus(), Some("prom_1"));
    }

    #[test]
    fn http_is_only_for_localhost() {
        assert!(ServerConfig::new(Some("http://localhost:3000"), Some("t"), None, None).is_ok());
        assert!(ServerConfig::new(Some("http://127.0.0.1:3000"), Some("t"), None, None).is_ok());
        assert_eq!(
            ServerConfig::new(Some("http://grafana.example.com"), Some("t"), None, None)
                .unwrap_err(),
            ConfigError::InsecureUrl
        );
        assert_eq!(
            ServerConfig::new(Some("ftp://x"), Some("t"), None, None).unwrap_err(),
            ConfigError::InsecureUrl
        );
    }

    #[test]
    fn every_missing_or_bad_variable_is_named() {
        assert_eq!(
            ServerConfig::new(None, Some("t"), None, None).unwrap_err(),
            ConfigError::MissingUrl
        );
        assert_eq!(
            ServerConfig::new(Some(" "), Some("t"), None, None).unwrap_err(),
            ConfigError::MissingUrl
        );
        assert!(matches!(
            ServerConfig::new(Some("https://"), Some("t"), None, None).unwrap_err(),
            ConfigError::InvalidUrl(_)
        ));
        assert_eq!(
            ServerConfig::new(Some("https://g"), None, None, None).unwrap_err(),
            ConfigError::MissingToken
        );
        assert_eq!(
            ServerConfig::new(Some("https://g"), Some("t"), Some("a b"), None).unwrap_err(),
            ConfigError::InvalidDatasource("GRAFANA_LOKI_DATASOURCE")
        );
        assert_eq!(
            ServerConfig::new(Some("https://g"), Some("t"), None, Some(""))
                .unwrap()
                .prometheus(),
            None
        );
        let from_vars = ServerConfig::from_vars(|name| {
            (name == "GRAFANA_URL").then(|| "https://g".to_string())
        });
        assert_eq!(from_vars.unwrap_err(), ConfigError::MissingToken);
    }
}
