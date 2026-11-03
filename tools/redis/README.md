# redis-mcp

One Redis or Valkey instance for a DBA bot, reached the way `redis-cli` would be: the node's
environment (`REDIS_URL`, or host, port, user, password and TLS mode), never a password in an
answer. The reads are the questions the catalog's `dba-redis` bot asks: INFO, the slow log, the
clients, latency, memory, config, cluster, keys by SCAN (never KEYS) with bounded previews, the
biggest keys. With `REDIS_WRITES=true`, expire, delete, config set (an allowlist of parameters)
and client kill after a preview. FLUSHALL, FLUSHDB, SHUTDOWN, DEBUG and data writes have no tool.

```text
bot ──▶ redis-mcp ──redis crate + rustls──▶ the instance (REDIS_* or REDIS_URL)
          read:  one connection per call, RESP2, a response timeout on every command
          write: preview (the key's facts, the current value, the client) → confirm=true → the command
```

This is the MCP route to what the catalog's `dba-redis` bot does today through `redis-cli`: the
same allow shape (INFO, SLOWLOG, CLIENT LIST, MEMORY, LATENCY, DBSIZE, SCAN, TTL, TYPE, OBJECT,
CONFIG GET, CLUSTER as reads; CONFIG SET, EXPIRE, DEL and CLIENT KILL as writes with an approval)
and the same deny list (FLUSH*, SHUTDOWN, DEBUG, KEYS). The bot definition in axlr-bots is not
changed by this crate.

## Install and register

```bash
make -C tools install        # builds in release and copies redis-mcp to ~/.local/bin
```

```json
{
  "command": "/home/<user>/.local/bin/redis-mcp",
  "env": {"REDIS_HOST": "cache.internal", "REDIS_TLS": "on", "REDISCLI_AUTH": "<password>", "REDIS_WRITES": "true"}
}
```

## Configuration

| variable                               | meaning                                                                                    |
|----------------------------------------|--------------------------------------------------------------------------------------------|
| `REDIS_URL`                            | `redis://user:password@host:6379/0`; `rediss://` for TLS, `#insecure` to skip the certificate check. Wins over the variables below for host, port, db, user and password |
| `REDIS_HOST`, `REDIS_PORT`, `REDIS_DB` | the instance; port 6379 and db 0 when unset                                                |
| `REDIS_USERNAME`, `REDIS_PASSWORD` or `REDISCLI_AUTH` | the credentials; the password never appears in an answer, a log or an error |
| `REDIS_TLS`                            | `off` (the default), `on` (certificate checked against the system roots), `insecure` (encrypted, not checked: a tunnel to localhost, a self-signed server) |
| `REDIS_CONNECT_TIMEOUT`                | seconds to connect, 5 when unset; every command has a 30 s response timeout                |
| `REDIS_WRITES`                         | `true` publishes the four writes; off when unset                                           |

No host, port, database or key prefix is built in. `doctor` shows `user@host:port/db (tls …)`.

**One node.** The plain client sees one instance. In cluster mode `keys`, `biggest` and `delete`
cover this node's slots only; a `MOVED` error names the owner of a key. `cluster` still answers
with CLUSTER INFO and NODES, and `doctor` says when cluster mode is on.

## Tools

Answers are JSON. A failure is `isError` with `error` and, when there is one, `hint`. Every text
is cut at 4 KiB.

| tool          | arguments                                                        | answer                                                                                         |
|---------------|------------------------------------------------------------------|------------------------------------------------------------------------------------------------|
| `doctor`      | —                                                                | the connection without the password, whether the server answers and what it is, role, memory limits, cluster mode, the write switch |
| `discover`    | —                                                                | version, role, memory, clients, stats, persistence, keyspace per database, what reads and what writes |
| `info`        | `section`                                                        | INFO parsed by section into key:value                                                          |
| `slowlog`     | `count` (25, ≤128)                                               | the slowest commands with microseconds and client, the log's length, the threshold             |
| `clients`     | —                                                                | CLIENT LIST parsed; blocked clients and replica links counted                                  |
| `latency`     | —                                                                | LATENCY LATEST events with latest and max milliseconds; the threshold (0 means off)            |
| `memory`      | —                                                                | MEMORY DOCTOR, MEMORY STATS, used and peak, maxmemory and its policy, fragmentation, used percent of max |
| `config_get`  | `pattern`                                                        | CONFIG GET by name or glob                                                                     |
| `cluster`     | —                                                                | CLUSTER INFO and NODES, or that cluster mode is off                                            |
| `keys`        | `pattern`, `value_contains`?, `ignore_case`, `with_values`, `limit` (100, ≤1000) | keys through SCAN with COUNT 500, `examined`, `complete`; with values, each key's type, TTL, encoding, memory and preview |
| `key`         | `key`                                                            | type, TTL, encoding, memory usage, a bounded preview, idle time, LFU frequency                 |
| `biggest`     | `pattern`?, `limit` (20, ≤200), `sample` (5000, ≤20000)          | the largest keys of the sample by MEMORY USAGE, bytes per type, `keyspace` from DBSIZE, `sampled`, `complete` |
| `expire`      | `key`, `seconds`, `confirm`                                      | preview: the key's facts; done: whether the TTL was set                                        |
| `delete`      | `keys`? or `pattern` + `value_contains`?, `ignore_case`, `confirm` | preview: the keys selected with their facts; done: `deleted`, `skipped`, by UNLINK            |
| `config_set`  | `parameter`, `value`, `confirm`                                  | preview: the current value; done: the value now                                                |
| `client_kill` | `id`, `confirm`                                                  | preview: the client as CLIENT LIST shows it; done: `killed`                                    |

**Previews are bounded.** A string shows its first 64 KiB; a list, set, sorted set, hash or
stream its first 50 elements, read with LRANGE, SSCAN, ZRANGE, HSCAN and XRANGE with a COUNT,
never the whole collection. The value filter of `keys` and `delete` sees that preview, and says
so. SCAN examines 50 000 keys at most per call; `examined` and `complete` say how far it got.

**Cost.** `keys` with a value filter and `delete` by pattern read two round trips per examined
key; at the 50 000-key cap that is a minute or more over a network. A narrower pattern is the
lever.

**Patterns.** Wildcards alone (`*`, `**`, `*:*`, `?*`) select every key and are refused unless a
value filter narrows them. `biggest` takes `*`, since it measures a bounded sample.

**Writes.** Published only with `REDIS_WRITES=true`; otherwise a call is refused with the way to
enable them. Each previews unless `confirm=true`, and a preview writes nothing (tested with
doubles).

- `delete`'s preview carries the keys it selected; confirm with that list to delete exactly
  those. A confirmed call with the pattern instead re-selects and re-checks the value filter on
  each key right before UNLINK, and reports `selected`, `deleted`, `skipped`. 1 000 keys at most.
  UNLINK one key per command in a pipeline: no CROSSSLOT on a cluster node.
- `config_set` changes only `maxmemory`, `maxmemory-policy`, `maxmemory-samples`, `timeout`,
  `tcp-keepalive`, `slowlog-log-slower-than`, `slowlog-max-len`, `latency-monitor-threshold`,
  `maxclients`, `lazyfree-lazy-*` and `notify-keyspace-events`, each value checked for its shape.
  `dir`, `dbfilename`, `bind`, `port`, `appendonly`, `save`, `requirepass`, `masterauth` and
  the rest stay with a person. The file on disk is not rewritten.
- `client_kill` uses `CLIENT KILL ID`, never this connection, never a client whose flags carry
  `S` or `M` (a replication link).

## Layout

```text
src/
├── bin/redis_mcp.rs      composition; installs the rustls provider once
├── domain/               ConnectionConfig, Tls, Reply and the parsers (INFO, CLIENT LIST, SLOWLOG,
│                         LATENCY, CLUSTER NODES), key patterns and value filters, the CONFIG SET
│                         allowlist, previews
├── ports/                RedisGateway, RedisSession
├── application/          InspectInstance, InspectKeys, ChangeInstance, DiagnoseSetup, DiscoverScope
└── adapters/
    ├── inbound/          tool catalogue, RedisMcpHost
    └── outbound/         RedisCrateGateway (the redis crate over tokio and rustls)
```

## Verified

Against Valkey 8.1.5 in Docker (`valkey/valkey:8.1.5-alpine` with a password, a 64 MB
maxmemory, keys of every type under `demo:`, a blocked BLPOP client), 3 November 2026: every
tool over stdio with writes off and on; the value filter; a wildcard pattern refused; a key of
each type previewed; `biggest`; `expire`; `delete` by preview then by list; `config_set` of an
allowed parameter and `dir` refused; `client_kill` of the blocked client, which ended it;
`REDIS_TLS=insecure` against a plain server answers a readable error. The gateway's own test
runs against `REDIS_MCP_TEST_URL` when set and is skipped in CI. Not exercised: TLS against a
real certificate, cluster mode, a replica.

Out of scope here: SSM tunnels and per-environment ports (the node reaches the instance), a free
command tool, DocumentDB.
