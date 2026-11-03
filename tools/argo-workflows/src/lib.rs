//! Argo Workflows, read only, for a diagnosis bot: which workflow failed, what
//! it said, which template or cron it came from.
//!
//! `domain` validates the queries and the configuration; `ports` says what the
//! use cases need from Argo; `application` summarises listings and runs
//! `doctor` and `discover`; `adapters::outbound` is the HTTP client of Argo
//! Server and `adapters::inbound` the MCP catalogue and dispatch.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
