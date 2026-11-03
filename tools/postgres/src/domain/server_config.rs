use std::path::Path;

use super::connection::Files;
use super::{ConfigError, ConnectionConfig};

/// Which writes `execute` may run: none, the maintenance statements, or all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WriteLevel {
    Off,
    Maintenance,
    All,
}

impl WriteLevel {
    pub fn parse(value: Option<&str>) -> Result<Self, ConfigError> {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("off") | Some("false") | Some("0") | Some("no") => Ok(Self::Off),
            Some("maintenance") => Ok(Self::Maintenance),
            Some("all") | Some("true") => Ok(Self::All),
            Some(_) => Err(ConfigError::InvalidWrites),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Maintenance => "maintenance",
            Self::All => "all",
        }
    }
}

/// What the environment says at start: the connection and the write level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    connection: ConnectionConfig,
    writes: WriteLevel,
}

/// The disk: the password file is read with its mode.
pub struct Disk;

impl Files for Disk {
    fn read(&self, path: &Path) -> Option<(String, u32)> {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).ok()?.permissions().mode();
        let text = std::fs::read_to_string(path).ok()?;
        Some((text, mode))
    }
}

impl ServerConfig {
    pub fn new(connection: ConnectionConfig, writes: WriteLevel) -> Self {
        Self { connection, writes }
    }

    /// `DATABASE_URL` or `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`,
    /// `PGPASSWORD`, `PGPASSFILE`, `PGSSLMODE`, `PGSSLROOTCERT`,
    /// `PGCONNECT_TIMEOUT`, `PGAPPNAME`; and `POSTGRES_WRITES`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var(name).ok(), &Disk)
    }

    pub fn from_vars(
        get: impl Fn(&str) -> Option<String>,
        files: &dyn Files,
    ) -> Result<Self, ConfigError> {
        let writes = WriteLevel::parse(get("POSTGRES_WRITES").as_deref())?;
        let connection = ConnectionConfig::from_vars(&get, files)?;
        Ok(Self { connection, writes })
    }

    #[must_use]
    pub fn connection(&self) -> &ConnectionConfig {
        &self.connection
    }

    #[must_use]
    pub fn writes(&self) -> WriteLevel {
        self.writes
    }

    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{}, writes {}",
            self.connection.display(),
            self.writes.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_write_level_is_read_and_the_disk_reads_a_file_with_its_mode() {
        assert_eq!(WriteLevel::parse(None).unwrap(), WriteLevel::Off);
        assert_eq!(
            WriteLevel::parse(Some("Maintenance")).unwrap(),
            WriteLevel::Maintenance
        );
        assert_eq!(WriteLevel::parse(Some("all")).unwrap(), WriteLevel::All);
        assert_eq!(
            WriteLevel::parse(Some("some")).unwrap_err(),
            ConfigError::InvalidWrites
        );
        assert!(WriteLevel::Maintenance < WriteLevel::All);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".pgpass");
        std::fs::write(&path, "h:5432:d:u:p\n").unwrap();
        let (text, mode) = Disk.read(&path).unwrap();
        assert!(text.starts_with("h:5432"));
        assert!(mode & 0o777 != 0);
        assert!(Disk.read(&dir.path().join("none")).is_none());
        let config = ServerConfig::from_vars(
            |name| match name {
                "PGHOST" => Some("h".into()),
                "PGDATABASE" => Some("d".into()),
                "PGUSER" => Some("u".into()),
                "PGPASSFILE" => Some(path.display().to_string()),
                "POSTGRES_WRITES" => Some("maintenance".into()),
                _ => None,
            },
            &Disk,
        )
        .unwrap();
        assert_eq!(config.writes(), WriteLevel::Maintenance);
        assert!(config.summary().contains("writes maintenance"));
        assert!(!config.summary().contains(":p"));
        assert_eq!(config.connection().host, "h");
        assert!(
            ServerConfig::from_vars(
                |name| (name == "POSTGRES_WRITES").then(|| "x".into()),
                &Disk
            )
            .is_err()
        );
    }
}
