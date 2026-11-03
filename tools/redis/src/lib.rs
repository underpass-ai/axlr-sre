//! One Redis or Valkey instance for a DBA bot, reached the way `redis-cli`
//! would be: the node's environment (`REDIS_URL`, or host, port, user,
//! password and TLS mode), never a password in an answer. The reads are
//! the questions the catalog's `dba-redis` bot asks: INFO, the slow log,
//! the clients, latency, memory, config, cluster, keys by SCAN (never
//! KEYS) with bounded previews, the biggest keys. With `REDIS_WRITES`,
//! expire, delete, config set (an allowlist of parameters) and client kill
//! after a preview. FLUSH, SHUTDOWN, DEBUG and data writes are never run.
//!
//! `domain` reads the connection, parses the replies, guards the patterns
//! and the parameters; `ports` is the one session the use cases need;
//! `application` runs the reads, the writes, `doctor` and `discover`;
//! `adapters::outbound` is the `redis` crate with rustls and
//! `adapters::inbound` the MCP catalogue and dispatch.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
