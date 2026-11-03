//! Opsgenie for a triage bot, through its REST API with an API key: the open
//! alerts of a scope (teams, escalations), one alert and its notes, and, with
//! `OPSGENIE_WRITES`, acknowledge, note and close, each previewed first. The
//! same crate builds `opsgenie-mirror`, the daemon that opens an axlr-bots
//! incident for every new alert of the scope and resolves it when the alert
//! closes: the "mirror" input of the design.
//!
//! `domain` validates the configuration, the scope, the alert references
//! and the notes, parses Opsgenie's answers and plans the mirror's work;
//! `ports` are the HTTP calls, the incident sink and the state store the
//! use cases need; `application` runs the reads, the writes, `doctor`,
//! `discover` and the mirror's tick; `adapters::outbound` are the HTTP
//! client, the `axlr-bots` subprocess and the state file; `adapters::inbound`
//! the MCP catalogue and dispatch and the daemon loop.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
