use std::fmt;
use std::path::{Path, PathBuf};

use url::Url;

use super::{ConfigError, pgpass_lookup};

const PORT_DEFAULT: u16 = 5432;
const CONNECT_TIMEOUT_DEFAULT: u64 = 10;
const APPLICATION_NAME: &str = "postgres-mcp";

/// `PGSSLMODE` as libpq means it: `prefer` and `require` encrypt without
/// checking the certificate, `verify-full` checks it against the roots and
/// the host name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyFull,
}

impl SslMode {
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disable" => Ok(Self::Disable),
            "prefer" | "allow" | "" => Ok(Self::Prefer),
            "require" | "verify-ca" => Ok(Self::Require),
            "verify-full" => Ok(Self::VerifyFull),
            _ => Err(ConfigError::InvalidSslMode),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyFull => "verify-full",
        }
    }
}

/// Where the password came from, for `doctor`; never the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordSource {
    Url,
    Environment,
    PasswordFile,
    None,
    /// A password file that libpq would ignore: readable by group or others.
    FileTooOpen,
}

/// What files the configuration reads: the password file. Injected so the
/// domain stays testable; the real one reads the disk.
pub trait Files {
    /// The contents and the permission bits of a file, if it exists.
    fn read(&self, path: &Path) -> Option<(String, u32)>;
}

/// How to reach the one database: `DATABASE_URL`, else the libpq
/// variables. The password is read once and never written anywhere;
/// `Debug` and `display` hide it.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    password: Option<String>,
    pub password_source: PasswordSource,
    pub ssl_mode: SslMode,
    pub root_cert: Option<PathBuf>,
    pub connect_timeout: u64,
    pub application_name: String,
}

impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConnectionConfig({})", self.display())
    }
}

impl ConnectionConfig {
    pub fn from_vars(
        get: impl Fn(&str) -> Option<String>,
        files: &dyn Files,
    ) -> Result<Self, ConfigError> {
        let var = |name: &str| {
            get(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut config = match var("DATABASE_URL") {
            Some(url) => Self::from_url(&url)?,
            None => {
                let (Some(host), Some(database), Some(user)) =
                    (var("PGHOST"), var("PGDATABASE"), var("PGUSER"))
                else {
                    return Err(ConfigError::MissingConnection);
                };
                let port = match var("PGPORT") {
                    Some(port) => port
                        .parse::<u16>()
                        .map_err(|_| ConfigError::Invalid("PGPORT", port))?,
                    None => PORT_DEFAULT,
                };
                let password = var("PGPASSWORD");
                Self {
                    host,
                    port,
                    database,
                    user,
                    password_source: if password.is_some() {
                        PasswordSource::Environment
                    } else {
                        PasswordSource::None
                    },
                    password,
                    ssl_mode: SslMode::Prefer,
                    root_cert: None,
                    connect_timeout: CONNECT_TIMEOUT_DEFAULT,
                    application_name: APPLICATION_NAME.into(),
                }
            }
        };
        if let Some(mode) = var("PGSSLMODE") {
            config.ssl_mode = SslMode::parse(&mode)?;
        }
        if let Some(cert) = var("PGSSLROOTCERT") {
            config.root_cert = Some(PathBuf::from(cert));
        }
        if let Some(timeout) = var("PGCONNECT_TIMEOUT") {
            config.connect_timeout = timeout
                .parse::<u64>()
                .ok()
                .filter(|t| (1..=300).contains(t))
                .ok_or(ConfigError::Invalid("PGCONNECT_TIMEOUT", timeout))?;
        }
        if let Some(name) = var("PGAPPNAME") {
            config.application_name = name.chars().take(63).collect();
        }
        if config.password.is_none() {
            let path = var("PGPASSFILE")
                .map(PathBuf::from)
                .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".pgpass")));
            if let Some((text, mode)) = path.as_deref().and_then(|p| files.read(p)) {
                if mode & 0o077 != 0 {
                    config.password_source = PasswordSource::FileTooOpen;
                } else if let Some(password) = pgpass_lookup(
                    &text,
                    &config.host,
                    config.port,
                    &config.database,
                    &config.user,
                ) {
                    config.password = Some(password);
                    config.password_source = PasswordSource::PasswordFile;
                }
            }
        }
        Ok(config)
    }

    fn from_url(raw: &str) -> Result<Self, ConfigError> {
        let url = Url::parse(raw).map_err(|e| ConfigError::InvalidUrl(e.to_string()))?;
        if !matches!(url.scheme(), "postgres" | "postgresql") {
            return Err(ConfigError::InvalidUrl(format!("scheme {}", url.scheme())));
        }
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| ConfigError::InvalidUrl("no host".into()))?
            .trim_matches(['[', ']'])
            .to_string();
        let database = url.path().trim_start_matches('/').to_string();
        if database.is_empty() {
            return Err(ConfigError::InvalidUrl("no database in the path".into()));
        }
        let user = percent_decode(url.username());
        if user.is_empty() {
            return Err(ConfigError::InvalidUrl("no user".into()));
        }
        let password = url.password().map(percent_decode).filter(|p| !p.is_empty());
        let mut config = Self {
            host,
            port: url.port().unwrap_or(PORT_DEFAULT),
            database: percent_decode(&database),
            user,
            password_source: if password.is_some() {
                PasswordSource::Url
            } else {
                PasswordSource::None
            },
            password,
            ssl_mode: SslMode::Prefer,
            root_cert: None,
            connect_timeout: CONNECT_TIMEOUT_DEFAULT,
            application_name: APPLICATION_NAME.into(),
        };
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "sslmode" => config.ssl_mode = SslMode::parse(&value)?,
                "sslrootcert" => config.root_cert = Some(PathBuf::from(value.as_ref())),
                "application_name" => config.application_name = value.chars().take(63).collect(),
                "connect_timeout" => {
                    config.connect_timeout = value
                        .parse::<u64>()
                        .ok()
                        .filter(|t| (1..=300).contains(t))
                        .ok_or_else(|| {
                            ConfigError::Invalid("connect_timeout", value.to_string())
                        })?;
                }
                _ => {}
            }
        }
        Ok(config)
    }

    #[must_use]
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// `user@host:port/database (sslmode)`, never the password.
    #[must_use]
    pub fn display(&self) -> String {
        format!(
            "{}@{}:{}/{} (sslmode {})",
            self.user,
            self.host,
            self.port,
            self.database,
            self.ssl_mode.as_str()
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

    struct NoFiles;

    impl Files for NoFiles {
        fn read(&self, _: &Path) -> Option<(String, u32)> {
            None
        }
    }

    struct OneFile(&'static str, u32);

    impl Files for OneFile {
        fn read(&self, path: &Path) -> Option<(String, u32)> {
            path.ends_with(".pgpass")
                .then(|| (self.0.to_string(), self.1))
        }
    }

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn the_libpq_variables_build_a_connection_and_the_password_file_fills_the_password() {
        let config = ConnectionConfig::from_vars(
            vars(&[
                ("PGHOST", "db.example"),
                ("PGDATABASE", "orders"),
                ("PGUSER", "app"),
                ("HOME", "/home/u"),
            ]),
            &OneFile("db.example:5432:orders:app:s3cret\n", 0o600),
        )
        .unwrap();
        assert_eq!(config.port, 5432);
        assert_eq!(config.password(), Some("s3cret"));
        assert_eq!(config.password_source, PasswordSource::PasswordFile);
        assert_eq!(config.ssl_mode, SslMode::Prefer);
        assert_eq!(
            config.display(),
            "app@db.example:5432/orders (sslmode prefer)"
        );
        assert!(!format!("{config:?}").contains("s3cret"));
        let open = ConnectionConfig::from_vars(
            vars(&[
                ("PGHOST", "db.example"),
                ("PGDATABASE", "orders"),
                ("PGUSER", "app"),
                ("PGPASSFILE", "/x/.pgpass"),
            ]),
            &OneFile("*:*:*:*:leak\n", 0o644),
        )
        .unwrap();
        assert_eq!(open.password(), None);
        assert_eq!(open.password_source, PasswordSource::FileTooOpen);
        let env = ConnectionConfig::from_vars(
            vars(&[
                ("PGHOST", "h"),
                ("PGDATABASE", "d"),
                ("PGUSER", "u"),
                ("PGPASSWORD", "p"),
                ("PGPORT", "5433"),
                ("PGSSLMODE", "verify-full"),
                ("PGSSLROOTCERT", "/certs/rds.pem"),
                ("PGCONNECT_TIMEOUT", "5"),
                ("PGAPPNAME", "bot"),
            ]),
            &NoFiles,
        )
        .unwrap();
        assert_eq!(env.password_source, PasswordSource::Environment);
        assert_eq!(env.port, 5433);
        assert_eq!(env.ssl_mode, SslMode::VerifyFull);
        assert_eq!(env.root_cert.as_deref(), Some(Path::new("/certs/rds.pem")));
        assert_eq!(env.connect_timeout, 5);
        assert_eq!(env.application_name, "bot");
        let none = ConnectionConfig::from_vars(
            vars(&[("PGHOST", "h"), ("PGDATABASE", "d"), ("PGUSER", "u")]),
            &NoFiles,
        )
        .unwrap();
        assert_eq!(none.password_source, PasswordSource::None);
    }

    #[test]
    fn a_url_is_read_with_its_options_and_decoded() {
        let config = ConnectionConfig::from_vars(
            vars(&[("DATABASE_URL", "postgresql://app%40x:p%3Ass@db.example:5433/orders?sslmode=require&application_name=bot&connect_timeout=3")]),
            &NoFiles,
        )
        .unwrap();
        assert_eq!(config.user, "app@x");
        assert_eq!(config.password(), Some("p:ss"));
        assert_eq!(config.password_source, PasswordSource::Url);
        assert_eq!(config.port, 5433);
        assert_eq!(config.ssl_mode, SslMode::Require);
        assert_eq!(config.application_name, "bot");
        assert_eq!(config.connect_timeout, 3);
        let env_wins_ssl = ConnectionConfig::from_vars(
            vars(&[
                ("DATABASE_URL", "postgres://u@h/d"),
                ("PGSSLMODE", "disable"),
                ("HOME", "/home/u"),
            ]),
            &OneFile("h:5432:d:u:fromfile\n", 0o600),
        )
        .unwrap();
        assert_eq!(env_wins_ssl.ssl_mode, SslMode::Disable);
        assert_eq!(
            env_wins_ssl.password(),
            Some("fromfile"),
            "a URL without a password still reads the file"
        );
    }

    #[test]
    fn every_bad_variable_is_named() {
        for (pairs, expected) in [
            (vec![], ConfigError::MissingConnection),
            (
                vec![("PGHOST", "h"), ("PGDATABASE", "d")],
                ConfigError::MissingConnection,
            ),
            (
                vec![
                    ("PGHOST", "h"),
                    ("PGDATABASE", "d"),
                    ("PGUSER", "u"),
                    ("PGPORT", "x"),
                ],
                ConfigError::Invalid("PGPORT", "x".into()),
            ),
            (
                vec![
                    ("PGHOST", "h"),
                    ("PGDATABASE", "d"),
                    ("PGUSER", "u"),
                    ("PGSSLMODE", "maybe"),
                ],
                ConfigError::InvalidSslMode,
            ),
            (
                vec![
                    ("PGHOST", "h"),
                    ("PGDATABASE", "d"),
                    ("PGUSER", "u"),
                    ("PGCONNECT_TIMEOUT", "0"),
                ],
                ConfigError::Invalid("PGCONNECT_TIMEOUT", "0".into()),
            ),
            (
                vec![("DATABASE_URL", "mysql://u@h/d")],
                ConfigError::InvalidUrl("scheme mysql".into()),
            ),
            (
                vec![("DATABASE_URL", "postgres://u@/d")],
                ConfigError::InvalidUrl("empty host".into()),
            ),
            (
                vec![("DATABASE_URL", "postgres://u@h")],
                ConfigError::InvalidUrl("no database in the path".into()),
            ),
            (
                vec![("DATABASE_URL", "postgres://h/d")],
                ConfigError::InvalidUrl("no user".into()),
            ),
            (
                vec![("DATABASE_URL", "postgres://u@h/d?connect_timeout=x")],
                ConfigError::Invalid("connect_timeout", "x".into()),
            ),
            (
                vec![("DATABASE_URL", "::")],
                ConfigError::InvalidUrl("relative URL without a base".into()),
            ),
        ] {
            assert_eq!(
                ConnectionConfig::from_vars(vars(&pairs), &NoFiles).unwrap_err(),
                expected,
                "{pairs:?}"
            );
        }
        assert_eq!(SslMode::parse("verify-ca").unwrap(), SslMode::Require);
        assert_eq!(SslMode::parse("allow").unwrap().as_str(), "prefer");
        assert_eq!(percent_decode("a%2Fb%zz"), "a/b%zz");
    }
}
