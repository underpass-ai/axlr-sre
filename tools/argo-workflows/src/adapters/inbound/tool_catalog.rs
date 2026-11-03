use serde_json::Value;
use sre_mcp::ToolDefinition;

const NAMESPACE: &str = "Namespace; the configured ARGO_NAMESPACE when omitted";

/// The catalogue `tools/list` publishes: doctor, discover, three listings,
/// three details and the logs. Nothing writes.
pub(crate) fn tools() -> Vec<Value> {
    let listings = [
        (
            "list_workflows",
            "Lists workflow runs, newest first, summarised: name, phase, message, times",
        ),
        (
            "list_workflow_templates",
            "Lists WorkflowTemplates, summarised",
        ),
        (
            "list_cron_workflows",
            "Lists CronWorkflows, summarised with their schedule and whether they are suspended",
        ),
    ]
    .into_iter()
    .map(|(name, description)| {
        ToolDefinition::new(name, description)
            .text("namespace", NAMESPACE)
            .number("limit", "Items per page, 1 to 100", 25)
            .text("continue", "The cursor the previous page returned")
            .to_json()
    });
    let details = [
        ("get_workflow", "The whole workflow: spec, status, nodes"),
        ("get_workflow_template", "The whole WorkflowTemplate"),
        ("get_cron_workflow", "The whole CronWorkflow"),
    ]
    .into_iter()
    .map(|(name, description)| {
        ToolDefinition::new(name, description)
            .text("namespace", NAMESPACE)
            .text("name", "The exact resource name")
            .required(&["name"])
            .to_json()
    });
    let logs = ToolDefinition::new(
        "get_workflow_logs",
        "The last lines of a workflow's logs, 256 KiB at most",
    )
    .text("namespace", NAMESPACE)
    .text("name", "The exact workflow name")
    .text("pod_name", "Only this pod's logs")
    .number("tail_lines", "Last lines, 1 to 200", 100)
    .required(&["name"])
    .to_json();
    [
        ToolDefinition::doctor().to_json(),
        ToolDefinition::discover()
            .text("namespace", NAMESPACE)
            .to_json(),
    ]
    .into_iter()
    .chain(listings)
    .chain(details)
    .chain(std::iter::once(logs))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_tools_start_with_doctor_and_discover_and_none_writes() {
        let tools = tools();
        assert_eq!(tools.len(), 9);
        assert_eq!(tools[0]["name"], "doctor");
        assert_eq!(tools[1]["name"], "discover");
        for tool in &tools {
            let name = tool["name"].as_str().unwrap();
            assert!(
                !name.contains("submit") && !name.contains("delete") && !name.contains("resubmit")
            );
        }
        assert_eq!(
            tools[8]["inputSchema"]["required"],
            serde_json::json!(["name"])
        );
    }
}
