use std::fs::{DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use crate::domain::MirrorState;
use crate::ports::StateStore;

/// The mirror's state in a JSON file, directory 0700, file 0600, written
/// through a temporary file and `rename` so a crash leaves the old one.
pub struct JsonStateFile {
    path: PathBuf,
}

impl JsonStateFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn broken(&self, detail: impl std::fmt::Display) -> anyhow::Error {
        anyhow!(
            "state {}: {detail}; fix it or delete it, and the mirror starts again with a baseline",
            self.path.display()
        )
    }
}

impl StateStore for JsonStateFile {
    fn load(&self) -> Result<MirrorState> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(MirrorState::default()),
            Err(error) => return Err(self.broken(error)),
        };
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| self.broken(error))?;
        MirrorState::from_json(&value).map_err(|error| self.broken(error))
    }

    fn save(&self, state: &MirrorState) -> Result<()> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|error| self.broken(error))?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(&state.to_json()).context("state as JSON")?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| self.broken(error))?;
        file.write_all(body.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| self.broken(error))?;
        std::fs::rename(&temporary, &self.path).map_err(|error| self.broken(error))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn without_a_file_the_state_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let state = JsonStateFile::new(dir.path().join("no/such.json"))
            .load()
            .unwrap();
        assert_eq!(state, MirrorState::default());
    }

    #[test]
    fn saves_and_loads_with_0600_and_a_0700_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mirror/state.json");
        let file = JsonStateFile::new(path.clone());
        let mut state = MirrorState::default();
        state.record_baseline(&["a".to_string()]);
        state.record_opened("b", "inc-b");
        file.save(&state).unwrap();
        assert_eq!(file.load().unwrap(), state);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("mirror"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(!dir.path().join("mirror/state.json.tmp").exists());
    }

    #[test]
    fn a_corrupt_file_is_explained_and_never_a_fresh_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{").unwrap();
        let error = JsonStateFile::new(path.clone())
            .load()
            .unwrap_err()
            .to_string();
        assert!(error.contains("baseline"), "{error}");
        std::fs::write(&path, r#"{"version": 2}"#).unwrap();
        assert!(JsonStateFile::new(path).load().is_err());
    }

    #[test]
    fn a_directory_that_cannot_be_created_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file"), "").unwrap();
        let file = JsonStateFile::new(dir.path().join("file/state.json"));
        assert!(file.save(&MirrorState::default()).is_err());
        assert!(JsonStateFile::new(dir.path().to_path_buf()).load().is_err());
    }
}
