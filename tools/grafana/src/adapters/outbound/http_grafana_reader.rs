use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use url::Url;

use crate::domain::{Request, ServerConfig};
use crate::ports::GrafanaReader;

const ERROR_BODY_CHARS: usize = 500;

/// Grafana's HTTP API, GET only, the token in the header. Every segment of
/// a request is pushed as a path segment, never interpolated.
pub struct HttpGrafanaReader {
    client: Client,
    base_url: Url,
    authorization: String,
}

impl HttpGrafanaReader {
    pub fn new(config: &ServerConfig) -> Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
        Ok(Self {
            client,
            base_url: config.base_url().clone(),
            authorization: config.authorization().to_string(),
        })
    }

    fn url_for(&self, request: &Request) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("the base URL cannot take a path"))?
            .pop_if_empty()
            .extend(request.segments());
        if !request.query().is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in request.query() {
                pairs.append_pair(name, value);
            }
        }
        Ok(url)
    }
}

#[async_trait]
impl GrafanaReader for HttpGrafanaReader {
    async fn get(&self, request: &Request) -> Result<Value> {
        let url = self.url_for(request)?;
        let response = self
            .client
            .get(url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Grafana could not be reached")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            bail!(
                "Grafana answered {} for {}: check GRAFANA_TOKEN and the service account's role",
                status.as_u16(),
                request.path()
            );
        }
        if !status.is_success() {
            let shown: String = body.chars().take(ERROR_BODY_CHARS).collect();
            bail!(
                "Grafana answered {} for {}: {}",
                status.as_u16(),
                request.path(),
                shown.trim()
            );
        }
        serde_json::from_str(&body)
            .with_context(|| format!("{} did not answer JSON", request.path()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::domain::TimeWindow;

    fn reader(server: &MockServer, prefix: &str) -> HttpGrafanaReader {
        let config = ServerConfig::new(
            Some(&format!("{}{prefix}", server.uri())),
            Some("tok"),
            None,
            None,
        )
        .unwrap();
        HttpGrafanaReader::new(&config).unwrap()
    }

    #[tokio::test]
    async fn segments_query_and_token_reach_grafana_under_a_prefix() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/grafana/api/datasources/proxy/uid/loki-1/loki/api/v1/label/app/values",
            ))
            .and(query_param("start", "1793696400000000000"))
            .and(header("Authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": ["web"]})))
            .mount(&server)
            .await;
        let window = TimeWindow::new(
            "2026-11-03T09:00:00Z".parse().unwrap(),
            "2026-11-03T10:00:00Z".parse().unwrap(),
        )
        .unwrap();
        let request = Request::loki_label_values("loki-1", "app", &window).unwrap();
        let answer = reader(&server, "/grafana").get(&request).await.unwrap();
        assert_eq!(answer["data"][0], "web");
        let plain = reader(&server, "")
            .get(&Request::health())
            .await
            .unwrap_err();
        assert!(plain.to_string().contains("404"), "{plain}");
    }

    #[tokio::test]
    async fn a_401_or_403_names_the_token_and_hides_the_body() {
        let server = MockServer::start().await;
        Mock::given(path("/api/org"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("{\"message\":\"invalid API key abc\"}"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/api/datasources"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;
        let error = reader(&server, "")
            .get(&Request::org())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("401") && error.contains("GRAFANA_TOKEN") && !error.contains("abc"),
            "{error}"
        );
        let error = reader(&server, "")
            .get(&Request::datasources())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("403") && !error.contains("forbidden"));
    }

    #[tokio::test]
    async fn other_failures_show_a_bounded_body_and_non_json_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(path("/api/health"))
            .respond_with(ResponseTemplate::new(400).set_body_string("x".repeat(2000)))
            .mount(&server)
            .await;
        Mock::given(path("/api/org"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>login</html>"))
            .mount(&server)
            .await;
        let error = reader(&server, "")
            .get(&Request::health())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("400") && error.len() < 700, "{error}");
        let error = reader(&server, "")
            .get(&Request::org())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("did not answer JSON"), "{error}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_said_so() {
        let config =
            ServerConfig::new(Some("http://127.0.0.1:9"), Some("tok"), None, None).unwrap();
        let error = HttpGrafanaReader::new(&config)
            .unwrap()
            .get(&Request::health())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("could not be reached"));
    }
}
