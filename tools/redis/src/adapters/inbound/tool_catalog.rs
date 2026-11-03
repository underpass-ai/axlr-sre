use serde_json::Value;
use sre_mcp::ToolDefinition;

const CONFIRM: &str = "false (the default) previews and changes nothing; true runs it";
const PATTERN: &str = "A SCAN MATCH glob such as session:* or demo:lock:?; wildcards alone select every key and are refused without a value filter";

pub(crate) const WRITE_TOOLS: [&str; 4] = ["expire", "delete", "config_set", "client_kill"];

/// The catalogue `tools/list` publishes: doctor, discover, ten reads and,
/// with writes on, four writes that preview unless `confirm` is true.
pub(crate) fn tools(with_writes: bool) -> Vec<Value> {
    let mut tools = vec![
        ToolDefinition::doctor(),
        ToolDefinition::discover(),
        ToolDefinition::new("info", "INFO by section, parsed into key:value")
            .choice("section", "The section", &["default", "server", "clients", "memory", "persistence", "stats", "replication", "cpu", "errorstats", "cluster", "keyspace", "commandstats", "latencystats", "all"], "default"),
        ToolDefinition::new("slowlog", "SLOWLOG GET: the slowest commands with their microseconds and client, and the threshold")
            .number("count", "Entries at most, 1 to 128", 25),
        ToolDefinition::new("clients", "CLIENT LIST parsed: every client with its flags, command, idle time; blocked ones and replica links counted"),
        ToolDefinition::new("latency", "LATENCY LATEST: the events above latency-monitor-threshold with their latest and max milliseconds"),
        ToolDefinition::new("memory", "MEMORY DOCTOR, MEMORY STATS and the memory section of INFO: used, peak, maxmemory and its policy, fragmentation, used percent of max"),
        ToolDefinition::new("config_get", "CONFIG GET by name or glob")
            .text("pattern", "A parameter name or glob, such as maxmemory* or slowlog-*")
            .required(&["pattern"]),
        ToolDefinition::new("cluster", "CLUSTER INFO and CLUSTER NODES; says when cluster mode is off"),
        ToolDefinition::new("keys", "Keys by pattern through SCAN (never KEYS), with their type, TTL, encoding, memory and a bounded preview when with_values or a value filter is given; the filter sees the preview only")
            .text("pattern", PATTERN)
            .text("value_contains", "Keep only keys whose previewed value contains this text")
            .flag("ignore_case", "Match the value filter ignoring case", false)
            .flag("with_values", "Read each key's facts and preview", false)
            .number("limit", "Keys at most, 1 to 1000", 100),
        ToolDefinition::new("key", "One key: type, TTL, encoding, memory usage, a bounded preview, idle time and LFU frequency")
            .text("key", "The key")
            .required(&["key"]),
        ToolDefinition::new("biggest", "The largest keys of a sample: SCAN with a pattern, MEMORY USAGE per key, the top by bytes and the bytes per type")
            .text("pattern", "A SCAN MATCH glob; every key when omitted")
            .number("limit", "Keys to show, 1 to 200", 20)
            .number("sample", "Keys to measure at most, 1 to 20000", 5000),
    ];
    if with_writes {
        tools.extend([
            ToolDefinition::new("expire", "Set a TTL on one existing key. Preview unless confirm=true")
                .text("key", "The key")
                .number("seconds", "Seconds to live, 1 to a year", 3600)
                .flag("confirm", CONFIRM, false)
                .required(&["key", "seconds"]),
            ToolDefinition::new("delete", "UNLINK keys: an explicit list, or a pattern with an optional value filter. The preview lists the keys it selected; confirm with that list to delete exactly those. Never FLUSH. Preview unless confirm=true")
                .list("keys", "The keys to delete, as the preview listed them", serde_json::json!({"type": "string"}))
                .text("pattern", PATTERN)
                .text("value_contains", "Keep only keys whose previewed value contains this text")
                .flag("ignore_case", "Match the value filter ignoring case", false)
                .flag("confirm", CONFIRM, false),
            ToolDefinition::new("config_set", "CONFIG SET one of the memory and latency parameters (maxmemory, maxmemory-policy, slowlog-*, latency-monitor-threshold, timeout, maxclients, lazyfree-lazy-*, notify-keyspace-events); the rest stay with a person. Preview shows the current value. Preview unless confirm=true")
                .text("parameter", "The parameter")
                .text("value", "The value, checked for the parameter's shape")
                .flag("confirm", CONFIRM, false)
                .required(&["parameter", "value"]),
            ToolDefinition::new("client_kill", "CLIENT KILL one client by id; never this connection, never a replication link. Preview unless confirm=true")
                .number("id", "The client id, as clients lists it", 0)
                .flag("confirm", CONFIRM, false)
                .required(&["id"]),
        ]);
    }
    tools.iter().map(ToolDefinition::to_json).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_reads_always_and_four_writes_only_with_writes_on() {
        let reads = tools(false);
        assert_eq!(reads.len(), 12);
        assert_eq!(reads[0]["name"], "doctor");
        assert_eq!(reads[1]["name"], "discover");
        for tool in &reads {
            assert!(tool["inputSchema"]["properties"].get("confirm").is_none());
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
            let name = tool["name"].as_str().unwrap();
            assert!(
                !name.contains("flush") && !name.contains("command"),
                "{name}"
            );
        }
        let all = tools(true);
        assert_eq!(all.len(), 16);
        for (tool, name) in all[12..].iter().zip(WRITE_TOOLS) {
            assert_eq!(tool["name"], name);
            assert_eq!(
                tool["inputSchema"]["properties"]["confirm"]["default"],
                false
            );
        }
        assert_eq!(
            all[13]["inputSchema"]["properties"]["keys"]["type"],
            "array"
        );
    }
}
