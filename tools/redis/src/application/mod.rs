mod change_instance;
mod diagnose_setup;
mod discover_scope;
mod inspect_instance;
mod inspect_keys;
pub(crate) mod shared;

pub use change_instance::ChangeInstance;
pub use diagnose_setup::DiagnoseSetup;
pub use discover_scope::DiscoverScope;
pub use inspect_instance::InspectInstance;
pub use inspect_keys::InspectKeys;

#[cfg(test)]
pub(crate) mod test_doubles {
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;

    use crate::domain::Reply;
    use crate::ports::{RedisGateway, RedisSession};

    pub(crate) type Answer = (&'static str, Result<Reply, String>);

    pub(crate) fn text(s: &str) -> Reply {
        Reply::Text(s.into())
    }

    pub(crate) fn texts(items: &[&str]) -> Reply {
        Reply::Array(items.iter().map(|s| text(s)).collect())
    }

    /// Answers each command by the first key its joined form starts with;
    /// a command with no key answers OK. Every connection shares the
    /// answers and the log.
    pub(crate) struct FakeGateway {
        pub answers: Mutex<Vec<Answer>>,
        pub log: Arc<Mutex<Vec<String>>>,
        pub refuse: bool,
    }

    impl FakeGateway {
        pub fn answering(answers: Vec<Answer>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                log: Arc::new(Mutex::new(Vec::new())),
                refuse: false,
            })
        }

        pub fn refusing() -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(Vec::new()),
                log: Arc::new(Mutex::new(Vec::new())),
                refuse: true,
            })
        }

        pub fn set(&self, answers: Vec<Answer>) {
            *self.answers.lock().unwrap() = answers;
            self.log.lock().unwrap().clear();
        }

        pub fn commands(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    struct FakeSession {
        answers: Vec<Answer>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl FakeSession {
        fn answer(&self, args: &[String]) -> Result<Reply> {
            let line = args.join(" ");
            self.log.lock().unwrap().push(line.clone());
            match self.answers.iter().find(|(key, _)| line.starts_with(key)) {
                Some((_, Ok(reply))) => Ok(reply.clone()),
                Some((_, Err(message))) => Err(anyhow!("{message}")),
                None => Ok(text("OK")),
            }
        }
    }

    #[async_trait]
    impl RedisGateway for FakeGateway {
        async fn connect(&self) -> Result<Box<dyn RedisSession>> {
            if self.refuse {
                return Err(anyhow!("connection refused"));
            }
            Ok(Box::new(FakeSession {
                answers: self.answers.lock().unwrap().clone(),
                log: self.log.clone(),
            }))
        }
    }

    #[async_trait]
    impl RedisSession for FakeSession {
        async fn command(&mut self, args: &[String]) -> Result<Reply> {
            self.answer(args)
        }

        async fn pipeline(&mut self, commands: &[Vec<String>]) -> Result<Vec<Reply>> {
            commands.iter().map(|c| self.answer(c)).collect()
        }
    }
}
