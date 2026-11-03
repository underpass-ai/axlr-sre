use serde_json::Value;
use sre_mcp::ToolDefinition;

const REGION: &str = "The region, such as eu-west-1; the first of AWS_MCP_REGIONS when omitted. discover lists the regions in scope";
const START: &str = "Window start: RFC 3339, or how long ago (15m, 3h, 1d)";
const END: &str = "Window end: RFC 3339, or how long ago; now when omitted";
const PERIOD: &str = "Seconds per point, a multiple of 60; 60 for a fine zoom";

/// The catalogue `tools/list` publishes: doctor, discover and seven reads.
/// Nothing writes.
pub(crate) fn tools() -> Vec<Value> {
    [
        ToolDefinition::doctor(),
        ToolDefinition::discover().text("region", "With no AWS_MCP_REGIONS, the region to look at"),
        ToolDefinition::new("metric_query", "One CloudWatch metric over a window, compacted to the points where the value changes plus the last; `no data` means CloudWatch has not one point, which is not a failure")
            .text("namespace", "Such as AWS/Lambda, AWS/SQS, AWS/RDS, AWS/EC2 or a custom one")
            .text("metric", "Such as Errors, ApproximateNumberOfMessagesVisible or CPUUtilization")
            .string_map("dimensions", "Name to value, such as {\"FunctionName\": \"web\"} or {\"QueueName\": \"orders-dlq\"}")
            .text("stat", "Sum, Average, Minimum, Maximum, SampleCount or a percentile such as p99; Sum when omitted")
            .number("period", PERIOD, 300)
            .text("start", START)
            .text("end", END)
            .text("region", REGION)
            .required(&["namespace", "metric"]),
        ToolDefinition::new("lambda_health", "A function's invocations, errors, throttles, duration (average and p99) and concurrency over a window, each compacted; a warning says when there is not one point")
            .text("function", "The exact function name, without alias")
            .number("period", PERIOD, 300)
            .text("start", START)
            .text("end", END)
            .text("region", REGION)
            .required(&["function"]),
        ToolDefinition::new("alarms", "The CloudWatch alarms of a region: name, state, since when, why (200 characters) and what each watches")
            .choice("state", "Only alarms in this state; every state when omitted", &["ALARM", "OK", "INSUFFICIENT_DATA", ""], "")
            .text("prefix", "Only alarms whose name starts with this")
            .number("limit", "Alarms at most, 1 to 100", 50)
            .text("region", REGION),
        ToolDefinition::new("lambda_functions", "The functions of a region whose name contains a text, sorted: runtime, memory, timeout, state, last modified and the log group each writes to")
            .text("contains", "Text the function name contains, case-insensitive")
            .number("limit", "Functions at most, 1 to 100", 50)
            .text("region", REGION),
        ToolDefinition::new("lambda_function", "One function: its aliases and the version each points at, its triggers (event source mappings with their state, and the services allowed to invoke it), handler, layers and log group. No environment variables")
            .text("function", "The function name or ARN")
            .text("region", REGION)
            .required(&["function"]),
        ToolDefinition::new("logs_filter", "The log events of a log group in a window, by filter pattern, oldest first; 256 KiB and 4 KiB per message at most, with truncated when there were more")
            .text("log_group", "The log group, such as /aws/lambda/web; lambda_function shows a function's")
            .text("function", "A function name: its log group is derived as /aws/lambda/<name> when log_group is omitted")
            .text("filter", "A CloudWatch Logs filter pattern, such as ERROR or \"Task timed out\"")
            .text("start", START)
            .text("end", END)
            .number("limit", "Events at most, 1 to 1000", 200)
            .text("region", REGION),
        ToolDefinition::new("logs_groups", "The log groups of a region by name prefix: name, stored bytes, retention")
            .text("prefix", "Such as /aws/lambda/ or /aws/eks/")
            .number("limit", "Groups at most, 1 to 100", 50)
            .text("region", REGION),
    ]
    .iter()
    .map(ToolDefinition::to_json)
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
            assert!(tool["inputSchema"]["properties"].get("confirm").is_none());
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        assert_eq!(
            tools[2]["inputSchema"]["required"],
            serde_json::json!(["namespace", "metric"])
        );
        assert!(tools[2]["inputSchema"]["properties"]["dimensions"].is_object());
        assert_eq!(
            tools[7]["inputSchema"]["properties"]["limit"]["default"],
            200
        );
    }
}
