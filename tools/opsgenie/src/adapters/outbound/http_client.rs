use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use url::Url;

use crate::domain::{Method, Request, ServerConfig};
use crate::ports::OpsgenieClient;

const ERROR_BODY_CHARS: usize = 500;

/// Opsgenie's REST API over HTTPS, the key in the `Authorization` header.
/// Every segment of a request is pushed as a path segment, never
/// interpolated; a POST carries its body as JSON.
pub struct HttpOpsgenieClient {
    client: Client,
    base_url: Url,
    authorization: String,
}

impl HttpOpsgenieClient {
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
impl OpsgenieClient for HttpOpsgenieClient {
    async fn send(&self, request: &Request) -> Result<Value> {
        let url = self.url_for(request)?;
        let builder = match request.method() {
            Method::Get => self.client.get(url),
            Method::Post => self
                .client
                .post(url)
                .json(request.body().unwrap_or(&Value::Null)),
        };
        let response = builder
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Opsgenie could not be reached")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            bail!(
                "Opsgenie answered {} for {}: check OPSGENIE_API_KEY, its access rights and, for an EU account, OPSGENIE_URL",
                status.as_u16(),
                request.path()
            );
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            bail!(
                "Opsgenie answered 429 for {}: rate limited, wait and call again",
                request.path()
            );
        }
        if !status.is_success() {
            let shown: String = body.chars().take(ERROR_BODY_CHARS).collect();
            bail!(
                "Opsgenie answered {} for {}: {}",
                status.as_u16(),
                request.path(),
                shown.trim()
            );
        }
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body)
            .with_context(|| format!("{} did not answer JSON", request.path()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::domain::{AlertRef, Note};

    fn client(server: &MockServer, prefix: &str) -> HttpOpsgenieClient {
        let config = ServerConfig::new(
            Some(&format!("{}{prefix}", server.uri())),
            Some("key-1"),
            Some("platform"),
            None,
            None,
            None,
        )
        .unwrap();
        HttpOpsgenieClient::new(&config).unwrap()
    }

    #[tokio::test]
    async fn a_get_carries_the_query_and_the_key_under_a_prefix() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/og/v2/alerts/12"))
            .and(query_param("identifierType", "tiny"))
            .and(header("Authorization", "GenieKey key-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"id": "x"}})))
            .mount(&server)
            .await;
        let answer = client(&server, "/og")
            .send(&Request::alert(&AlertRef::Tiny("12".into())))
            .await
            .unwrap();
        assert_eq!(answer["data"]["id"], "x");
        let missing = client(&server, "")
            .send(&Request::open_count())
            .await
            .unwrap_err();
        assert!(missing.to_string().contains("404"), "{missing}");
    }

    #[tokio::test]
    async fn a_post_carries_its_body_and_an_empty_answer_is_null() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/alerts/id-1/close"))
            .and(query_param("identifierType", "id"))
            .and(body_json(
                json!({"source": "opsgenie-mcp", "note": "fixed"}),
            ))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({"requestId": "r-1"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v2/alerts/id-1/acknowledge"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;
        let note = Note::parse(Some("fixed")).unwrap();
        let answer = client(&server, "")
            .send(&Request::close("id-1", &note, None))
            .await
            .unwrap();
        assert_eq!(answer["requestId"], "r-1");
        let empty = client(&server, "")
            .send(&Request::acknowledge("id-1", None, None))
            .await
            .unwrap();
        assert_eq!(empty, Value::Null);
    }

    #[tokio::test]
    async fn a_401_403_or_429_names_the_cause_and_hides_the_body() {
        let server = MockServer::start().await;
        Mock::given(path("/v2/alerts/count"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("{\"message\":\"Key abc not valid\"}"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/v2/teams"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;
        Mock::given(path("/v2/escalations"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .mount(&server)
            .await;
        let client = client(&server, "");
        let error = client
            .send(&Request::open_count())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("401") && error.contains("OPSGENIE_API_KEY") && !error.contains("abc"),
            "{error}"
        );
        let error = client
            .send(&Request::teams())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("403") && !error.contains("forbidden"));
        let error = client
            .send(&Request::escalations())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("rate limited") && !error.contains("slow down"));
    }

    #[tokio::test]
    async fn other_failures_show_a_bounded_body_and_non_json_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(path("/v2/alerts/count"))
            .respond_with(ResponseTemplate::new(400).set_body_string("x".repeat(2000)))
            .mount(&server)
            .await;
        Mock::given(path("/v2/teams"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>login</html>"))
            .mount(&server)
            .await;
        let client = client(&server, "");
        let error = client
            .send(&Request::open_count())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("400") && error.len() < 700, "{error}");
        let error = client
            .send(&Request::teams())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("did not answer JSON"), "{error}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_said_so() {
        let config = ServerConfig::new(
            Some("http://127.0.0.1:9"),
            Some("k"),
            Some("t"),
            None,
            None,
            None,
        )
        .unwrap();
        let error = HttpOpsgenieClient::new(&config)
            .unwrap()
            .send(&Request::open_count())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("could not be reached"));
    }
}
