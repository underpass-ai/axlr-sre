use serde_json::Value;
use sre_mcp::ToolDefinition;

const TIMEOUT: &str = "statement_timeout for this call in seconds, 1 to 300";
const LIMIT: &str = "Rows at most, 1 to 200";
const SCHEMA: &str = "Only this schema";
const CONFIRM: &str = "false (the default) previews and changes nothing; true runs it";

pub(crate) const WRITE_TOOLS: [&str; 2] = ["execute", "terminate"];

/// The catalogue `tools/list` publishes: doctor, discover, nine reads and,
/// with writes on, `execute` and `terminate`, which preview unless
/// `confirm` is true.
pub(crate) fn tools(with_writes: bool) -> Vec<Value> {
    let mut tools = vec![
        ToolDefinition::doctor(),
        ToolDefinition::discover().text("schema", "Only this schema's tables in the listing"),
        ToolDefinition::new("query", "One read (SELECT, WITH, TABLE, VALUES, EXPLAIN or SHOW) in a read-only transaction with a statement timeout, through a cursor: at most max_rows rows leave the server, every cell text, 4 KiB each and 256 KiB in all. Functions that act on the server (pg_terminate_backend, pg_read_file, dblink…) are refused")
            .text("sql", "The statement; one per call, no parameters: inline the values")
            .number("max_rows", "Rows at most, 1 to 1000", 200)
            .number("timeout_seconds", TIMEOUT, 30)
            .required(&["sql"]),
        ToolDefinition::new("explain", "The plan of a SELECT; analyze runs it inside a transaction that is rolled back and adds buffers")
            .text("sql", "The SELECT")
            .flag("analyze", "EXPLAIN (ANALYZE, BUFFERS): runs the query", false)
            .choice("format", "text lines, or json parsed", &["text", "json"], "text")
            .number("timeout_seconds", TIMEOUT, 30)
            .required(&["sql"]),
        ToolDefinition::new("activity", "The sessions of pg_stat_activity other than this one, longest transaction first: state, wait event, seconds in transaction and in query, the query")
            .flag("include_idle", "Also the idle sessions", false)
            .number("limit", LIMIT, 50),
        ToolDefinition::new("locks", "Who waits on whom: every blocked backend with the backends that block it, from pg_blocking_pids")
            .number("limit", LIMIT, 50),
        ToolDefinition::new("table_stats", "pg_stat_user_tables: live and dead rows, sequential and index scans, writes, last vacuum and analyze; ordered by what hurts")
            .text("schema", SCHEMA)
            .choice("order", "dead rows, total size, sequential scans or writes first", &["dead", "size", "seq_scans", "writes"], "dead")
            .number("limit", LIMIT, 30),
        ToolDefinition::new("index_stats", "pg_stat_user_indexes: size, scans, valid, unique, definition; invalid indexes (a failed CONCURRENTLY build) first")
            .text("schema", SCHEMA)
            .flag("unused_only", "Only indexes never scanned, and the invalid ones", false)
            .number("limit", LIMIT, 30),
        ToolDefinition::new("statements", "pg_stat_statements: calls, total and mean time, rows, buffers, the query; needs the extension")
            .choice("order", "total time, mean time, calls or rows first", &["total", "mean", "calls", "rows"], "total")
            .number("limit", LIMIT, 20),
        ToolDefinition::new("settings", "pg_settings by exact names or by prefix: value, unit, source")
            .list("names", "Setting names, such as work_mem", serde_json::json!({"type": "string"}))
            .text("prefix", "Such as autovacuum or log_"),
        ToolDefinition::new("replication", "On a primary, the standbys with their lag; on a standby, the replay delay"),
    ];
    if with_writes {
        tools.extend([
            ToolDefinition::new("execute", "One write: VACUUM, ANALYZE, REINDEX, CREATE INDEX CONCURRENTLY, ALTER at the maintenance level; any statement but a read at the all level. The preview names the kind and the relation with its size and statistics and, for DML, the plan; confirmed, it runs with the statement timeout and reports the command tag")
                .text("sql", "The statement; one per call")
                .number("timeout_seconds", TIMEOUT, 60)
                .flag("confirm", CONFIRM, false)
                .required(&["sql"]),
            ToolDefinition::new("terminate", "End one backend (pg_terminate_backend) or only its query (cancel); needs the all level. The preview shows the backend as activity does")
                .number("pid", "The backend's pid, as activity lists it", 0)
                .flag("cancel", "Only cancel the running query, keep the session", false)
                .flag("confirm", CONFIRM, false)
                .required(&["pid"]),
        ]);
    }
    tools.iter().map(ToolDefinition::to_json).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_reads_always_and_two_writes_only_with_writes_on() {
        let reads = tools(false);
        assert_eq!(reads.len(), 11);
        assert_eq!(reads[0]["name"], "doctor");
        assert_eq!(reads[1]["name"], "discover");
        for tool in &reads {
            assert!(tool["inputSchema"]["properties"].get("confirm").is_none());
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        let all = tools(true);
        assert_eq!(all.len(), 13);
        for (tool, name) in all[11..].iter().zip(WRITE_TOOLS) {
            assert_eq!(tool["name"], name);
            assert_eq!(
                tool["inputSchema"]["properties"]["confirm"]["default"],
                false
            );
        }
        assert_eq!(
            all[2]["inputSchema"]["required"],
            serde_json::json!(["sql"])
        );
        assert_eq!(
            all[12]["inputSchema"]["required"],
            serde_json::json!(["pid"])
        );
    }
}
