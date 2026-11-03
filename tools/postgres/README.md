# postgres-mcp

One PostgreSQL database for a DBA bot, reached the way `psql` would be: the libpq environment of
the node (`PGHOST`, `PGDATABASE`, `PGUSER`, a password file) or `DATABASE_URL`, never a password
in an answer. Bounded read-only queries, plans, and the DBA's questions ready-made: activity,
locks, table and index statistics, `pg_stat_statements`, settings, replication. With
`POSTGRES_WRITES`, maintenance statements or any DML after a preview. No tunnel: the node reaches
the database.

```text
bot ──▶ postgres-mcp ──tokio-postgres + rustls──▶ the database (PG* or DATABASE_URL)
            read:  BEGIN READ ONLY; SET LOCAL statement_timeout, lock_timeout; DECLARE … CURSOR; FETCH n+1; ROLLBACK
            write: preview (relation facts, EXPLAIN rolled back) → confirm=true → the statement, alone or in one transaction
```

This is the MCP route to what the catalog's `dba-postgres` bot does today through `psql -X -At -c`:
its reads (SELECT, EXPLAIN, SHOW, auto-approved) are `query`, `explain` and the fixed reads; its
maintenance writes (VACUUM, ANALYZE, REINDEX, CREATE INDEX CONCURRENTLY, ALTER, each with an
approval) are `execute` at the `maintenance` level. The bot definition in axlr-bots is not changed
by this crate.

## Install and register

```bash
make -C tools install        # builds in release and copies postgres-mcp to ~/.local/bin
```

```json
{
  "command": "/home/<user>/.local/bin/postgres-mcp",
  "env": {"PGHOST": "db.internal", "PGDATABASE": "orders", "PGUSER": "dba_ro", "PGPASSFILE": "/etc/axlr/pgpass", "PGSSLMODE": "verify-full", "POSTGRES_WRITES": "maintenance"}
}
```

## Configuration

| variable                               | meaning                                                                                         |
|----------------------------------------|-------------------------------------------------------------------------------------------------|
| `DATABASE_URL`                         | `postgres://user:password@host:5432/db?sslmode=…&sslrootcert=…&connect_timeout=…`; wins over the `PG*` variables |
| `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER` | the connection; port 5432 when unset                                                        |
| `PGPASSWORD`, `PGPASSFILE`             | the password, or the file (`~/.pgpass` when unset): `host:port:db:user:password`, `*` wildcards. A file readable by group or others is ignored, as libpq ignores it, and `doctor` says so |
| `PGSSLMODE`                            | `disable`; `prefer` (the default) and `require`: encrypted, certificate not checked, as libpq; `verify-full`: the system roots plus `PGSSLROOTCERT`, host name checked. An RDS database needs `verify-full` with its bundle |
| `PGCONNECT_TIMEOUT`, `PGAPPNAME`       | seconds to connect (10), the `application_name` (`postgres-mcp`)                                |
| `POSTGRES_WRITES`                      | `off` (unset), `maintenance` or `all`: what `execute` may run                                   |

No host, database, user or port is built in. The password never appears in an answer, a log or
an error: `doctor` shows `user@host:port/db (sslmode)` and where the password came from.

## Tools

Answers are JSON. A failure is `isError` with `error` and, when there is one, `hint`. Every cell
is text, as `psql -At` gives it, cut at 4 KiB; an answer holds 256 KiB at most.

| tool           | arguments                                                        | answer                                                                                       |
|----------------|------------------------------------------------------------------|----------------------------------------------------------------------------------------------|
| `doctor`       | —                                                                | the connection without the password, where the password comes from, the server and the user it answers as, the role's powers, `pg_stat_statements`, the write level |
| `discover`     | `schema`?                                                        | size, schemas, the twenty largest tables, extensions, the settings that matter, what reads and what writes |
| `query`        | `sql`, `max_rows` (200, ≤1000), `timeout_seconds` (30, ≤300)     | columns, rows, `truncated` (one row past the cap was fetched, no more), `cut`               |
| `explain`      | `sql`, `analyze`, `format` text\|json, `timeout_seconds`         | the plan as lines, or parsed                                                                 |
| `activity`     | `include_idle`, `limit`                                          | sessions other than this one, longest transaction first, with wait events                    |
| `locks`        | `limit`                                                          | every blocked backend and the backends that block it (`pg_blocking_pids`)                    |
| `table_stats`  | `schema`?, `order` dead\|size\|seq_scans\|writes, `limit`        | live and dead rows, scans, writes, last vacuum and analyze                                    |
| `index_stats`  | `schema`?, `unused_only`, `limit`                                | size, scans, valid, unique, definition; invalid indexes first                                |
| `statements`   | `order` total\|mean\|calls\|rows, `limit`                        | `pg_stat_statements` with the time columns as this version names them; says when not installed. Other users' queries read `<insufficient privilege>` unless the role has `pg_read_all_stats` |
| `settings`     | `names`?, `prefix`?                                              | value, unit, source                                                                          |
| `replication`  | —                                                                | the standbys and their lag from a primary; the replay delay on a standby                     |
| `execute`      | `sql`, `timeout_seconds` (60), `confirm`                         | preview: kind, target relation with size and statistics, the plan for DML; done: command tag, rows affected, `RETURNING` rows |
| `terminate`    | `pid`, `cancel`, `confirm`                                       | preview: the backend as `activity` shows it; done: whether it was signalled. Client sessions only, never a background process |

**Reads.** One statement per call (strings, quoted identifiers, dollar quotes and comments are
walked, so a `;` inside them does not count), no parameters. It runs in `BEGIN … READ ONLY` with
`SET LOCAL statement_timeout` and `lock_timeout = 5s`, so it neither writes, nor runs for long,
nor waits behind DDL; a SELECT goes through a `NO SCROLL` cursor and `FETCH max_rows + 1`, so the
server sends no more than is shown. Functions that act on the server are refused by name:
`pg_terminate_backend`, `pg_cancel_backend`, `pg_reload_conf`, `pg_switch_wal`, `pg_promote`,
the replication slot functions, `lo_*`, `pg_read_file`, `pg_read_binary_file`, `pg_ls_dir`,
`pg_stat_file`, `dblink`. `terminate` is the one door for backends.

**Writes.** Published only with `POSTGRES_WRITES`; otherwise a call is refused with the way to
enable them. `maintenance` allows VACUUM, ANALYZE, REINDEX, CREATE INDEX **CONCURRENTLY** (a
plain CREATE INDEX locks the table and is refused), ALTER of a table, index, sequence or view
without a DROP in it (ALTER … DROP COLUMN, ALTER ROLE, ALTER DATABASE and ALTER SYSTEM need
`all`), CLUSTER and REFRESH MATERIALIZED VIEW. `all` allows any statement but a read, DROP, TRUNCATE, DELETE, UPDATE and
INSERT included, and `terminate`. Transaction control, SET, COPY and LISTEN are never run. Each
write previews unless `confirm=true`: the preview reads the relation's facts and, for DML, runs
`EXPLAIN` in a transaction that is rolled back; it writes nothing (tested with doubles).
Confirmed, a statement that refuses a transaction block (VACUUM, CREATE INDEX CONCURRENTLY,
REINDEX, ALTER SYSTEM, CLUSTER) runs alone; the rest run in one transaction, committed on success
and rolled back on failure.

**Cost.** Every call opens one connection and drops it: no pool, no state between calls. A
`query` of a large table costs what the cursor fetches, not the table.

## Layout

```text
src/
├── bin/postgres_mcp.rs   composition
├── domain/               ConnectionConfig (PG* and DATABASE_URL), the password file, SslMode,
│                         Statement (the scanner and the classifier), RowSet, the fixed DBA reads
├── ports/                SqlGateway, SqlSession, Outcome
├── application/          RunQuery, InspectServer, ChangeServer, DiagnoseSetup, DiscoverScope
└── adapters/
    ├── inbound/          tool catalogue, PostgresMcpHost
    └── outbound/         TokioPostgresGateway (simple protocol, rustls)
```

## Verified

Against PostgreSQL 16.15 in Docker (`postgres:16-alpine` with `pg_stat_statements`, a `demo`
schema of 500 rows), 3 November 2026: every tool over stdio at the three write levels; the cursor
cap; the statement timeout; the refusals (a server function, two statements, DML as a read, a
plain CREATE INDEX at `maintenance`, DML at `maintenance`); VACUUM outside a transaction; CREATE
INDEX CONCURRENTLY; UPDATE previewed with its plan then committed; INSERT … RETURNING; `terminate`
of a `pg_sleep` backend, which ended its `psql`; `sslmode=require` refused by a server without
TLS. The gateway's own test runs against `POSTGRES_MCP_TEST_URL` when set and is skipped in CI.
Not exercised: `verify-full` against a real certificate, a standby, a server older than 13.

Out of scope here: SSM tunnels, Secrets Manager, per-environment profiles (the node reaches the
database and takes its credentials from the environment), Redis (next tool), DocumentDB.
