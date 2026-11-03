use super::{ConfigError, ConnectionConfig};

/// What the environment says at start: the connection and whether the
/// writes are published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    connection: ConnectionConfig,
    writes: bool,
}

impl ServerConfig {
    pub fn new(connection: ConnectionConfig, writes: bool) -> Self {
        Self { connection, writes }
    }

    /// The connection variables plus `REDIS_WRITES`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let writes = match get("REDIS_WRITES")
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref()
        {
            None | Some("") | Some("false") | Some("0") | Some("no") => false,
            Some("true") | Some("1") | Some("yes") => true,
            Some(_) => return Err(ConfigError::InvalidWrites),
        };
        Ok(Self {
            connection: ConnectionConfig::from_vars(&get)?,
            writes,
        })
    }

    #[must_use]
    pub fn connection(&self) -> &ConnectionConfig {
        &self.connection
    }

    #[must_use]
    pub fn writes(&self) -> bool {
        self.writes
    }

    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{}, writes {}",
            self.connection.display(),
            if self.writes { "on" } else { "off" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_are_read_with_the_connection() {
        let config = ServerConfig::from_vars(|name| match name {
            "REDIS_HOST" => Some("h".into()),
            "REDIS_WRITES" => Some("TRUE".into()),
            _ => None,
        })
        .unwrap();
        assert!(config.writes());
        assert_eq!(config.summary(), "h:6379/0 (tls off), writes on");
        assert_eq!(config.connection().host, "h");
        assert!(
            !ServerConfig::from_vars(|name| (name == "REDIS_HOST").then(|| "h".into()))
                .unwrap()
                .writes()
        );
        assert_eq!(
            ServerConfig::from_vars(|name| match name {
                "REDIS_HOST" => Some("h".into()),
                "REDIS_WRITES" => Some("maybe".into()),
                _ => None,
            })
            .unwrap_err(),
            ConfigError::InvalidWrites
        );
        assert_eq!(
            ServerConfig::from_vars(|_| None).unwrap_err(),
            ConfigError::MissingConnection
        );
    }
}
