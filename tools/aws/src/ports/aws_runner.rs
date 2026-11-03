use anyhow::Result;
use async_trait::async_trait;

use crate::domain::Invocation;

/// What an `aws` run left: the exit code (none when a signal killed it),
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

/// The one thing the use cases need: run `aws` with validated arguments and
/// give back what it said. An `Err` is a failure to run at all; a non-zero
/// exit comes back as a `Completed` whose stderr the domain translates.
#[async_trait]
pub trait AwsRunner: Send + Sync {
    async fn run(&self, invocation: &Invocation) -> Result<Completed>;
}
