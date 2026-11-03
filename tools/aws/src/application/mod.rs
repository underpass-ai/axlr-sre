mod diagnose_setup;
mod discover_scope;
mod inspect_lambda;
mod query_logs;
mod query_metrics;
pub(crate) mod shared;

pub use diagnose_setup::DiagnoseSetup;
pub use discover_scope::DiscoverScope;
pub use inspect_lambda::InspectLambda;
pub use query_logs::QueryLogs;
pub use query_metrics::QueryMetrics;

#[cfg(test)]
pub(crate) mod test_doubles {
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use serde_json::Value;

    use crate::domain::Invocation;
    use crate::ports::{AwsRunner, Completed};

    /// Answers by the first entry whose key every argument-joined line
    /// contains; `Err(text)` is a non-zero exit with that stderr. Records
    /// every invocation.
    pub(crate) struct FakeRunner {
        pub answers: Mutex<Vec<(&'static str, Result<Value, String>)>>,
        pub calls: Mutex<Vec<Invocation>>,
        pub fail_to_run: bool,
    }

    impl FakeRunner {
        pub fn answering(answers: Vec<(&'static str, Result<Value, String>)>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                calls: Mutex::new(Vec::new()),
                fail_to_run: false,
            })
        }

        pub fn set(&self, answers: Vec<(&'static str, Result<Value, String>)>) {
            *self.answers.lock().unwrap() = answers;
        }

        pub fn verbs(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(Invocation::verb)
                .collect()
        }

        pub fn lines(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|i| i.args().join(" "))
                .collect()
        }
    }

    #[async_trait]
    impl AwsRunner for FakeRunner {
        async fn run(&self, invocation: &Invocation) -> Result<Completed> {
            if self.fail_to_run {
                return Err(anyhow!("could not start aws"));
            }
            self.calls.lock().unwrap().push(invocation.clone());
            let line = invocation.args().join(" ");
            let answers = self.answers.lock().unwrap();
            match answers.iter().find(|(key, _)| line.contains(key)) {
                Some((_, Ok(value))) => Ok(Completed {
                    exit_code: Some(0),
                    stdout: if value.is_string() {
                        value.as_str().unwrap_or_default().to_string()
                    } else {
                        value.to_string()
                    },
                    stderr: String::new(),
                }),
                Some((_, Err(stderr))) => Ok(Completed {
                    exit_code: Some(254),
                    stdout: String::new(),
                    stderr: stderr.clone(),
                }),
                None => Ok(Completed {
                    exit_code: Some(252),
                    stdout: String::new(),
                    stderr: format!("fake: unexpected invocation: {line}"),
                }),
            }
        }
    }
}
