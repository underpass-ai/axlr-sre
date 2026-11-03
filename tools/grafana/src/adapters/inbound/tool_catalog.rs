use serde_json::Value;
use sre_mcp::ToolDefinition;

const LOKI_DS: &str = "Loki datasource uid; GRAFANA_LOKI_DATASOURCE, or the only Loki datasource, when omitted. discover lists the uids";
const PROM_DS: &str = "Prometheus datasource uid; GRAFANA_PROMETHEUS_DATASOURCE, or the only Prometheus datasource, when omitted";
const START: &str =
    "Window start: RFC 3339, or how long ago (15m, 2h, 1d); one hour before end when omitted";
const END: &str = "Window end: RFC 3339, or how long ago; now when omitted";

/// The catalogue `tools/list` publishes: doctor, discover, three Loki reads,
/// two Prometheus reads, two dashboard reads and the alerts. Nothing writes.
pub(crate) fn tools() -> Vec<Value> {
    [
        ToolDefinition::doctor(),
        ToolDefinition::discover(),
        ToolDefinition::new("loki_query", "Log lines matching a LogQL query in a window, newest first, with next.end to page further back. 256 KiB and 4 KiB per line at most")
            .text("query", "LogQL, starting with a stream selector: {namespace=\"shop\", app=\"web\"} |= \"error\"")
            .text("datasource", LOKI_DS)
            .text("start", START)
            .text("end", END)
            .number("limit", "Lines at most, 1 to 1000", 100)
            .choice("direction", "backward walks from end to start, newest first; forward the other way", &["backward", "forward"], "backward")
            .required(&["query"]),
        ToolDefinition::new("loki_labels", "The label names Loki has in the window, to build a selector")
            .text("datasource", LOKI_DS)
            .text("start", START)
            .text("end", END),
        ToolDefinition::new("loki_label_values", "The values of one label in the window, such as the namespaces or apps that logged")
            .text("label", "The label name, such as namespace or app")
            .text("datasource", LOKI_DS)
            .text("start", START)
            .text("end", END)
            .required(&["label"]),
        ToolDefinition::new("metrics_query", "A PromQL instant query: one value per series at a time, 50 series at most")
            .text("query", "PromQL, such as sum by (pod) (rate(container_cpu_usage_seconds_total{namespace=\"shop\"}[5m]))")
            .text("datasource", PROM_DS)
            .text("time", "The instant: RFC 3339, or how long ago; now when omitted")
            .required(&["query"]),
        ToolDefinition::new("metrics_range", "A PromQL range query over a window, about a hundred points per series, 50 series at most")
            .text("query", "PromQL")
            .text("datasource", PROM_DS)
            .text("start", START)
            .text("end", END)
            .required(&["query"]),
        ToolDefinition::new("dashboards", "Dashboards by title text or tag: uid, title, folder, tags, url")
            .text("query", "Text the title contains")
            .text("tag", "A dashboard tag")
            .number("limit", "Dashboards at most, 1 to 100", 25),
        ToolDefinition::new("dashboard", "One dashboard summarised: its variables and its panels with their type, datasource and query expressions; the rows' panels included")
            .text("uid", "The dashboard uid, as dashboards lists it")
            .flag("with_queries", "Include each panel's query expressions", true)
            .required(&["uid"]),
        ToolDefinition::new("alerts", "The alerts of Grafana's Alertmanager, or of an Alertmanager datasource: name, state, severity, namespace, summary, since when")
            .text("datasource", "An Alertmanager datasource uid; Grafana's own when omitted")
            .flag("firing_only", "Only active alerts", true),
    ]
    .iter()
    .map(ToolDefinition::to_json)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_tools_start_with_doctor_and_discover_and_none_writes() {
        let tools = tools();
        assert_eq!(tools.len(), 10);
        assert_eq!(tools[0]["name"], "doctor");
        assert_eq!(tools[1]["name"], "discover");
        for tool in &tools {
            let name = tool["name"].as_str().unwrap();
            assert!(
                !name.contains("create") && !name.contains("delete") && !name.contains("update"),
                "{name}"
            );
            assert!(
                tool["inputSchema"]["properties"].get("confirm").is_none(),
                "{name}"
            );
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        assert_eq!(
            tools[2]["inputSchema"]["required"],
            serde_json::json!(["query"])
        );
        assert_eq!(
            tools[2]["inputSchema"]["properties"]["direction"]["default"],
            "backward"
        );
    }
}
