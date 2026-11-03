//! Grafana, read only, for a diagnosis bot: the logs Loki holds, the metrics
//! Prometheus holds, the dashboards that say which of them matter and the
//! alerts that are firing. Everything goes through Grafana's HTTP API with
//! a service account token, Loki and Prometheus through the datasource
//! proxy, so one token and one URL reach them all.
//!
//! `domain` validates the time windows, the queries and the requests and
//! parses the answers into bounded summaries; `ports` is the one GET the use
//! cases need; `application` runs the queries, `doctor` and `discover`;
//! `adapters::outbound` is the HTTP client and `adapters::inbound` the MCP
//! catalogue and dispatch.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
