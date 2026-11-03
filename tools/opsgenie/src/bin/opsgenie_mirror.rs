use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use opsgenie_mcp::adapters::inbound::MirrorDaemon;
use opsgenie_mcp::adapters::outbound::{AxlrBotsCli, HttpOpsgenieClient, JsonStateFile};
use opsgenie_mcp::application::{MirrorIncidents, ReadAlerts};
use opsgenie_mcp::domain::{MirrorConfig, ServerConfig};
use opsgenie_mcp::ports::OpsgenieClient;

const USAGE: &str = "usage: opsgenie-mirror [--once] [--dry-run]\n\
  Every OPSGENIE_MIRROR_INTERVAL seconds, reads the open alerts of the scope\n\
  (OPSGENIE_URL, OPSGENIE_API_KEY, OPSGENIE_TEAMS, OPSGENIE_ESCALATIONS) and\n\
  mirrors them into axlr-bots (AXLR_BOTS, OPSGENIE_MIRROR_TENANT,\n\
  OPSGENIE_MIRROR_BOT): a new alert opens an incident, a closed one resolves\n\
  it. The first tick is a baseline. State in OPSGENIE_MIRROR_STATE.\n\
  --once: one tick, exit 1 on any failure. --dry-run: one tick that says\n\
  what it would do, calling nothing and saving nothing.";
const AXLR_BOTS_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> ExitCode {
    let mut once = false;
    let mut dry_run = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--once" => once = true,
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("opsgenie-mirror: unknown argument {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let server = match ServerConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("opsgenie-mirror: {error}");
            return ExitCode::from(2);
        }
    };
    let mirror = match MirrorConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("opsgenie-mirror: {error}");
            return ExitCode::from(2);
        }
    };
    let client: Arc<dyn OpsgenieClient> = match HttpOpsgenieClient::new(&server) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            eprintln!("opsgenie-mirror: HTTP client: {error:#}");
            return ExitCode::from(2);
        }
    };
    let read = Arc::new(ReadAlerts::new(client, server.scope().clone()));
    let sync = MirrorIncidents::new(
        read,
        Arc::new(AxlrBotsCli::new(
            mirror.axlr_bots().clone(),
            AXLR_BOTS_TIMEOUT,
        )),
        Arc::new(JsonStateFile::new(mirror.state().clone())),
        mirror.tenant().to_string(),
        mirror.bot().to_string(),
    );
    let daemon = MirrorDaemon::new(sync, mirror.interval(), dry_run);
    if once || dry_run {
        let ok = if dry_run {
            daemon.run_once(&mut std::io::stdout(), true).await
        } else {
            daemon.run_once(&mut std::io::stderr(), true).await
        };
        return if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    daemon.run_forever(&mut std::io::stderr()).await;
    ExitCode::SUCCESS
}
