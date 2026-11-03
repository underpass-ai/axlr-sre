use std::path::PathBuf;

use aws_mcp::domain::ServerConfig;

use super::ConfigError;

/// What the environment says at start: the `aws-mcp` variables
/// (`AWS_MCP_REGIONS`, `AWS_MCP_PROFILE`, `AWS_CLI`), whether writes are
/// published and where a drain writes its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagingConfig {
    aws: ServerConfig,
    writes: bool,
    snapshot_dir: PathBuf,
}

impl MessagingConfig {
    pub fn new(
        aws: ServerConfig,
        writes: Option<&str>,
        snapshot_dir: Option<&str>,
        home: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let writes = match writes
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("") | Some("false") | Some("0") | Some("no") => false,
            Some("true") | Some("1") | Some("yes") => true,
            Some(_) => return Err(ConfigError::InvalidWrites),
        };
        let snapshot_dir = match snapshot_dir.map(str::trim).filter(|d| !d.is_empty()) {
            Some(dir) if !dir.contains('\0') => PathBuf::from(dir),
            Some(_) => return Err(ConfigError::InvalidSnapshotDir),
            None => home
                .filter(|h| !h.trim().is_empty())
                .map(|h| PathBuf::from(h).join("messaging-mcp").join("snapshots"))
                .ok_or(ConfigError::InvalidSnapshotDir)?,
        };
        Ok(Self {
            aws,
            writes,
            snapshot_dir,
        })
    }

    /// The `aws-mcp` variables plus `MESSAGING_WRITES` and
    /// `MESSAGING_SNAPSHOT_DIR` (`$XDG_STATE_HOME/messaging-mcp/snapshots` or
    /// `~/.local/state/messaging-mcp/snapshots` when unset).
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let aws = ServerConfig::from_vars(&get)?;
        let home = get("XDG_STATE_HOME")
            .filter(|v| !v.trim().is_empty())
            .or_else(|| get("HOME").map(|h| format!("{h}/.local/state")));
        Self::new(
            aws,
            get("MESSAGING_WRITES").as_deref(),
            get("MESSAGING_SNAPSHOT_DIR").as_deref(),
            home.as_deref(),
        )
    }

    #[must_use]
    pub fn aws(&self) -> &ServerConfig {
        &self.aws
    }

    #[must_use]
    pub fn writes(&self) -> bool {
        self.writes
    }

    #[must_use]
    pub fn snapshot_dir(&self) -> &PathBuf {
        &self.snapshot_dir
    }

    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{}, writes {}, snapshots in {}",
            self.aws.summary(),
            if self.writes { "on" } else { "off" },
            self.snapshot_dir.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aws() -> ServerConfig {
        ServerConfig::new(Some("eu-west-1"), None, None).unwrap()
    }

    #[test]
    fn writes_and_the_snapshot_dir_have_defaults_and_are_checked() {
        let config = MessagingConfig::new(aws(), None, None, Some("/home/x/.local/state")).unwrap();
        assert!(!config.writes());
        assert_eq!(
            config.snapshot_dir(),
            &PathBuf::from("/home/x/.local/state/messaging-mcp/snapshots")
        );
        assert!(config.summary().contains("writes off"));
        assert_eq!(config.aws().regions().default_region(), Some("eu-west-1"));
        let explicit = MessagingConfig::new(aws(), Some("TRUE"), Some("/var/snap"), None).unwrap();
        assert!(explicit.writes());
        assert_eq!(explicit.snapshot_dir(), &PathBuf::from("/var/snap"));
        assert_eq!(
            MessagingConfig::new(aws(), Some("maybe"), None, Some("/h")).unwrap_err(),
            ConfigError::InvalidWrites
        );
        assert_eq!(
            MessagingConfig::new(aws(), None, None, None).unwrap_err(),
            ConfigError::InvalidSnapshotDir
        );
        assert_eq!(
            MessagingConfig::new(aws(), None, Some("a\0b"), None).unwrap_err(),
            ConfigError::InvalidSnapshotDir
        );
    }

    #[test]
    fn the_variables_are_read_and_an_aws_error_passes_through() {
        let vars = |pairs: &'static [(&str, &str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        let config = MessagingConfig::from_vars(vars(&[
            ("AWS_MCP_REGIONS", "eu-west-1"),
            ("MESSAGING_WRITES", "true"),
            ("XDG_STATE_HOME", "/state"),
        ]))
        .unwrap();
        assert!(config.writes());
        assert_eq!(
            config.snapshot_dir(),
            &PathBuf::from("/state/messaging-mcp/snapshots")
        );
        let home = MessagingConfig::from_vars(vars(&[("HOME", "/home/u")])).unwrap();
        assert_eq!(
            home.snapshot_dir(),
            &PathBuf::from("/home/u/.local/state/messaging-mcp/snapshots")
        );
        let bad = MessagingConfig::from_vars(vars(&[("AWS_MCP_REGIONS", "mars"), ("HOME", "/h")]))
            .unwrap_err();
        assert!(matches!(bad, ConfigError::Aws(_)));
        assert!(bad.to_string().contains("AWS_MCP_REGIONS"));
    }
}
