mod datasources;
mod diagnose_setup;
mod discover_scope;
mod inspect_grafana;
mod query_logs;
mod query_metrics;

pub use datasources::Datasources;
pub use diagnose_setup::DiagnoseSetup;
pub use discover_scope::DiscoverScope;
pub use inspect_grafana::InspectGrafana;
pub use query_logs::QueryLogs;
pub use query_metrics::QueryMetrics;

#[cfg(test)]
pub(crate) mod test_doubles {
    use std::sync::Mutex;

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use serde_json::Value;

    use crate::domain::Request;
    use crate::ports::GrafanaReader;

    type Call = (String, Vec<(String, String)>);

    /// Answers by path prefix; records every request.
    pub(crate) struct FakeReader {
        pub answers: Vec<(&'static str, Result<Value, String>)>,
        pub calls: Mutex<Vec<Call>>,
    }

    impl FakeReader {
        pub fn answering(
            answers: Vec<(&'static str, Result<Value, String>)>,
        ) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                answers,
                calls: Mutex::new(Vec::new()),
            })
        }

        pub fn paths(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(path, _)| path.clone())
                .collect()
        }

        pub fn queries(&self) -> Vec<Vec<(String, String)>> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(_, query)| query.clone())
                .collect()
        }
    }

    #[async_trait]
    impl GrafanaReader for FakeReader {
        async fn get(&self, request: &Request) -> Result<Value> {
            let path = request.path();
            self.calls
                .lock()
                .unwrap()
                .push((path.clone(), request.query().to_vec()));
            match self
                .answers
                .iter()
                .find(|(prefix, _)| path.starts_with(prefix))
            {
                Some((_, Ok(value))) => Ok(value.clone()),
                Some((_, Err(message))) => Err(anyhow!("{message}")),
                None => Err(anyhow!("unexpected request: {path}")),
            }
        }
    }
}
