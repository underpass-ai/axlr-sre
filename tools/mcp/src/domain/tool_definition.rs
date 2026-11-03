use serde_json::{Map, Value, json};

/// A tool as `tools/list` publishes it: name, description and input schema.
///
/// The schema closes `additionalProperties`, so an argument the catalogue
/// does not name is refused by a host that validates.
pub struct ToolDefinition {
    name: &'static str,
    description: String,
    properties: Map<String, Value>,
    required: Vec<&'static str>,
}

impl ToolDefinition {
    #[must_use]
    pub fn new(name: &'static str, description: impl Into<String>) -> Self {
        Self {
            name,
            description: description.into(),
            properties: Map::new(),
            required: Vec::new(),
        }
    }

    /// The `doctor` every tool exposes: can the server work, and what to do if not.
    #[must_use]
    pub fn doctor() -> Self {
        Self::new(
            "doctor",
            "Says whether this server can work: configuration, access and credentials present, with the action that fixes each failure. Shows no secret and changes nothing.",
        )
    }

    /// The `discover` every tool exposes: a bounded inventory of what it reaches.
    #[must_use]
    pub fn discover() -> Self {
        Self::new(
            "discover",
            "A bounded inventory of the resources this server can read or operate, so a model knows the scope before it calls anything else.",
        )
    }

    #[must_use]
    pub fn text(mut self, name: &str, description: &str) -> Self {
        self.properties.insert(
            name.into(),
            json!({"type": "string", "description": description}),
        );
        self
    }

    #[must_use]
    pub fn choice(mut self, name: &str, description: &str, values: &[&str], default: &str) -> Self {
        self.properties.insert(
            name.into(),
            json!({"type": "string", "description": description, "enum": values, "default": default}),
        );
        self
    }

    #[must_use]
    pub fn flag(mut self, name: &str, description: &str, default: bool) -> Self {
        self.properties.insert(
            name.into(),
            json!({"type": "boolean", "description": description, "default": default}),
        );
        self
    }

    #[must_use]
    pub fn number(mut self, name: &str, description: &str, default: i64) -> Self {
        self.properties.insert(
            name.into(),
            json!({"type": "integer", "description": description, "default": default}),
        );
        self
    }

    #[must_use]
    pub fn list(mut self, name: &str, description: &str, items: Value) -> Self {
        self.properties.insert(
            name.into(),
            json!({"type": "array", "description": description, "items": items}),
        );
        self
    }

    /// A map of string to string, such as labels or dimensions.
    #[must_use]
    pub fn string_map(mut self, name: &str, description: &str) -> Self {
        self.properties.insert(
            name.into(),
            json!({
                "type": "object",
                "description": description,
                "additionalProperties": {"type": "string"},
            }),
        );
        self
    }

    /// An object whose shape the tool explains in its description, such as a cursor.
    #[must_use]
    pub fn object(mut self, name: &str, description: &str) -> Self {
        self.properties.insert(
            name.into(),
            json!({"type": "object", "description": description}),
        );
        self
    }

    #[must_use]
    pub fn required(mut self, names: &[&'static str]) -> Self {
        self.required = names.to_vec();
        self
    }

    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": {
                "type": "object",
                "properties": Value::Object(self.properties.clone()),
                "required": self.required,
                "additionalProperties": false,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_its_input_schema() {
        let tool = ToolDefinition::new("apply", "applies")
            .text("target", "the target")
            .choice("mode", "how", &["preview", "apply"], "preview")
            .flag("dry_run", "without sending", true)
            .list("items", "lines", json!({"type": "object"}))
            .required(&["target"]);
        let json = tool.to_json();
        assert_eq!(tool.name(), "apply");
        assert_eq!(json["name"], "apply");
        assert_eq!(json["inputSchema"]["required"], json!(["target"]));
        assert_eq!(
            json["inputSchema"]["properties"]["dry_run"]["default"],
            json!(true)
        );
        assert_eq!(
            json["inputSchema"]["properties"]["mode"]["enum"],
            json!(["preview", "apply"])
        );
        assert_eq!(json["inputSchema"]["additionalProperties"], json!(false));
    }

    #[test]
    fn numbers_and_string_maps_carry_their_type() {
        let json = ToolDefinition::new("query", "queries")
            .number("limit", "how many", 50)
            .string_map("dimensions", "metric dimensions")
            .to_json();
        let properties = &json["inputSchema"]["properties"];
        assert_eq!(
            properties["limit"],
            json!({"type": "integer", "description": "how many", "default": 50})
        );
        assert_eq!(
            properties["dimensions"]["additionalProperties"],
            json!({"type": "string"})
        );
    }

    #[test]
    fn an_object_is_neither_a_list_nor_a_string_map() {
        let tool = ToolDefinition::new("next", "pages")
            .object("cursor", "opaque cursor")
            .required(&["cursor"])
            .to_json();
        let cursor = &tool["inputSchema"]["properties"]["cursor"];
        assert_eq!(cursor["type"], "object");
        assert!(cursor.get("items").is_none());
        assert!(cursor.get("additionalProperties").is_none());
    }

    #[test]
    fn doctor_and_discover_have_their_fixed_names_and_no_arguments() {
        let doctor = ToolDefinition::doctor().to_json();
        let discover = ToolDefinition::discover().to_json();
        assert_eq!(doctor["name"], "doctor");
        assert_eq!(discover["name"], "discover");
        assert_eq!(doctor["inputSchema"]["properties"], json!({}));
        assert!(
            discover["description"]
                .as_str()
                .unwrap()
                .contains("inventory")
        );
    }
}
