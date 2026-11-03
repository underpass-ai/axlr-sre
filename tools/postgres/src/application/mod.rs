mod change_server;
mod diagnose_setup;
mod discover_scope;
mod inspect_server;
mod run_query;
pub(crate) mod shared;

pub use change_server::ChangeServer;
pub use diagnose_setup::DiagnoseSetup;
pub use discover_scope::DiscoverScope;
pub use inspect_server::InspectServer;
pub use run_query::RunQuery;

#[cfg(test)]
pub(crate) mod test_doubles {
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;

    use crate::ports::{Outcome, SqlGateway, SqlSession};

    pub(crate) type Answer = (&'static str, Result<Outcome, String>);

    /// Rows from column names and text cells.
    pub(crate) fn rows(columns: &[&str], rows: &[&[&str]]) -> Result<Outcome, String> {
        Ok(Outcome::Rows {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows: rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| (*c != "NULL").then(|| c.to_string()))
                        .collect()
                })
                .collect(),
        })
    }

    pub(crate) fn command(tag: &str, affected: Option<u64>) -> Result<Outcome, String> {
        Ok(Outcome::Command {
            tag: tag.into(),
            affected,
        })
    }

    /// Answers each statement by the first key it contains; a statement
    /// with no key succeeds as an empty command (BEGIN, SET, ROLLBACK).
    /// Every connection shares the answers and the log.
    pub(crate) struct FakeGateway {
        pub answers: Mutex<Vec<Answer>>,
        pub log: Arc<Mutex<Vec<String>>>,
        pub connections: Mutex<usize>,
        pub refuse: bool,
    }

    impl FakeGateway {
        pub fn answering(answers: Vec<Answer>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                log: Arc::new(Mutex::new(Vec::new())),
                connections: Mutex::new(0),
                refuse: false,
            })
        }

        pub fn refusing() -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(Vec::new()),
                log: Arc::new(Mutex::new(Vec::new())),
                connections: Mutex::new(0),
                refuse: true,
            })
        }

        pub fn set(&self, answers: Vec<Answer>) {
            *self.answers.lock().unwrap() = answers;
            self.log.lock().unwrap().clear();
        }

        pub fn statements(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    struct FakeSession {
        answers: Vec<Answer>,
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl SqlGateway for FakeGateway {
        async fn connect(&self) -> Result<Box<dyn SqlSession>> {
            if self.refuse {
                return Err(anyhow!("connection refused (127.0.0.1:5432)"));
            }
            *self.connections.lock().unwrap() += 1;
            Ok(Box::new(FakeSession {
                answers: self.answers.lock().unwrap().clone(),
                log: self.log.clone(),
            }))
        }
    }

    #[async_trait]
    impl SqlSession for FakeSession {
        async fn run(&mut self, sql: &str) -> Result<Outcome> {
            self.log.lock().unwrap().push(sql.to_string());
            match self.answers.iter().find(|(key, _)| sql.contains(key)) {
                Some((_, Ok(outcome))) => Ok(outcome.clone()),
                Some((_, Err(message))) => Err(anyhow!("{message}")),
                None => Ok(Outcome::Command {
                    tag: sql.split_whitespace().next().unwrap_or("").to_uppercase(),
                    affected: None,
                }),
            }
        }
    }
}
