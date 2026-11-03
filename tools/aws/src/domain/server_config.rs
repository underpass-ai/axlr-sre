use std::path::PathBuf;

use super::{ConfigError, Regions};

/// What the environment says at start: the regions in scope, the profile
/// to pass the CLI (none: the CLI's own credential chain, which is how a
/// node with a role works) and where the `aws` executable is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    regions: Regions,
    profile: Option<String>,
    cli: PathBuf,
}

impl ServerConfig {
    pub fn new(
        regions: Option<&str>,
        profile: Option<&str>,
        cli: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let regions = Regions::new(regions)?;
        let profile = match profile.map(str::trim).filter(|p| !p.is_empty()) {
            None => None,
            Some(name)
                if name.chars().count() <= 128
                    && !name.starts_with('-')
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) =>
            {
                Some(name.to_string())
            }
            Some(_) => return Err(ConfigError::InvalidProfile),
        };
        let cli = match cli {
            None => PathBuf::from("aws"),
            Some(path) if !path.trim().is_empty() => PathBuf::from(path.trim()),
            Some(_) => return Err(ConfigError::InvalidCli),
        };
        Ok(Self {
            regions,
            profile,
            cli,
        })
    }

    /// `AWS_MCP_REGIONS`, `AWS_MCP_PROFILE`, `AWS_CLI`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Self::new(
            get("AWS_MCP_REGIONS").as_deref(),
            get("AWS_MCP_PROFILE").as_deref(),
            get("AWS_CLI").as_deref(),
        )
    }

    #[must_use]
    pub fn regions(&self) -> &Regions {
        &self.regions
    }

    #[must_use]
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    #[must_use]
    pub fn cli(&self) -> &PathBuf {
        &self.cli
    }

    /// One line for `doctor` and `discover`.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "regions {}, profile {}, cli {}",
            if self.regions.allowed().is_empty() {
                "any, named per call".to_string()
            } else {
                self.regions.allowed().join(", ")
            },
            self.profile
                .as_deref()
                .unwrap_or("the CLI's own credentials"),
            self.cli.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_cli_on_the_path_and_no_profile() {
        let config = ServerConfig::new(None, None, None).unwrap();
        assert_eq!(config.profile(), None);
        assert_eq!(config.cli(), &PathBuf::from("aws"));
        assert!(config.regions().allowed().is_empty());
        assert!(config.summary().contains("any, named per call"));
        let full = ServerConfig::new(
            Some("eu-west-1"),
            Some(" ops-ro "),
            Some("/usr/local/bin/aws"),
        )
        .unwrap();
        assert_eq!(full.profile(), Some("ops-ro"));
        assert!(full.summary().contains("eu-west-1") && full.summary().contains("ops-ro"));
        let from_vars =
            ServerConfig::from_vars(|name| (name == "AWS_MCP_PROFILE").then(|| "demo".to_string()))
                .unwrap();
        assert_eq!(from_vars.profile(), Some("demo"));
    }

    #[test]
    fn bad_variables_are_named() {
        assert_eq!(
            ServerConfig::new(Some("eu-west-1,x"), None, None).unwrap_err(),
            ConfigError::InvalidRegions("x".into())
        );
        assert_eq!(
            ServerConfig::new(None, Some("-p"), None).unwrap_err(),
            ConfigError::InvalidProfile
        );
        assert_eq!(
            ServerConfig::new(None, Some("a b"), None).unwrap_err(),
            ConfigError::InvalidProfile
        );
        assert_eq!(
            ServerConfig::new(None, None, Some(" ")).unwrap_err(),
            ConfigError::InvalidCli
        );
    }
}
