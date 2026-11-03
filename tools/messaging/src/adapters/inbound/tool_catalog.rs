use serde_json::Value;
use sre_mcp::ToolDefinition;

const REGION: &str = "The region, such as eu-west-1; the first of AWS_MCP_REGIONS when omitted";
const QUEUE: &str = "The queue URL, or its name in the region";
const CONFIRM: &str = "false (the default) previews and changes nothing; true runs it";
const MESSAGES: &str = "The messages: [{\"body\": text or JSON, \"attributes\": {name: text}, \"message_group_id\"?, \"message_deduplication_id\"?, \"subject\"?}], 1 to 100";
const GROUP: &str = "For a FIFO destination: the group id of every message that names none";

pub(crate) const WRITE_TOOLS: [&str; 5] = ["send", "publish", "redrive", "redrive_cancel", "drain"];

/// The catalogue `tools/list` publishes: doctor, discover, six reads and,
/// with writes on, five writes that preview unless `confirm` is true.
pub(crate) fn tools(with_writes: bool) -> Vec<Value> {
    let mut tools = vec![
        ToolDefinition::doctor(),
        ToolDefinition::discover().text("region", "With no AWS_MCP_REGIONS, the region to look at"),
        ToolDefinition::new("queues", "The queues of a region by name prefix; with with_depth, how many messages the first fifty hold and whether each is a dead-letter queue")
            .text("prefix", "Only queues whose name starts with this")
            .number("limit", "Queues at most, 1 to 100", 50)
            .flag("with_depth", "Read each queue's depth and redrive facts (one call per queue)", false)
            .text("region", REGION),
        ToolDefinition::new("queue", "One queue: depth (visible, in flight, delayed), FIFO, redrive policy, the queues that send their failed messages to it, retention; with history, the visible messages over the last day and the age of the oldest")
            .text("queue", QUEUE)
            .flag("history", "Add CloudWatch's last day", true)
            .text("region", REGION)
            .required(&["queue"]),
        ToolDefinition::new("peek", "A look at a queue's messages without deleting them: ids, bodies (4 KiB each), attempts, groups, and a summary. The messages stay hidden for visibility_seconds; a queue with a redrive policy is refused, since a receive counts as a delivery attempt")
            .text("queue", QUEUE)
            .number("limit", "Messages at most, 1 to 100", 10)
            .number("visibility_seconds", "How long the messages stay hidden from consumers, 0 to 43200", 30)
            .text("region", REGION)
            .required(&["queue"]),
        ToolDefinition::new("redrive_status", "The message move tasks of a queue: running, completed or failed, with how many messages moved")
            .text("queue", QUEUE)
            .text("region", REGION)
            .required(&["queue"]),
        ToolDefinition::new("topics", "The SNS topics of a region")
            .number("limit", "Topics at most, 1 to 100", 50)
            .text("region", REGION),
        ToolDefinition::new("topic", "One topic: display name, FIFO, subscription counts and the subscriptions with their protocol and endpoint")
            .text("topic", "The topic ARN")
            .text("region", REGION)
            .required(&["topic"]),
    ];
    if with_writes {
        tools.extend([
            ToolDefinition::new("send", "Send messages to a queue, in batches of ten. Preview unless confirm=true")
                .text("queue", QUEUE)
                .list("messages", MESSAGES, serde_json::json!({"type": "object"}))
                .text("message_group_id", GROUP)
                .flag("confirm", CONFIRM, false)
                .text("region", REGION)
                .required(&["queue", "messages"]),
            ToolDefinition::new("publish", "Publish messages to a topic, in batches of ten. Preview unless confirm=true")
                .text("topic", "The topic ARN")
                .list("messages", MESSAGES, serde_json::json!({"type": "object"}))
                .text("message_group_id", GROUP)
                .flag("confirm", CONFIRM, false)
                .text("region", REGION)
                .required(&["topic", "messages"]),
            ToolDefinition::new("redrive", "Start the native redrive of a dead-letter queue: every message back to the queue it came from, or all to destination. One task at a time. Preview unless confirm=true")
                .text("queue", "The dead-letter queue URL or name")
                .text("destination", "A queue URL or name to move every message to; omitted, each goes back to its source")
                .number("per_second", "Messages per second, 1 to 500; SQS decides when omitted", 0)
                .flag("confirm", CONFIRM, false)
                .text("region", REGION)
                .required(&["queue"]),
            ToolDefinition::new("redrive_cancel", "Cancel the running message move task of a queue. Preview unless confirm=true")
                .text("queue", QUEUE)
                .flag("confirm", CONFIRM, false)
                .text("region", REGION)
                .required(&["queue"]),
            ToolDefinition::new("drain", "Empty a dead-letter queue keeping every message: each batch is written to a JSON Lines file and synced before it is deleted. Never PurgeQueue. Preview unless confirm=true")
                .text("queue", "The dead-letter queue URL or name")
                .number("limit", "Messages at most, 1 to 1000", 100)
                .number("visibility_seconds", "How long each received batch stays hidden while it is saved and deleted, 0 to 43200", 60)
                .flag("confirm", CONFIRM, false)
                .text("region", REGION)
                .required(&["queue"]),
        ]);
    }
    tools.iter().map(ToolDefinition::to_json).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_reads_always_and_five_writes_only_with_writes_on() {
        let reads = tools(false);
        assert_eq!(reads.len(), 8);
        assert_eq!(reads[0]["name"], "doctor");
        assert_eq!(reads[1]["name"], "discover");
        for tool in &reads {
            assert!(tool["inputSchema"]["properties"].get("confirm").is_none());
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        let all = tools(true);
        assert_eq!(all.len(), 13);
        for (tool, name) in all[8..].iter().zip(WRITE_TOOLS) {
            assert_eq!(tool["name"], name);
            assert_eq!(
                tool["inputSchema"]["properties"]["confirm"]["default"],
                false
            );
        }
        assert_eq!(
            all[8]["inputSchema"]["required"],
            serde_json::json!(["queue", "messages"])
        );
        assert_eq!(
            all[8]["inputSchema"]["properties"]["messages"]["type"],
            "array"
        );
    }
}
