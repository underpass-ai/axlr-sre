use rmcp::ErrorData;
use rmcp::model::Tool;
use serde_json::Value;

pub(crate) struct ToolMapper;

impl ToolMapper {
    pub(crate) fn to_tool(definition: Value) -> Result<Tool, ErrorData> {
        serde_json::from_value(definition).map_err(|error| {
            ErrorData::internal_error(format!("invalid tool definition: {error}"), None)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::ToolDefinition;

    #[test]
    fn a_tool_definition_becomes_an_rmcp_tool() {
        let definition = ToolDefinition::new("apply", "applies").text("target", "where");
        let tool = ToolMapper::to_tool(definition.to_json()).unwrap();
        assert_eq!(tool.name, "apply");
        assert_eq!(tool.description.as_deref(), Some("applies"));
        assert_eq!(
            tool.input_schema["properties"]["target"]["type"],
            json!("string")
        );
    }

    #[test]
    fn a_definition_without_input_schema_is_rejected() {
        let error = ToolMapper::to_tool(json!({"name": "broken"})).unwrap_err();
        assert!(error.message.contains("invalid tool definition"));
    }
}
