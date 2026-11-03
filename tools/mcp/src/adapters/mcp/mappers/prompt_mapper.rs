use rmcp::ErrorData;
use rmcp::model::{GetPromptResult, Prompt};
use serde_json::Value;

pub(crate) struct PromptMapper;

impl PromptMapper {
    pub(crate) fn to_prompt(definition: Value) -> Result<Prompt, ErrorData> {
        serde_json::from_value(definition).map_err(|error| {
            ErrorData::internal_error(format!("invalid prompt definition: {error}"), None)
        })
    }

    pub(crate) fn to_result(rendered: Value) -> Result<GetPromptResult, ErrorData> {
        serde_json::from_value(rendered).map_err(|error| {
            ErrorData::internal_error(format!("invalid rendered prompt: {error}"), None)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_prompt_definition_and_its_rendering_become_rmcp_types() {
        let prompt = PromptMapper::to_prompt(json!({
            "name": "doctor",
            "description": "diagnose",
            "arguments": [{"name": "env", "description": "environment", "required": false}],
        }))
        .unwrap();
        assert_eq!(prompt.name, "doctor");
        let result = PromptMapper::to_result(json!({
            "description": "diagnose",
            "messages": [{"role": "user", "content": {"type": "text", "text": "run doctor"}}],
        }))
        .unwrap();
        assert_eq!(result.messages.len(), 1);
    }

    #[test]
    fn malformed_prompts_are_internal_errors() {
        assert!(PromptMapper::to_prompt(json!({"description": "no name"})).is_err());
        assert!(PromptMapper::to_result(json!({"messages": "not a list"})).is_err());
    }
}
