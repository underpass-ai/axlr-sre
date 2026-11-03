use anyhow::Result;
use async_trait::async_trait;

use crate::domain::Invocation;

/// What a `kubectl` run left: the exit code (none when a signal killed it),
/// stdout and stderr.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Completed {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Completed {
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// The one thing the use cases need: run `kubectl` with validated arguments
/// and give back what it said. An `Err` is a failure to run at all; a
/// non-zero exit comes back as a `Completed`.
#[async_trait]
pub trait KubectlRunner: Send + Sync {
    async fn run(&self, invocation: &Invocation) -> Result<Completed>;

    /// Whether a program `kubectl` shells out to (such as `diff`) is on the PATH.
    fn program_available(&self, program: &str) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exit_zero_is_success() {
        assert!(
            Completed {
                exit_code: Some(0),
                ..Default::default()
            }
            .succeeded()
        );
        assert!(
            !Completed {
                exit_code: Some(1),
                ..Default::default()
            }
            .succeeded()
        );
        assert!(!Completed::default().succeeded());
    }
}
