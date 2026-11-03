//! AWS, read only, for a cloud diagnosis bot, through the `aws` CLI: the
//! metrics and alarms CloudWatch holds, the Lambda functions with what runs
//! (alias), where they log (log group) and what triggers them, and the log
//! events themselves. Every call names its region, checked against the
//! server's allowlist; every answer is bounded; nothing writes.
//!
//! `domain` validates the regions, the names, the windows and the metric
//! queries, builds the CLI invocations, parses the answers into bounded
//! summaries and translates the CLI's failures; `ports` is the one runner
//! the use cases need; `application` runs the reads, `doctor` and
//! `discover`; `adapters::outbound` spawns `aws` and `adapters::inbound` is
//! the MCP catalogue and dispatch.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
