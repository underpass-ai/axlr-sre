//! One PostgreSQL database for a DBA bot, reached the way `psql` would be:
//! the libpq environment (`PGHOST`, `PGDATABASE`, `PGUSER`, a password file)
//! or `DATABASE_URL`, never a password in an answer. Reads run in a
//! read-only transaction with a statement timeout, a lock timeout and a
//! row cap, through a cursor so the server sends no more than is shown;
//! the fixed DBA reads (activity, locks, table and index statistics,
//! pg_stat_statements, settings, replication) are the questions the
//! catalog's `dba-postgres` bot asks. With `POSTGRES_WRITES`, maintenance
//! statements or DML run after a preview. No tunnel: the node reaches the
//! database.
//!
//! `domain` reads the connection, classifies a statement and refuses the
//! dangerous ones, bounds rows; `ports` is the one session the use cases
//! need; `application` runs the reads, the writes, `doctor` and `discover`;
//! `adapters::outbound` is `tokio-postgres` with rustls and
//! `adapters::inbound` the MCP catalogue and dispatch.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;
