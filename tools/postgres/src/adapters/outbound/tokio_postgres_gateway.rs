use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio_postgres::{Client, SimpleQueryMessage};

use crate::domain::{ConnectionConfig, SslMode};
use crate::ports::{Outcome, SqlGateway, SqlSession};

/// `tokio-postgres` over rustls. One connection per session, the simple
/// protocol so every cell arrives as text and any type decodes, the
/// connection task dropped with the session.
pub struct TokioPostgresGateway {
    config: ConnectionConfig,
}

impl TokioPostgresGateway {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config }
    }

    fn driver_config(&self) -> tokio_postgres::Config {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&self.config.host)
            .port(self.config.port)
            .dbname(&self.config.database)
            .user(&self.config.user)
            .connect_timeout(Duration::from_secs(self.config.connect_timeout))
            .application_name(&self.config.application_name)
            .ssl_mode(match self.config.ssl_mode {
                SslMode::Disable => tokio_postgres::config::SslMode::Disable,
                SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
                SslMode::Require | SslMode::VerifyFull => tokio_postgres::config::SslMode::Require,
            });
        if let Some(password) = self.config.password() {
            config.password(password);
        }
        config
    }

    /// The TLS client: for `prefer` and `require`, encryption without a
    /// certificate check, as libpq does; for `verify-full`, the system roots
    /// plus `PGSSLROOTCERT`, host name checked.
    fn tls(&self) -> Result<ClientConfig> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .context("TLS protocol versions")?;
        let config = if self.config.ssl_mode == SslMode::VerifyFull {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            if let Some(path) = &self.config.root_cert {
                let pem = std::fs::read(path)
                    .with_context(|| format!("PGSSLROOTCERT {}", path.display()))?;
                for cert in rustls_pemfile_certs(&pem) {
                    roots
                        .add(cert)
                        .with_context(|| format!("a certificate in {}", path.display()))?;
                }
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        } else {
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAny))
                .with_no_client_auth()
        };
        Ok(config)
    }
}

/// PEM certificates without a pemfile dependency: every BEGIN/END CERTIFICATE block.
fn rustls_pemfile_certs(pem: &[u8]) -> Vec<CertificateDer<'static>> {
    let text = String::from_utf8_lossy(pem);
    let mut certs = Vec::new();
    for block in text.split("-----BEGIN CERTIFICATE-----").skip(1) {
        let Some(end) = block.find("-----END CERTIFICATE-----") else {
            continue;
        };
        let body: String = block[..end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if let Some(der) = base64_decode(&body) {
            certs.push(CertificateDer::from(der));
        }
    }
    certs
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        if c == b'=' {
            break;
        }
        let value = TABLE.iter().position(|&t| t == c)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// libpq's `require`: the channel is encrypted, the peer is not checked.
#[derive(Debug)]
struct AcceptAny;

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

struct Session {
    client: Client,
    driver: tokio::task::JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

#[async_trait]
impl SqlGateway for TokioPostgresGateway {
    async fn connect(&self) -> Result<Box<dyn SqlSession>> {
        let config = self.driver_config();
        let (client, driver) = if self.config.ssl_mode == SslMode::Disable {
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(|error| anyhow!("connecting to {}: {error}", self.config.display()))?;
            (
                client,
                tokio::spawn(async move {
                    let _ = connection.await;
                }),
            )
        } else {
            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(self.tls()?);
            let (client, connection) = config
                .connect(tls)
                .await
                .map_err(|error| anyhow!("connecting to {}: {error}", self.config.display()))?;
            (
                client,
                tokio::spawn(async move {
                    let _ = connection.await;
                }),
            )
        };
        Ok(Box::new(Session { client, driver }))
    }
}

#[async_trait]
impl SqlSession for Session {
    async fn run(&mut self, sql: &str) -> Result<Outcome> {
        let messages = self.client.simple_query(sql).await.map_err(|error| {
            anyhow!(
                "{}",
                error.as_db_error().map_or_else(
                    || error.to_string(),
                    |db| format!(
                        "{}: {}{}",
                        db.code().code(),
                        db.message(),
                        db.hint().map(|h| format!(" ({h})")).unwrap_or_default()
                    )
                )
            )
        })?;
        let mut columns: Vec<String> = Vec::new();
        let mut rows = Vec::new();
        let mut command: Option<(String, Option<u64>)> = None;
        for message in messages {
            match message {
                SimpleQueryMessage::RowDescription(description) => {
                    columns = description.iter().map(|c| c.name().to_string()).collect();
                }
                SimpleQueryMessage::Row(row) => {
                    if columns.is_empty() {
                        columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                    }
                    rows.push(
                        (0..row.len())
                            .map(|i| row.get(i).map(str::to_string))
                            .collect(),
                    );
                }
                SimpleQueryMessage::CommandComplete(count) => {
                    command = Some((
                        sql.split_whitespace().next().unwrap_or("").to_uppercase(),
                        Some(count),
                    ));
                }
                _ => {}
            }
        }
        if !columns.is_empty() {
            return Ok(Outcome::Rows { columns, rows });
        }
        let (tag, affected) = command.unwrap_or_default();
        Ok(Outcome::Command { tag, affected })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Files;
    use std::path::Path;

    struct NoFiles;
    impl Files for NoFiles {
        fn read(&self, _: &Path) -> Option<(String, u32)> {
            None
        }
    }

    fn config(mode: &str, url: Option<&str>) -> ConnectionConfig {
        ConnectionConfig::from_vars(
            |name| match name {
                "DATABASE_URL" => url.map(str::to_string),
                "PGSSLMODE" => Some(mode.into()),
                _ => None,
            },
            &NoFiles,
        )
        .unwrap()
    }

    #[test]
    fn the_tls_configs_build_and_pem_blocks_are_decoded() {
        let url = Some("postgres://u:p@127.0.0.1:9/d");
        assert!(
            TokioPostgresGateway::new(config("require", url))
                .tls()
                .is_ok()
        );
        assert!(
            TokioPostgresGateway::new(config("verify-full", url))
                .tls()
                .is_ok()
        );
        let mut with_cert = config("verify-full", url);
        with_cert.root_cert = Some(std::path::PathBuf::from("/nope/ca.pem"));
        assert!(TokioPostgresGateway::new(with_cert).tls().is_err());
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("!!"), None);
        assert_eq!(
            rustls_pemfile_certs(
                b"-----BEGIN CERTIFICATE-----\naGVsbG8=\n-----END CERTIFICATE-----\n"
            )
            .len(),
            1
        );
        assert!(rustls_pemfile_certs(b"-----BEGIN CERTIFICATE-----\nno end").is_empty());
        let verifier = AcceptAny;
        assert!(!verifier.supported_verify_schemes().is_empty());
        let driver = TokioPostgresGateway::new(config("disable", url)).driver_config();
        assert_eq!(driver.get_dbname(), Some("d"));
        assert_eq!(driver.get_application_name(), Some("postgres-mcp"));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_said_so() {
        for mode in ["disable", "prefer"] {
            let gateway = TokioPostgresGateway::new(config(
                mode,
                Some("postgres://u:p@127.0.0.1:9/d?connect_timeout=1"),
            ));
            let error = gateway
                .connect()
                .await
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(
                error.contains("connecting to u@127.0.0.1:9/d"),
                "{mode}: {error}"
            );
            assert!(!error.contains(":p@"), "no password in the error");
        }
    }

    /// The real thing, against a server `POSTGRES_MCP_TEST_URL` names; skipped otherwise.
    #[tokio::test]
    async fn against_a_live_server_rows_commands_and_errors_come_back() {
        let Ok(url) = std::env::var("POSTGRES_MCP_TEST_URL") else {
            return;
        };
        let gateway = TokioPostgresGateway::new(config("prefer", Some(&url)));
        let mut session = gateway.connect().await.unwrap();
        let rows = session
            .run("SELECT 1 AS one, NULL::text AS none, 'a''b' AS text")
            .await
            .unwrap();
        assert_eq!(
            rows,
            Outcome::Rows {
                columns: vec!["one".into(), "none".into(), "text".into()],
                rows: vec![vec![Some("1".into()), None, Some("a'b".into())]]
            }
        );
        assert_eq!(
            session.run("BEGIN").await.unwrap(),
            Outcome::Command {
                tag: "BEGIN".into(),
                affected: Some(0)
            }
        );
        let created = session.run("CREATE TEMP TABLE t (id int)").await.unwrap();
        assert!(matches!(created, Outcome::Command { .. }));
        assert_eq!(
            session.run("INSERT INTO t VALUES (1), (2)").await.unwrap(),
            Outcome::Command {
                tag: "INSERT".into(),
                affected: Some(2)
            }
        );
        session.run("ROLLBACK").await.unwrap();
        let error = session
            .run("SELECT * FROM nope_missing")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("42P01:"), "{error}");
        assert!(
            session.run("SELECT 1").await.is_ok(),
            "the session survives a failed statement"
        );
    }
}
