use super::InputError;

/// A strict identifier for an interpolated position: letters, digits and
/// `_`, 63 bytes, not starting with a digit.
pub fn identifier(what: &'static str, value: &str) -> Result<String, InputError> {
    let value = value.trim();
    let ok = !value.is_empty()
        && value.len() <= 63
        && !value.starts_with(|c: char| c.is_ascii_digit())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(value.to_string())
    } else {
        Err(InputError::Invalid {
            what,
            value: value.chars().take(60).collect(),
            rule: "letters, digits and `_`, 63 at most",
        })
    }
}

/// A text as a SQL literal: quotes doubled, control characters refused.
pub fn literal(what: &'static str, value: &str) -> Result<String, InputError> {
    if value.chars().any(char::is_control) || value.len() > 256 {
        return Err(InputError::Invalid {
            what,
            value: value.chars().take(60).collect(),
            rule: "printable text, 256 characters at most",
        });
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn limit(value: i64, max: i64) -> Result<i64, InputError> {
    if (1..=max).contains(&value) {
        Ok(value)
    } else {
        Err(InputError::OutOfRange {
            what: "limit",
            min: 1,
            max,
        })
    }
}

/// The fixed DBA reads, each a complete statement over the catalog views.
pub struct Fixed;

impl Fixed {
    pub const LIMIT_MAX: i64 = 200;

    pub const SERVER: &'static str = "SELECT version() AS version, current_user AS \"user\", current_database() AS database, pg_is_in_recovery() AS in_recovery, pg_backend_pid() AS pid, current_setting('server_version_num') AS version_num, current_setting('transaction_read_only') AS read_only";

    pub const DATABASE_SIZE: &'static str = "SELECT pg_size_pretty(pg_database_size(current_database())) AS size, pg_database_size(current_database()) AS bytes";

    pub const SCHEMAS: &'static str = "SELECT n.nspname AS schema, count(c.oid) FILTER (WHERE c.relkind IN ('r','p')) AS tables, count(c.oid) FILTER (WHERE c.relkind IN ('v','m')) AS views FROM pg_namespace n LEFT JOIN pg_class c ON c.relnamespace = n.oid WHERE n.nspname NOT IN ('pg_catalog','information_schema','pg_toast') AND n.nspname NOT LIKE 'pg_temp%' GROUP BY n.nspname ORDER BY n.nspname";

    pub const EXTENSIONS: &'static str =
        "SELECT extname AS name, extversion AS version FROM pg_extension ORDER BY extname";

    pub const KEY_SETTINGS: &'static str = "SELECT name, setting, unit FROM pg_settings WHERE name IN ('max_connections','shared_buffers','work_mem','maintenance_work_mem','effective_cache_size','statement_timeout','lock_timeout','idle_in_transaction_session_timeout','autovacuum','wal_level','max_wal_size','log_min_duration_statement') ORDER BY name";

    pub const STATEMENTS_COLUMNS: &'static str = "SELECT column_name FROM information_schema.columns WHERE table_name = 'pg_stat_statements' AND column_name IN ('total_exec_time','total_time','mean_exec_time','mean_time')";

    pub const REPLICATION: &'static str = "SELECT client_addr::text AS client, application_name, state, sync_state, pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn) AS replay_lag_bytes, write_lag::text, flush_lag::text, replay_lag::text FROM pg_stat_replication ORDER BY application_name";

    pub const RECOVERY: &'static str = "SELECT pg_is_in_recovery() AS in_recovery, CASE WHEN pg_is_in_recovery() THEN extract(epoch FROM now() - pg_last_xact_replay_timestamp())::int END AS replay_delay_seconds, CASE WHEN pg_is_in_recovery() THEN pg_last_wal_receive_lsn()::text END AS receive_lsn, CASE WHEN pg_is_in_recovery() THEN pg_last_wal_replay_lsn()::text END AS replay_lsn";

    /// Top tables by total size.
    pub fn tables(schema: Option<&str>, limit_value: i64) -> Result<String, InputError> {
        let filter = match schema {
            Some(schema) => format!(
                "AND n.nspname = {}",
                literal("schema", &identifier("schema", schema)?)?
            ),
            None => String::new(),
        };
        Ok(format!(
            "SELECT n.nspname AS schema, c.relname AS table, pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size, pg_total_relation_size(c.oid) AS total_bytes, c.reltuples::bigint AS estimated_rows FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r','p','m') AND n.nspname NOT IN ('pg_catalog','information_schema','pg_toast') {filter} ORDER BY pg_total_relation_size(c.oid) DESC LIMIT {}",
            limit(limit_value, Self::LIMIT_MAX)?
        ))
    }

    /// Sessions other than ours, longest-running first; idle ones on request.
    pub fn activity(include_idle: bool, limit_value: i64) -> Result<String, InputError> {
        let idle = if include_idle {
            ""
        } else {
            "AND state <> 'idle'"
        };
        Ok(format!(
            "SELECT pid, usename AS \"user\", application_name, client_addr::text AS client, state, wait_event_type, wait_event, extract(epoch FROM now() - xact_start)::int AS transaction_seconds, extract(epoch FROM now() - query_start)::int AS query_seconds, extract(epoch FROM now() - state_change)::int AS state_seconds, backend_type, left(query, 500) AS query FROM pg_stat_activity WHERE pid <> pg_backend_pid() AND backend_type = 'client backend' {idle} ORDER BY xact_start NULLS LAST LIMIT {}",
            limit(limit_value, Self::LIMIT_MAX)?
        ))
    }

    /// Who waits on whom: every blocked backend with the pids that block it.
    pub fn locks(limit_value: i64) -> Result<String, InputError> {
        Ok(format!(
            "SELECT a.pid AS blocked_pid, a.usename AS blocked_user, extract(epoch FROM now() - a.query_start)::int AS blocked_seconds, left(a.query, 300) AS blocked_query, pg_blocking_pids(a.pid) AS blocking_pids, b.pid AS blocking_pid, b.usename AS blocking_user, b.state AS blocking_state, extract(epoch FROM now() - b.xact_start)::int AS blocking_transaction_seconds, left(b.query, 300) AS blocking_query FROM pg_stat_activity a JOIN LATERAL unnest(pg_blocking_pids(a.pid)) AS blocker(pid) ON true JOIN pg_stat_activity b ON b.pid = blocker.pid WHERE cardinality(pg_blocking_pids(a.pid)) > 0 ORDER BY a.query_start LIMIT {}",
            limit(limit_value, Self::LIMIT_MAX)?
        ))
    }

    pub const TABLE_ORDERS: &'static str = "dead, size, seq_scans, writes";

    /// Table statistics, ordered by what hurts: dead tuples, size, sequential scans or writes.
    pub fn table_stats(
        schema: Option<&str>,
        order: &str,
        limit_value: i64,
    ) -> Result<String, InputError> {
        let order_by = match order {
            "dead" => "s.n_dead_tup DESC",
            "size" => "pg_total_relation_size(s.relid) DESC",
            "seq_scans" => "s.seq_scan DESC",
            "writes" => "(s.n_tup_ins + s.n_tup_upd + s.n_tup_del) DESC",
            other => {
                return Err(InputError::UnknownChoice {
                    what: "order",
                    value: other.chars().take(20).collect(),
                    allowed: Self::TABLE_ORDERS,
                });
            }
        };
        let filter = match schema {
            Some(schema) => format!(
                "WHERE s.schemaname = {}",
                literal("schema", &identifier("schema", schema)?)?
            ),
            None => String::new(),
        };
        Ok(format!(
            "SELECT s.schemaname AS schema, s.relname AS table, pg_size_pretty(pg_total_relation_size(s.relid)) AS total_size, s.n_live_tup AS live_rows, s.n_dead_tup AS dead_rows, CASE WHEN s.n_live_tup > 0 THEN round(100.0 * s.n_dead_tup / s.n_live_tup, 1) END AS dead_pct, s.seq_scan AS seq_scans, s.idx_scan AS index_scans, s.n_tup_ins AS inserts, s.n_tup_upd AS updates, s.n_tup_del AS deletes, s.last_vacuum::text, s.last_autovacuum::text, s.last_analyze::text, s.last_autoanalyze::text FROM pg_stat_user_tables s {filter} ORDER BY {order_by} LIMIT {}",
            limit(limit_value, Self::LIMIT_MAX)?
        ))
    }

    /// Indexes with their use; `unused_only` keeps those never scanned, and
    /// an invalid index (a failed CONCURRENTLY build) is always shown.
    pub fn index_stats(
        schema: Option<&str>,
        unused_only: bool,
        limit_value: i64,
    ) -> Result<String, InputError> {
        let mut filters = Vec::new();
        if let Some(schema) = schema {
            filters.push(format!(
                "s.schemaname = {}",
                literal("schema", &identifier("schema", schema)?)?
            ));
        }
        if unused_only {
            filters.push("(s.idx_scan = 0 OR NOT i.indisvalid)".to_string());
        }
        let filter = if filters.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filters.join(" AND "))
        };
        Ok(format!(
            "SELECT s.schemaname AS schema, s.relname AS table, s.indexrelname AS index, pg_size_pretty(pg_relation_size(s.indexrelid)) AS size, s.idx_scan AS scans, i.indisvalid AS valid, i.indisunique AS unique, i.indisprimary AS primary, pg_get_indexdef(s.indexrelid) AS definition FROM pg_stat_user_indexes s JOIN pg_index i ON i.indexrelid = s.indexrelid {filter} ORDER BY NOT i.indisvalid DESC, pg_relation_size(s.indexrelid) DESC LIMIT {}",
            limit(limit_value, Self::LIMIT_MAX)?
        ))
    }

    pub const STATEMENT_ORDERS: &'static str = "total, mean, calls, rows";

    /// `pg_stat_statements`, with the time columns named as this version
    /// names them (`total_exec_time` since 13, `total_time` before).
    pub fn statements(
        order: &str,
        limit_value: i64,
        total_column: &str,
        mean_column: &str,
    ) -> Result<String, InputError> {
        let total = identifier("column", total_column)?;
        let mean = identifier("column", mean_column)?;
        let order_by = match order {
            "total" => format!("{total} DESC"),
            "mean" => format!("{mean} DESC"),
            "calls" => "calls DESC".into(),
            "rows" => "rows DESC".into(),
            other => {
                return Err(InputError::UnknownChoice {
                    what: "order",
                    value: other.chars().take(20).collect(),
                    allowed: Self::STATEMENT_ORDERS,
                });
            }
        };
        Ok(format!(
            "SELECT queryid::text, calls, round({total}::numeric, 1) AS total_ms, round({mean}::numeric, 2) AS mean_ms, rows, shared_blks_hit, shared_blks_read, left(query, 500) AS query FROM pg_stat_statements ORDER BY {order_by} LIMIT {}",
            limit(limit_value, Self::LIMIT_MAX)?
        ))
    }

    /// Settings by exact names or by prefix.
    pub fn settings(names: &[String], prefix: Option<&str>) -> Result<String, InputError> {
        let mut filters = Vec::new();
        if !names.is_empty() {
            let list: Vec<String> = names
                .iter()
                .map(|n| identifier("setting", n).and_then(|i| literal("setting", &i)))
                .collect::<Result<_, _>>()?;
            filters.push(format!("name IN ({})", list.join(", ")));
        }
        if let Some(prefix) = prefix {
            let prefix = identifier("prefix", prefix)?;
            filters.push(format!(
                "name LIKE {}",
                literal("prefix", &format!("{prefix}%"))?
            ));
        }
        if filters.is_empty() {
            return Err(InputError::Empty("names or prefix"));
        }
        Ok(format!(
            "SELECT name, setting, unit, source, short_desc FROM pg_settings WHERE {} ORDER BY name LIMIT 200",
            filters.join(" OR ")
        ))
    }

    /// One backend, for `terminate`'s preview.
    pub fn backend(pid: i64) -> Result<String, InputError> {
        if !(1..=i64::from(i32::MAX)).contains(&pid) {
            return Err(InputError::OutOfRange {
                what: "pid",
                min: 1,
                max: i64::from(i32::MAX),
            });
        }
        Ok(format!(
            "SELECT pid, usename AS \"user\", application_name, client_addr::text AS client, state, wait_event_type, wait_event, extract(epoch FROM now() - xact_start)::int AS transaction_seconds, extract(epoch FROM now() - query_start)::int AS query_seconds, backend_type, left(query, 500) AS query FROM pg_stat_activity WHERE pid = {pid}"
        ))
    }

    /// The relation a write touches, for its preview.
    pub fn relation(qualified: &str) -> Result<String, InputError> {
        let (schema, table) = match qualified.split_once('.') {
            Some((schema, table)) => (
                Some(identifier("schema", schema)?),
                identifier("table", table)?,
            ),
            None => (None, identifier("table", qualified)?),
        };
        let filter = match schema {
            Some(schema) => format!(
                "n.nspname = {} AND c.relname = {}",
                literal("schema", &schema)?,
                literal("table", &table)?
            ),
            None => format!(
                "c.relname = {} AND n.nspname = ANY(current_schemas(false))",
                literal("table", &table)?
            ),
        };
        Ok(format!(
            "SELECT n.nspname AS schema, c.relname AS relation, c.relkind AS kind, pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size, c.reltuples::bigint AS estimated_rows, s.n_dead_tup AS dead_rows, s.last_autovacuum::text, s.last_analyze::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid WHERE {filter}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_literals_are_strict() {
        assert_eq!(identifier("schema", " demo_1 ").unwrap(), "demo_1");
        for bad in ["", "1x", "a-b", "a b", "a;", &"x".repeat(64)] {
            assert!(identifier("schema", bad).is_err(), "{bad}");
        }
        assert_eq!(literal("x", "it's").unwrap(), "'it''s'");
        assert!(literal("x", "a\nb").is_err());
        assert!(literal("x", &"a".repeat(257)).is_err());
    }

    #[test]
    fn the_fixed_reads_build_with_their_filters() {
        assert!(
            Fixed::tables(Some("demo"), 10)
                .unwrap()
                .contains("n.nspname = 'demo'")
        );
        assert!(Fixed::tables(None, 10).unwrap().contains("LIMIT 10"));
        assert!(Fixed::tables(None, 0).is_err());
        assert!(Fixed::tables(Some("bad schema"), 10).is_err());
        assert!(
            Fixed::activity(false, 50)
                .unwrap()
                .contains("state <> 'idle'")
        );
        assert!(
            !Fixed::activity(true, 50)
                .unwrap()
                .contains("state <> 'idle'")
        );
        assert!(Fixed::locks(20).unwrap().contains("pg_blocking_pids"));
        assert!(
            Fixed::table_stats(Some("demo"), "dead", 20)
                .unwrap()
                .contains("s.n_dead_tup DESC")
        );
        assert!(
            Fixed::table_stats(None, "size", 20)
                .unwrap()
                .contains("pg_total_relation_size")
        );
        assert!(
            Fixed::table_stats(None, "seq_scans", 20)
                .unwrap()
                .contains("seq_scan DESC")
        );
        assert!(
            Fixed::table_stats(None, "writes", 20)
                .unwrap()
                .contains("n_tup_del) DESC")
        );
        assert!(Fixed::table_stats(None, "bloat", 20).is_err());
        assert!(
            Fixed::index_stats(Some("demo"), true, 20)
                .unwrap()
                .contains("idx_scan = 0")
        );
        assert!(
            !Fixed::index_stats(None, false, 20)
                .unwrap()
                .contains("WHERE")
        );
        assert!(
            Fixed::statements("total", 20, "total_exec_time", "mean_exec_time")
                .unwrap()
                .contains("total_exec_time DESC")
        );
        assert!(
            Fixed::statements("mean", 20, "total_time", "mean_time")
                .unwrap()
                .contains("mean_time DESC")
        );
        assert!(
            Fixed::statements("calls", 20, "total_time", "mean_time")
                .unwrap()
                .contains("calls DESC")
        );
        assert!(
            Fixed::statements("rows", 20, "total_time", "mean_time")
                .unwrap()
                .contains("rows DESC")
        );
        assert!(Fixed::statements("fastest", 20, "total_time", "mean_time").is_err());
        assert!(Fixed::statements("total", 20, "bad col", "x").is_err());
        let settings =
            Fixed::settings(&["work_mem".into(), "shared_buffers".into()], Some("log_")).unwrap();
        assert!(
            settings.contains("name IN ('work_mem', 'shared_buffers')")
                && settings.contains("name LIKE 'log_%'")
        );
        assert!(Fixed::settings(&[], None).is_err());
        assert!(Fixed::settings(&["bad name".into()], None).is_err());
        assert!(Fixed::backend(42).unwrap().contains("pid = 42"));
        assert!(Fixed::backend(0).is_err());
        assert!(
            Fixed::relation("demo.orders")
                .unwrap()
                .contains("n.nspname = 'demo' AND c.relname = 'orders'")
        );
        assert!(
            Fixed::relation("orders")
                .unwrap()
                .contains("current_schemas(false)")
        );
        assert!(Fixed::relation("a.b.c").is_err());
        for fixed in [
            Fixed::SERVER,
            Fixed::DATABASE_SIZE,
            Fixed::SCHEMAS,
            Fixed::EXTENSIONS,
            Fixed::KEY_SETTINGS,
            Fixed::STATEMENTS_COLUMNS,
            Fixed::REPLICATION,
            Fixed::RECOVERY,
        ] {
            assert!(fixed.starts_with("SELECT"));
        }
    }
}
