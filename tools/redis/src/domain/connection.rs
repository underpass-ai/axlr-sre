use std::fmt;

use url::Url;

use super::ConfigError;

const PORT_DEFAULT: u16 = 6379;
const CONNECT_TIMEOUT_DEFAULT: u64 = 5;

/// How the connection is encrypted: not at all, with the certificate
/// checked against the system roots, or encrypted without a check (a
/// tunnel to localhost, a self-signed server).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tls {
    Off,
    On,
    Insecure,
}

impl Tls {
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "false" | "0" | "no" | "" => Ok(Self::Off),
            "on" | "true" | "1" | "yes" | "verify" => Ok(Self::On),
            "insecure" | "noverify" => Ok(Self::Insecure),
            _ => Err(ConfigError::InvalidTls),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Insecure => "insecure",
        }
    }
}

/// How to reach the one instance: `REDIS_URL`, else the variables. The
/// password is read once and never written anywhere; `Debug` and
/// `display` hide it.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub db: i64,
    pub username: Option<String>,
    password: Option<String>,
    pub tls: Tls,
    pub connect_timeout: u64,
}

impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConnectionConfig({})", self.display())
    }
}

impl ConnectionConfig {
    /// `REDIS_URL` (`redis://user:password@host:6379/0`, `rediss://` for
    /// TLS, `#insecure` to skip the check), else `REDIS_HOST`, `REDIS_PORT`,
    /// `REDIS_DB`, `REDIS_USERNAME`, `REDIS_PASSWORD` (or `REDISCLI_AUTH`),
    /// `REDIS_TLS`; `REDIS_CONNECT_TIMEOUT` in seconds.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let var = |name: &str| {
            get(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut config = match var("REDIS_URL") {
            Some(url) => Self::from_url(&url)?,
            None => {
                let host = var("REDIS_HOST").ok_or(ConfigError::MissingConnection)?;
                let port = match var("REDIS_PORT") {
                    Some(port) => port
                        .parse::<u16>()
                        .map_err(|_| ConfigError::Invalid("REDIS_PORT", port))?,
                    None => PORT_DEFAULT,
                };
                Self {
                    host,
                    port,
                    db: 0,
                    username: None,
                    password: None,
                    tls: Tls::Off,
                    connect_timeout: CONNECT_TIMEOUT_DEFAULT,
                }
            }
        };
        if let Some(db) = var("REDIS_DB") {
            config.db = db
                .parse::<i64>()
                .ok()
                .filter(|d| (0..=15).contains(d))
                .ok_or(ConfigError::Invalid("REDIS_DB", db))?;
        }
        if let Some(user) = var("REDIS_USERNAME") {
            config.username = Some(user);
        }
        if let Some(password) = var("REDIS_PASSWORD").or_else(|| var("REDISCLI_AUTH")) {
            config.password = Some(password);
        }
        if let Some(tls) = var("REDIS_TLS") {
            config.tls = Tls::parse(&tls)?;
        }
        if let Some(timeout) = var("REDIS_CONNECT_TIMEOUT") {
            config.connect_timeout = timeout
                .parse::<u64>()
                .ok()
                .filter(|t| (1..=60).contains(t))
                .ok_or(ConfigError::Invalid("REDIS_CONNECT_TIMEOUT", timeout))?;
        }
        Ok(config)
    }

    fn from_url(raw: &str) -> Result<Self, ConfigError> {
        let url = Url::parse(raw).map_err(|e| ConfigError::InvalidUrl(e.to_string()))?;
        let tls = match url.scheme() {
            "redis" | "valkey" => Tls::Off,
            "rediss" | "valkeys" => {
                if url.fragment() == Some("insecure") {
                    Tls::Insecure
                } else {
                    Tls::On
                }
            }
            other => return Err(ConfigError::InvalidUrl(format!("scheme {other}"))),
        };
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| ConfigError::InvalidUrl("empty host".into()))?
            .trim_matches(['[', ']'])
            .to_string();
        let db = match url.path().trim_start_matches('/') {
            "" => 0,
            path => path
                .parse::<i64>()
                .ok()
                .filter(|d| (0..=15).contains(d))
                .ok_or_else(|| ConfigError::InvalidUrl(format!("database `{path}`")))?,
        };
        let username = Some(percent_decode(url.username())).filter(|u| !u.is_empty());
        let password = url.password().map(percent_decode).filter(|p| !p.is_empty());
        Ok(Self {
            host,
            port: url.port().unwrap_or(PORT_DEFAULT),
            db,
            username,
            password,
            tls,
            connect_timeout: CONNECT_TIMEOUT_DEFAULT,
        })
    }

    #[must_use]
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// `user@host:port/db (tls …)`, never the password.
    #[must_use]
    pub fn display(&self) -> String {
        format!(
            "{}{}:{}/{} (tls {})",
            self.username
                .as_deref()
                .map(|u| format!("{u}@"))
                .unwrap_or_default(),
            self.host,
            self.port,
            self.db,
            self.tls.as_str()
        )
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                16,
            )
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn the_variables_build_a_connection_and_hide_the_password() {
        let config = ConnectionConfig::from_vars(vars(&[
            ("REDIS_HOST", "cache.internal"),
            ("REDISCLI_AUTH", "s3cret"),
            ("REDIS_TLS", "insecure"),
            ("REDIS_DB", "2"),
        ]))
        .unwrap();
        assert_eq!(config.port, 6379);
        assert_eq!(config.db, 2);
        assert_eq!(config.password(), Some("s3cret"));
        assert_eq!(config.tls, Tls::Insecure);
        assert_eq!(config.display(), "cache.internal:6379/2 (tls insecure)");
        assert!(!format!("{config:?}").contains("s3cret"));
        let full = ConnectionConfig::from_vars(vars(&[
            ("REDIS_HOST", "h"),
            ("REDIS_PORT", "6380"),
            ("REDIS_USERNAME", "bot"),
            ("REDIS_PASSWORD", "p"),
            ("REDIS_TLS", "on"),
            ("REDIS_CONNECT_TIMEOUT", "2"),
        ]))
        .unwrap();
        assert_eq!(full.display(), "bot@h:6380/0 (tls on)");
        assert_eq!(full.connect_timeout, 2);
    }

    #[test]
    fn a_url_is_read_and_the_variables_refine_it() {
        let config = ConnectionConfig::from_vars(vars(&[(
            "REDIS_URL",
            "rediss://bot:p%40ss@cache.internal:6380/3#insecure",
        )]))
        .unwrap();
        assert_eq!(config.username.as_deref(), Some("bot"));
        assert_eq!(config.password(), Some("p@ss"));
        assert_eq!(config.db, 3);
        assert_eq!(config.tls, Tls::Insecure);
        let plain = ConnectionConfig::from_vars(vars(&[
            ("REDIS_URL", "redis://h"),
            ("REDIS_PASSWORD", "x"),
        ]))
        .unwrap();
        assert_eq!(plain.tls, Tls::Off);
        assert_eq!(plain.password(), Some("x"));
        assert_eq!(plain.username, None);
        assert_eq!(
            ConnectionConfig::from_vars(vars(&[("REDIS_URL", "valkeys://h:1/0")]))
                .unwrap()
                .tls,
            Tls::On
        );
    }

    #[test]
    fn every_bad_variable_is_named() {
        for (pairs, expected) in [
            (vec![], ConfigError::MissingConnection),
            (
                vec![("REDIS_HOST", "h"), ("REDIS_PORT", "x")],
                ConfigError::Invalid("REDIS_PORT", "x".into()),
            ),
            (
                vec![("REDIS_HOST", "h"), ("REDIS_DB", "16")],
                ConfigError::Invalid("REDIS_DB", "16".into()),
            ),
            (
                vec![("REDIS_HOST", "h"), ("REDIS_TLS", "maybe")],
                ConfigError::InvalidTls,
            ),
            (
                vec![("REDIS_HOST", "h"), ("REDIS_CONNECT_TIMEOUT", "0")],
                ConfigError::Invalid("REDIS_CONNECT_TIMEOUT", "0".into()),
            ),
            (
                vec![("REDIS_URL", "http://h")],
                ConfigError::InvalidUrl("scheme http".into()),
            ),
            (
                vec![("REDIS_URL", "redis://")],
                ConfigError::InvalidUrl("empty host".into()),
            ),
            (
                vec![("REDIS_URL", "redis://h/99")],
                ConfigError::InvalidUrl("database `99`".into()),
            ),
            (
                vec![("REDIS_URL", "::")],
                ConfigError::InvalidUrl("relative URL without a base".into()),
            ),
        ] {
            assert_eq!(
                ConnectionConfig::from_vars(vars(&pairs)).unwrap_err(),
                expected,
                "{pairs:?}"
            );
        }
        assert_eq!(Tls::parse("verify").unwrap().as_str(), "on");
        assert_eq!(percent_decode("a%2Fb%zz"), "a/b%zz");
    }
}
