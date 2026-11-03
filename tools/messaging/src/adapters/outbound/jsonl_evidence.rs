use std::fs::{DirBuilder, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jiff::Timestamp;
use serde_json::Value;

use crate::ports::{EvidenceSink, EvidenceStore};

/// A drain's evidence as JSON Lines: `<dir>/<queue>-<time>.jsonl`, the
/// directory 0700 and the file 0600, opened once, a header line then one
/// line per message, synced to disk after every batch. A torn last line
/// means a crash mid-batch, and that batch was not deleted.
pub struct JsonlEvidence {
    dir: PathBuf,
}

impl JsonlEvidence {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

struct JsonlFile {
    file: File,
    path: PathBuf,
}

impl EvidenceStore for JsonlEvidence {
    fn start(&self, queue_name: &str, header: &Value) -> Result<Box<dyn EvidenceSink>> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .with_context(|| format!("evidence directory {}", self.dir.display()))?;
        let safe: String = queue_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let stamp = Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
        // Two drains of one queue in the same second get distinct files.
        let (file, path) = (0..100)
            .map(|n| {
                let suffix = if n == 0 {
                    String::new()
                } else {
                    format!("-{n}")
                };
                self.dir.join(format!("{safe}-{stamp}{suffix}.jsonl"))
            })
            .find_map(|path| {
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                {
                    Ok(file) => Some(Ok((file, path))),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(
                        Err(error).with_context(|| format!("evidence file {}", path.display())),
                    ),
                }
            })
            .unwrap_or_else(|| {
                Err(anyhow::anyhow!(
                    "a hundred evidence files already exist for {safe} this second"
                ))
            })?;
        let mut sink = JsonlFile { file, path };
        sink.append(std::slice::from_ref(header))?;
        Ok(Box::new(sink))
    }
}

impl EvidenceSink for JsonlFile {
    fn append(&mut self, lines: &[Value]) -> Result<()> {
        let mut text = String::new();
        for line in lines {
            text.push_str(&line.to_string());
            text.push('\n');
        }
        self.file
            .write_all(text.as_bytes())
            .and_then(|()| self.file.sync_all())
            .with_context(|| format!("writing evidence to {}", self.path.display()))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;

    #[test]
    fn a_file_per_drain_with_a_header_and_a_line_per_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonlEvidence::new(dir.path().join("snapshots"));
        let mut sink = store
            .start("demo-orders.fifo", &json!({"queue": "demo-orders.fifo"}))
            .unwrap();
        sink.append(&[json!({"id": "m1", "body": "a\nb"}), json!({"id": "m2"})])
            .unwrap();
        let path = sink.path().to_path_buf();
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("demo-orders_fifo-")
        );
        assert!(path.extension().is_some_and(|e| e == "jsonl"));
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["queue"], "demo-orders.fifo");
        assert_eq!(lines[1]["body"], "a\nb");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("snapshots"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn a_directory_that_cannot_be_created_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file"), "").unwrap();
        let store = JsonlEvidence::new(dir.path().join("file").join("snapshots"));
        let error = match store.start("q", &json!({})) {
            Ok(_) => panic!("started under a file"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("evidence directory"), "{error}");
    }
}
