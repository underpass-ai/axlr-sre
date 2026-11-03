//! The server every tool of this workspace is built on.
//!
//! A tool implements [`ToolHost`]: its name, version, instructions, catalogue
//! and `call`. [`StdioServer`] serves that host over stdio with `rmcp`, the
//! official MCP SDK, which answers `initialize`, `tools/list`, `tools/call`,
//! `ping`, the prompts and the protocol version negotiation. No tool
//! implements JSON-RPC by hand.
//!
//! Every tool exposes `doctor` and `discover` ([`ToolDefinition::doctor`],
//! [`ToolDefinition::discover`]): the first says whether the server can work
//! and what to do when it cannot, without showing secrets or touching
//! anything; the second returns a bounded inventory of what the server can
//! read or operate.

pub mod adapters;
pub mod domain;
pub mod ports;

pub use adapters::mcp::StdioServer;
pub use domain::{Arguments, OutputFormat, ToolDefinition, ToolOutcome};
pub use ports::ToolHost;
