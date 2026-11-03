use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use url::Url;

use crate::domain::{ArgoQuery, ServerConfig};
use crate::ports::ArgoReader;

const LOG_BYTES_MAX: usize = 262_144;
const LOG_ENTRIES_MAX: usize = 200;
const ERROR_BODY_CHARS: usize = 500;

/// Argo Server's HTTP API, GET only, with the bearer token in the header.
pub struct HttpArgoReader {
    client: Client,
    base_url: Url,
    authorization: String,
}

impl HttpArgoReader {
    pub fn new(config: &ServerConfig) -> Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(20)).build()?;
        Ok(Self {
            client,
            base_url: config.base_url().clone(),
            authorization: config.authorization().to_string(),
        })
    }

    fn url_for(&self, query: &ArgoQuery) -> Result<Url> {
        let (collection, namespace, name) = query.endpoint();
        let mut url = self.base_url.join(&format!("api/v1/{collection}"))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("the base URL cannot take a path"))?;
            if let Some(namespace) = namespace {
                segments.push(namespace);
            }
            if let Some(name) = name {
                segments.push(name);
            }
            if query.log_options().is_some() {
                segments.push("log");
            }
        }
        if let Some((limit, continuation)) = query.page() {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("listOptions.limit", &limit.to_string());
            if let Some(token) = continuation {
                pairs.append_pair("listOptions.continue", token);
            }
        }
        if let Some((pod_name, tail_lines)) = query.log_options() {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("logOptions.follow", "false");
            pairs.append_pair("logOptions.tailLines", &tail_lines.to_string());
            pairs.append_pair("logOptions.limitBytes", &LOG_BYTES_MAX.to_string());
            if let Some(pod_name) = pod_name {
                pairs.append_pair("podName", pod_name);
            }
        }
        Ok(url)
    }
}

#[async_trait]
impl ArgoReader for HttpArgoReader {
    async fn read(&self, query: &ArgoQuery) -> Result<Value> {
        let url = self.url_for(query)?;
        let mut response = self
            .client
            .get(url)
            .header("Authorization", &self.authorization)
            .send()
            .await
            .context("Argo Server could not be reached")?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            bail!(
                "Argo Server answered {status}: check ARGO_TOKEN and the service account's permissions on the namespace"
            );
        }
        if !status.is_success() {
            let body: String = response
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(ERROR_BODY_CHARS)
                .collect();
            bail!("Argo Server answered {status}: {body}");
        }
        if query.log_options().is_none() {
            return response
                .json()
                .await
                .context("Argo Server did not answer JSON");
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.context("the log stream broke")? {
            if body.len() + chunk.len() > LOG_BYTES_MAX {
                bail!("the logs exceed 256 KiB; narrow them with pod_name or a smaller tail_lines");
            }
            body.extend_from_slice(&chunk);
        }
        let body = std::str::from_utf8(&body).context("the logs are not UTF-8")?;
        let mut entries = Vec::new();
        for line in body.lines().filter(|line| !line.trim().is_empty()) {
            let line = line.strip_prefix("data: ").unwrap_or(line);
            let entry: Value = serde_json::from_str(line)
                .context("the log stream is not the JSON Argo Server sends")?;
            entries.push(entry.get("result").cloned().unwrap_or(entry));
            if entries.len() >= LOG_ENTRIES_MAX {
                break;
            }
        }
        Ok(json!({"entries": entries, "count": entries.len()}))
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::domain::{ListKind, ResourceKind};

    async fn reader(server: &MockServer) -> HttpArgoReader {
        let config = ServerConfig::new(Some(&server.uri()), Some("ci"), Some("test")).unwrap();
        HttpArgoReader::new(&config).unwrap()
    }

    #[tokio::test]
    async fn lists_with_the_token_and_the_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/workflows/ci"))
            .and(header("authorization", "Bearer test"))
            .and(query_param("listOptions.limit", "20"))
            .and(query_param("listOptions.continue", "next"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": []})))
            .expect(1)
            .mount(&server)
            .await;
        let query =
            ArgoQuery::list(ListKind::Workflows, "ci".into(), 20, Some("next".into())).unwrap();
        assert_eq!(
            reader(&server).await.read(&query).await.unwrap()["items"],
            json!([])
        );
    }

    #[tokio::test]
    async fn a_forbidden_answer_names_the_token_and_hides_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/workflow-templates/ci/build"))
            .respond_with(ResponseTemplate::new(403).set_body_string("secret details"))
            .mount(&server)
            .await;
        let query =
            ArgoQuery::get(ResourceKind::WorkflowTemplate, "ci".into(), "build".into()).unwrap();
        let error = reader(&server)
            .await
            .read(&query)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("403"));
        assert!(error.contains("ARGO_TOKEN"));
        assert!(!error.contains("secret details"));
    }

    #[tokio::test]
    async fn another_failure_shows_a_bounded_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/cron-workflows/ci/nightly"))
            .respond_with(ResponseTemplate::new(500).set_body_string("x".repeat(2000)))
            .mount(&server)
            .await;
        let query =
            ArgoQuery::get(ResourceKind::CronWorkflow, "ci".into(), "nightly".into()).unwrap();
        let error = reader(&server)
            .await
            .read(&query)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("500"));
        assert!(error.len() < 600);
    }

    #[tokio::test]
    async fn the_version_has_no_namespace_in_its_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": "v3.6.0"})))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            reader(&server)
                .await
                .read(&ArgoQuery::Version)
                .await
                .unwrap()["version"],
            "v3.6.0"
        );
    }

    #[tokio::test]
    async fn reads_bounded_workflow_logs() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/workflows/ci/run-1/log"))
            .and(query_param("logOptions.tailLines", "20"))
            .and(query_param("logOptions.follow", "false"))
            .and(query_param("podName", "run-1-pod"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{\"result\":{\"content\":\"hello\"}}\n\ndata: {\"result\":{\"content\":\"world\"}}\n",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let query =
            ArgoQuery::logs("ci".into(), "run-1".into(), Some("run-1-pod".into()), 20).unwrap();
        let result = reader(&server).await.read(&query).await.unwrap();
        assert_eq!(result["count"], 2);
        assert_eq!(result["entries"][0]["content"], "hello");
        assert_eq!(result["entries"][1]["content"], "world");
    }

    #[tokio::test]
    async fn a_log_stream_past_the_bound_or_not_json_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/workflows/ci/big/log"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(LOG_BYTES_MAX + 1)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/workflows/ci/odd/log"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json\n"))
            .mount(&server)
            .await;
        let reader = reader(&server).await;
        let big = ArgoQuery::logs("ci".into(), "big".into(), None, 10).unwrap();
        assert!(
            reader
                .read(&big)
                .await
                .unwrap_err()
                .to_string()
                .contains("256 KiB")
        );
        let odd = ArgoQuery::logs("ci".into(), "odd".into(), None, 10).unwrap();
        assert!(
            reader
                .read(&odd)
                .await
                .unwrap_err()
                .to_string()
                .contains("JSON")
        );
    }

    #[tokio::test]
    async fn a_server_that_is_down_is_reported_as_unreachable() {
        let config =
            ServerConfig::new(Some("http://127.0.0.1:9"), Some("ci"), Some("test")).unwrap();
        let reader = HttpArgoReader::new(&config).unwrap();
        let error = reader
            .read(&ArgoQuery::Version)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("could not be reached"));
    }
}
