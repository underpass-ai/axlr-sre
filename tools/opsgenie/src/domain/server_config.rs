use url::Url;

use super::{ConfigError, Scope};

const DEFAULT_URL: &str = "https://api.opsgenie.com";
const USER_MAX: usize = 100;

/// What the environment says at start: Opsgenie's API host, the API key,
/// the scope, whether writes are published and the user the writes are
/// signed as. The key is read once and never written anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    base_url: Url,
    authorization: String,
    scope: Scope,
    writes: bool,
    user: Option<String>,
}

impl ServerConfig {
    pub fn new(
        url: Option<&str>,
        api_key: Option<&str>,
        teams: Option<&str>,
        escalations: Option<&str>,
        writes: Option<&str>,
        user: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let url = url
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_URL);
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
        let key = api_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::MissingApiKey)?;
        let key = key.strip_prefix("GenieKey ").unwrap_or(key).trim();
        let scope = Scope::new(teams, escalations)?;
        let writes = match writes
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("") | Some("false") | Some("0") | Some("no") => false,
            Some("true") | Some("1") | Some("yes") => true,
            Some(_) => return Err(ConfigError::InvalidWrites),
        };
        let user = user
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if user.as_ref().is_some_and(|u| u.chars().count() > USER_MAX) {
            return Err(ConfigError::InvalidUser);
        }
        Ok(Self {
            base_url,
            authorization: format!("GenieKey {key}"),
            scope,
            writes,
            user,
        })
    }

    /// `OPSGENIE_URL` (the US host when unset; `https://api.eu.opsgenie.com` for an EU
    /// account), `OPSGENIE_API_KEY`, `OPSGENIE_TEAMS`, `OPSGENIE_ESCALATIONS`,
    /// `OPSGENIE_WRITES`, `OPSGENIE_USER`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Self::new(
            get("OPSGENIE_URL").as_deref(),
            get("OPSGENIE_API_KEY").as_deref(),
            get("OPSGENIE_TEAMS").as_deref(),
            get("OPSGENIE_ESCALATIONS").as_deref(),
            get("OPSGENIE_WRITES").as_deref(),
            get("OPSGENIE_USER").as_deref(),
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
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    #[must_use]
    pub fn writes(&self) -> bool {
        self.writes
    }

    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The host without the key, for `doctor`.
    #[must_use]
    pub fn server_display(&self) -> String {
        self.base_url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(url: Option<&str>, key: Option<&str>) -> Result<ServerConfig, ConfigError> {
        ServerConfig::new(url, key, Some("platform"), None, None, None)
    }

    #[test]
    fn the_us_host_is_the_default_and_the_key_becomes_a_geniekey() {
        let us = config(None, Some(" abc ")).unwrap();
        assert_eq!(us.base_url().as_str(), "https://api.opsgenie.com/");
        assert_eq!(us.authorization(), "GenieKey abc");
        assert_eq!(us.server_display(), "https://api.opsgenie.com/");
        assert!(!us.writes());
        assert_eq!(us.user(), None);
        let eu = config(Some("api.eu.opsgenie.com?x#y"), Some("GenieKey abc")).unwrap();
        assert_eq!(eu.base_url().as_str(), "https://api.eu.opsgenie.com/");
        assert_eq!(eu.authorization(), "GenieKey abc");
    }

    #[test]
    fn writes_and_user_are_read() {
        let config = ServerConfig::new(
            None,
            Some("k"),
            None,
            Some("critical"),
            Some("TRUE"),
            Some(" sre-bot "),
        )
        .unwrap();
        assert!(config.writes());
        assert_eq!(config.user(), Some("sre-bot"));
        assert_eq!(config.scope().escalations(), ["critical"]);
        assert_eq!(
            ServerConfig::new(None, Some("k"), Some("t"), None, Some("maybe"), None).unwrap_err(),
            ConfigError::InvalidWrites
        );
        assert_eq!(
            ServerConfig::new(
                None,
                Some("k"),
                Some("t"),
                None,
                None,
                Some(&"u".repeat(101))
            )
            .unwrap_err(),
            ConfigError::InvalidUser
        );
    }

    #[test]
    fn http_is_only_for_localhost() {
        assert!(config(Some("http://localhost:8080"), Some("k")).is_ok());
        assert_eq!(
            config(Some("http://opsgenie.example.com"), Some("k")).unwrap_err(),
            ConfigError::InsecureUrl
        );
        assert!(matches!(
            config(Some("https://"), Some("k")).unwrap_err(),
            ConfigError::InvalidUrl(_)
        ));
    }

    #[test]
    fn every_missing_piece_is_named() {
        assert_eq!(config(None, None).unwrap_err(), ConfigError::MissingApiKey);
        assert_eq!(
            ServerConfig::new(None, Some("k"), None, None, None, None).unwrap_err(),
            ConfigError::EmptyScope
        );
        let from_vars =
            ServerConfig::from_vars(|name| (name == "OPSGENIE_API_KEY").then(|| "k".to_string()));
        assert_eq!(from_vars.unwrap_err(), ConfigError::EmptyScope);
        let from_vars = ServerConfig::from_vars(|name| match name {
            "OPSGENIE_API_KEY" => Some("k".into()),
            "OPSGENIE_TEAMS" => Some("a, b".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(from_vars.scope().teams(), ["a", "b"]);
    }
}
