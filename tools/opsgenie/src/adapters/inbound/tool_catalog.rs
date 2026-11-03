use serde_json::Value;
use sre_mcp::ToolDefinition;

const ALERT: &str = "The alert: its tiny id (123 or #123) or its id, as alerts_open lists them";
const CONFIRM: &str = "false (the default) previews the write and sends nothing; true runs it";

/// The names of the tools that write; published only with `OPSGENIE_WRITES=true`.
pub(crate) const WRITE_TOOLS: [&str; 3] = ["alert_acknowledge", "alert_note", "alert_close"];

/// The catalogue `tools/list` publishes: doctor, discover, three reads and,
/// with writes on, three writes that preview unless `confirm` is true.
pub(crate) fn tools(with_writes: bool) -> Vec<Value> {
    let mut tools = vec![
        ToolDefinition::doctor(),
        ToolDefinition::discover(),
        ToolDefinition::new("alerts_open", "The open alerts of the scope, newest first: tiny id, id, message, priority, acknowledged, count, created, and which team or escalation lets it in. Alerts outside the scope are only counted")
            .number("limit", "Alerts at most, 1 to 100", 20)
            .number("offset", "Alerts to skip; next_offset of the previous page", 0),
        ToolDefinition::new("alert", "One alert of the scope with its description, details, tags, source and responders with their names")
            .text("alert", ALERT)
            .required(&["alert"]),
        ToolDefinition::new("alert_notes", "The notes of one alert of the scope, newest first, 2000 characters each at most")
            .text("alert", ALERT)
            .number("limit", "Notes at most, 1 to 100", 20)
            .required(&["alert"]),
    ];
    if with_writes {
        tools.extend([
            ToolDefinition::new("alert_acknowledge", "Acknowledge an open alert of the scope, with an optional note. Preview unless confirm=true")
                .text("alert", ALERT)
                .text("note", "What is being done, 25000 characters at most")
                .flag("confirm", CONFIRM, false)
                .required(&["alert"]),
            ToolDefinition::new("alert_note", "Add a note to an open alert of the scope: what was found or done. Preview unless confirm=true")
                .text("alert", ALERT)
                .text("note", "The note, 1 to 25000 characters")
                .flag("confirm", CONFIRM, false)
                .required(&["alert", "note"]),
            ToolDefinition::new("alert_close", "Close an open alert of the scope with a note that says why, in one call. Preview unless confirm=true")
                .text("alert", ALERT)
                .text("note", "Why it is closed and what was done, 1 to 25000 characters")
                .flag("confirm", CONFIRM, false)
                .required(&["alert", "note"]),
        ]);
    }
    tools.iter().map(ToolDefinition::to_json).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_reads_always_and_three_writes_only_with_writes_on() {
        let reads = tools(false);
        assert_eq!(reads.len(), 5);
        assert_eq!(reads[0]["name"], "doctor");
        assert_eq!(reads[1]["name"], "discover");
        for tool in &reads {
            assert!(tool["inputSchema"]["properties"].get("confirm").is_none());
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        let all = tools(true);
        assert_eq!(all.len(), 8);
        for (tool, name) in all[5..].iter().zip(WRITE_TOOLS) {
            assert_eq!(tool["name"], name);
            assert_eq!(
                tool["inputSchema"]["properties"]["confirm"]["default"],
                false
            );
        }
        assert_eq!(
            all[7]["inputSchema"]["required"],
            serde_json::json!(["alert", "note"])
        );
        assert_eq!(
            all[5]["inputSchema"]["required"],
            serde_json::json!(["alert"])
        );
    }
}
