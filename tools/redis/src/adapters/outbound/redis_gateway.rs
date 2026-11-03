use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use redis::aio::MultiplexedConnection;
use redis::{ConnectionAddr, ConnectionInfo, RedisConnectionInfo, Value};

use crate::domain::{ConnectionConfig, Reply, Tls};
use crate::ports::{RedisGateway, RedisSession};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// The `redis` crate over tokio and rustls. One connection per session,
/// RESP2, every call bounded by a response timeout; the connection info is
/// built from parts, never from a URL string, so no error carries the
/// password.
pub struct RedisCrateGateway {
    config: ConnectionConfig,
}

impl RedisCrateGateway {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config }
    }

    fn info(&self) -> ConnectionInfo {
        let addr = match self.config.tls {
            Tls::Off => ConnectionAddr::Tcp(self.config.host.clone(), self.config.port),
            Tls::On | Tls::Insecure => ConnectionAddr::TcpTls {
                host: self.config.host.clone(),
                port: self.config.port,
                insecure: self.config.tls == Tls::Insecure,
                tls_params: None,
            },
        };
        ConnectionInfo {
            addr,
            redis: RedisConnectionInfo {
                db: self.config.db,
                username: self.config.username.clone(),
                password: self.config.password().map(str::to_string),
                protocol: redis::ProtocolVersion::RESP2,
            },
        }
    }
}

struct Session {
    connection: MultiplexedConnection,
}

fn convert(value: Value) -> Reply {
    match value {
        Value::Nil => Reply::Nil,
        Value::Int(n) => Reply::Int(n),
        Value::Boolean(b) => Reply::Int(i64::from(b)),
        Value::Double(d) => Reply::Text(d.to_string()),
        Value::Okay => Reply::Text("OK".into()),
        Value::SimpleString(text) | Value::VerbatimString { text, .. } => Reply::Text(text),
        Value::BulkString(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Reply::Text(text),
            Err(error) => Reply::Bytes(
                error
                    .into_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            ),
        },
        Value::BigNumber(n) => Reply::Text(n.to_string()),
        Value::Array(items) | Value::Set(items) | Value::Push { data: items, .. } => {
            Reply::Array(items.into_iter().map(convert).collect())
        }
        Value::Map(pairs)
        | Value::Attribute {
            data: _,
            attributes: pairs,
        } => Reply::Array(
            pairs
                .into_iter()
                .flat_map(|(k, v)| [convert(k), convert(v)])
                .collect(),
        ),
        Value::ServerError(error) => {
            Reply::Text(format!("ERR {}", error.details().unwrap_or("error")))
        }
    }
}

fn describe(error: &redis::RedisError) -> String {
    match error.code() {
        Some(code) => format!("{code} {}", error.detail().unwrap_or(""))
            .trim()
            .to_string(),
        None => error.to_string(),
    }
}

#[async_trait]
impl RedisGateway for RedisCrateGateway {
    async fn connect(&self) -> Result<Box<dyn RedisSession>> {
        let client = redis::Client::open(self.info()).map_err(|error| {
            anyhow!("client for {}: {}", self.config.display(), describe(&error))
        })?;
        let connection = client
            .get_multiplexed_tokio_connection_with_response_timeouts(
                RESPONSE_TIMEOUT,
                Duration::from_secs(self.config.connect_timeout),
            )
            .await
            .map_err(|error| {
                anyhow!(
                    "connecting to {}: {}",
                    self.config.display(),
                    describe(&error)
                )
            })?;
        Ok(Box::new(Session { connection }))
    }
}

#[async_trait]
impl RedisSession for Session {
    async fn command(&mut self, args: &[String]) -> Result<Reply> {
        let Some((name, rest)) = args.split_first() else {
            return Err(anyhow!("an empty command"));
        };
        let mut command = redis::cmd(name);
        for argument in rest {
            command.arg(argument);
        }
        let value: Value =
            tokio::time::timeout(RESPONSE_TIMEOUT, command.query_async(&mut self.connection))
                .await
                .map_err(|_| anyhow!("{name} did not answer in {} s", RESPONSE_TIMEOUT.as_secs()))?
                .map_err(|error| anyhow!("{name}: {}", describe(&error)))?;
        Ok(convert(value))
    }

    async fn pipeline(&mut self, commands: &[Vec<String>]) -> Result<Vec<Reply>> {
        let mut pipeline = redis::pipe();
        for args in commands {
            let Some((name, rest)) = args.split_first() else {
                return Err(anyhow!("an empty command"));
            };
            let command = pipeline.cmd(name);
            for argument in rest {
                command.arg(argument);
            }
        }
        let values: Vec<Value> =
            tokio::time::timeout(RESPONSE_TIMEOUT, pipeline.query_async(&mut self.connection))
                .await
                .map_err(|_| {
                    anyhow!(
                        "the pipeline did not answer in {} s",
                        RESPONSE_TIMEOUT.as_secs()
                    )
                })?
                .map_err(|error| anyhow!("pipeline: {}", describe(&error)))?;
        Ok(values.into_iter().map(convert).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(tls: &str, url: &str) -> ConnectionConfig {
        ConnectionConfig::from_vars(|name| match name {
            "REDIS_URL" => Some(url.into()),
            "REDIS_TLS" => Some(tls.into()),
            "REDIS_CONNECT_TIMEOUT" => Some("1".into()),
            _ => None,
        })
        .unwrap()
    }

    #[test]
    fn replies_are_converted_and_the_connection_info_carries_the_parts() {
        assert_eq!(convert(Value::Okay), Reply::Text("OK".into()));
        assert_eq!(convert(Value::Boolean(true)), Reply::Int(1));
        assert_eq!(convert(Value::Double(1.5)), Reply::Text("1.5".into()));
        assert_eq!(
            convert(Value::BulkString(vec![0xff])),
            Reply::Bytes("ff".into())
        );
        assert_eq!(
            convert(Value::Map(vec![(
                Value::SimpleString("a".into()),
                Value::Int(1)
            )])),
            Reply::Array(vec![Reply::Text("a".into()), Reply::Int(1)])
        );
        assert_eq!(convert(Value::Nil), Reply::Nil);
        let gateway = RedisCrateGateway::new(config("insecure", "redis://bot:p@127.0.0.1:9/2"));
        let info = gateway.info();
        assert_eq!(info.redis.db, 2);
        assert_eq!(info.redis.username.as_deref(), Some("bot"));
        assert!(matches!(
            info.addr,
            ConnectionAddr::TcpTls { insecure: true, .. }
        ));
        let plain = RedisCrateGateway::new(config("off", "redis://127.0.0.1:9")).info();
        assert!(matches!(plain.addr, ConnectionAddr::Tcp(_, 9)));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_said_so_without_the_password() {
        for tls in ["off", "insecure"] {
            let gateway = RedisCrateGateway::new(config(tls, "redis://bot:s3cret@127.0.0.1:9/0"));
            let error = gateway
                .connect()
                .await
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(
                error.contains("connecting to bot@127.0.0.1:9/0"),
                "{tls}: {error}"
            );
            assert!(!error.contains("s3cret"));
        }
    }

    /// The real thing, against a server `REDIS_MCP_TEST_URL` names; skipped otherwise.
    #[tokio::test]
    async fn against_a_live_server_commands_and_pipelines_come_back() {
        let Ok(url) = std::env::var("REDIS_MCP_TEST_URL") else {
            return;
        };
        let gateway = RedisCrateGateway::new(config("off", &url));
        let mut session = gateway.connect().await.unwrap();
        assert_eq!(
            session.command(&["PING".to_string()]).await.unwrap(),
            Reply::Text("PONG".into())
        );
        let replies = session
            .pipeline(&[
                vec!["DBSIZE".into()],
                vec!["TYPE".into(), "redis-mcp:none".into()],
            ])
            .await
            .unwrap();
        assert!(matches!(replies[0], Reply::Int(_)));
        assert_eq!(replies[1], Reply::Text("none".into()));
        let error = session
            .command(&["NOSUCHCOMMAND".to_string()])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("ERR"), "{error}");
        assert!(
            session.command(&["PING".to_string()]).await.is_ok(),
            "the session survives an error reply"
        );
        assert!(session.command(&[]).await.is_err());
    }
}
