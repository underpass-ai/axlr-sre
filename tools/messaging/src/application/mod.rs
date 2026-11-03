mod change_queues;
mod diagnose_setup;
mod discover_scope;
mod inspect_queues;
mod queue_lookup;

pub use change_queues::ChangeQueues;
pub use diagnose_setup::DiagnoseSetup;
pub use discover_scope::DiscoverScope;
pub use inspect_queues::InspectQueues;
pub use queue_lookup::QueueLookup;

#[cfg(test)]
pub(crate) mod test_doubles {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use aws_mcp::domain::Invocation;
    use aws_mcp::ports::{AwsRunner, Completed};
    use serde_json::{Value, json};

    use crate::ports::{EvidenceSink, EvidenceStore};

    pub(crate) const URL: &str = "https://sqs.eu-west-1.amazonaws.com/123456789012/demo-orders-dlq";
    pub(crate) const SOURCE: &str = "https://sqs.eu-west-1.amazonaws.com/123456789012/demo-orders";
    pub(crate) const ARN: &str = "arn:aws:sqs:eu-west-1:123456789012:demo-orders-dlq";
    pub(crate) const TOPIC: &str = "arn:aws:sns:eu-west-1:123456789012:demo-topic";

    /// The attributes of a dead-letter queue with `visible` messages.
    pub(crate) fn dlq_attributes(visible: u64) -> Value {
        json!({"Attributes": {"QueueArn": ARN, "ApproximateNumberOfMessages": visible.to_string(), "ApproximateNumberOfMessagesNotVisible": "0", "VisibilityTimeout": "30"}})
    }

    pub(crate) fn message(id: &str, body: &str) -> Value {
        json!({"MessageId": id, "ReceiptHandle": format!("rh-{id}"), "Body": body, "Attributes": {"ApproximateReceiveCount": "5", "SentTimestamp": "1793707200000"}})
    }

    /// Answers by the first key the argument line contains; a key may be
    /// consumed once with `Once`. `Err(text)` is a non-zero exit.
    pub(crate) struct FakeRunner {
        pub answers: Mutex<Vec<(&'static str, Result<Value, String>)>>,
        pub calls: Mutex<Vec<Invocation>>,
    }

    impl FakeRunner {
        pub fn answering(answers: Vec<(&'static str, Result<Value, String>)>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                calls: Mutex::new(Vec::new()),
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

        pub fn writes(&self) -> Vec<String> {
            self.verbs()
                .into_iter()
                .filter(|v| {
                    v.contains("send-message")
                        || v.contains("delete-message")
                        || v.contains("publish")
                        || v.contains("move-task") && !v.contains("list")
                })
                .collect()
        }
    }

    #[async_trait]
    impl AwsRunner for FakeRunner {
        async fn run(&self, invocation: &Invocation) -> Result<Completed> {
            self.calls.lock().unwrap().push(invocation.clone());
            let line = invocation.args().join(" ");
            let mut answers = self.answers.lock().unwrap();
            let position = answers.iter().position(|(key, _)| {
                let key = key.strip_prefix("once:").unwrap_or(key);
                line.contains(key)
            });
            let Some(position) = position else {
                return Ok(Completed {
                    exit_code: Some(252),
                    stdout: String::new(),
                    stderr: format!("fake: unexpected invocation: {line}"),
                });
            };
            let (key, answer) = &answers[position];
            let completed = match answer {
                Ok(value) => Completed {
                    exit_code: Some(0),
                    stdout: match value {
                        Value::Null => String::new(),
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    },
                    stderr: String::new(),
                },
                Err(stderr) => Completed {
                    exit_code: Some(254),
                    stdout: String::new(),
                    stderr: stderr.clone(),
                },
            };
            if key.starts_with("once:") {
                drop(answers.remove(position));
            }
            Ok(completed)
        }
    }

    /// Keeps every appended line in memory and records the order of saves
    /// so a test can check a save came before a delete.
    #[derive(Default)]
    pub(crate) struct FakeEvidence {
        pub lines: Arc<Mutex<Vec<Value>>>,
        pub events: Arc<Mutex<Vec<String>>>,
        pub fail: bool,
    }

    struct FakeSink {
        lines: Arc<Mutex<Vec<Value>>>,
        events: Arc<Mutex<Vec<String>>>,
        path: PathBuf,
        fail: bool,
    }

    impl EvidenceStore for FakeEvidence {
        fn start(&self, queue_name: &str, header: &Value) -> Result<Box<dyn EvidenceSink>> {
            if self.fail {
                return Err(anyhow!("evidence dir not writable"));
            }
            self.lines.lock().unwrap().push(header.clone());
            Ok(Box::new(FakeSink {
                lines: self.lines.clone(),
                events: self.events.clone(),
                path: PathBuf::from(format!("/evidence/{queue_name}.jsonl")),
                fail: false,
            }))
        }
    }

    impl EvidenceSink for FakeSink {
        fn append(&mut self, lines: &[Value]) -> Result<()> {
            if self.fail {
                return Err(anyhow!("disk full"));
            }
            self.events
                .lock()
                .unwrap()
                .push(format!("save:{}", lines.len()));
            self.lines.lock().unwrap().extend(lines.iter().cloned());
            Ok(())
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    /// A runner wrapper that notes every delete, for the save-before-delete order.
    pub(crate) struct Recording {
        pub inner: Arc<FakeRunner>,
        pub events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AwsRunner for Recording {
        async fn run(&self, invocation: &Invocation) -> Result<Completed> {
            if invocation.verb() == "sqs delete-message-batch" {
                self.events.lock().unwrap().push("delete".into());
            }
            self.inner.run(invocation).await
        }
    }
}
