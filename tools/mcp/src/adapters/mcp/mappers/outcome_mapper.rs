use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};

use crate::domain::{OutputFormat, ToolOutcome};

pub(crate) struct OutcomeMapper;

impl OutcomeMapper {
    pub(crate) fn to_result(outcome: ToolOutcome, format: OutputFormat) -> CallToolResult {
        match outcome {
            ToolOutcome::Ok(value) => CallToolResult::success(vec![Self::text(&value, format)]),
            ToolOutcome::Failed { error, hint } => CallToolResult::error(vec![Self::text(
                &json!({"error": error, "hint": hint}),
                format,
            )]),
        }
    }

    fn text(payload: &Value, format: OutputFormat) -> ContentBlock {
        let text = match format {
            OutputFormat::Pretty => {
                serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
            }
            OutputFormat::Compact => payload.to_string(),
        };
        ContentBlock::text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(result: &CallToolResult) -> String {
        serde_json::to_value(result).unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_success_is_readable_json_text_without_error_flag() {
        let result =
            OutcomeMapper::to_result(ToolOutcome::Ok(json!({"ok": true})), OutputFormat::Pretty);
        assert_eq!(result.is_error, Some(false));
        assert!(text_of(&result).contains("\"ok\": true"));
    }

    #[test]
    fn a_failure_carries_error_and_hint_with_error_flag() {
        let outcome = ToolOutcome::failed("broken", Some("retry".into()));
        let result = OutcomeMapper::to_result(outcome, OutputFormat::Pretty);
        assert_eq!(result.is_error, Some(true));
        let text = text_of(&result);
        assert!(text.contains("broken"));
        assert!(text.contains("retry"));
    }

    #[test]
    fn compact_output_has_no_indentation() {
        let result =
            OutcomeMapper::to_result(ToolOutcome::Ok(json!({"ok": true})), OutputFormat::Compact);
        assert_eq!(text_of(&result), "{\"ok\":true}");
    }
}
