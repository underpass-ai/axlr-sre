use serde_json::Value;

/// The arguments of a `tools/call`, read by name and by kind.
///
/// A blank string counts as not sent; a number sent as a string still counts
/// as a number, because some hosts send every argument as text.
#[derive(Debug, Clone)]
pub struct Arguments(Value);

impl Arguments {
    #[must_use]
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn raw(&self) -> &Value {
        &self.0
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    #[must_use]
    pub fn text(&self, name: &str) -> Option<String> {
        self.0
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    #[must_use]
    pub fn flag(&self, name: &str, default: bool) -> bool {
        self.0.get(name).and_then(Value::as_bool).unwrap_or(default)
    }

    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        match self.0.get(name)? {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
    }

    #[must_use]
    pub fn integer(&self, name: &str) -> Option<i64> {
        match self.0.get(name)? {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
    }

    /// The strings of a list argument; a single string counts as a list of one.
    #[must_use]
    pub fn texts(&self, name: &str) -> Vec<String> {
        match self.0.get(name) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect(),
            Some(Value::String(_)) => self.text(name).into_iter().collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_blank_text_counts_as_not_sent() {
        let arguments = Arguments::new(json!({"a": "  ", "b": " hello "}));
        assert!(arguments.text("a").is_none());
        assert_eq!(arguments.text("b").unwrap(), "hello");
        assert!(arguments.text("c").is_none());
    }

    #[test]
    fn numbers_count_as_number_and_as_string() {
        let arguments = Arguments::new(json!({"n": 30, "s": "60", "f": 1.5, "bad": "x"}));
        assert_eq!(arguments.integer("n"), Some(30));
        assert_eq!(arguments.integer("s"), Some(60));
        assert_eq!(arguments.number("f"), Some(1.5));
        assert_eq!(arguments.number("s"), Some(60.0));
        assert_eq!(arguments.integer("bad"), None);
        assert_eq!(arguments.integer("missing"), None);
    }

    #[test]
    fn an_absent_flag_keeps_its_default() {
        let arguments = Arguments::new(json!({"yes": true, "no": false}));
        assert!(arguments.flag("yes", false));
        assert!(!arguments.flag("no", true));
        assert!(arguments.flag("absent", true));
        assert!(!arguments.flag("absent", false));
    }

    #[test]
    fn a_number_shaped_like_something_else_is_not_forced() {
        assert_eq!(Arguments::new(json!({"n": true})).integer("n"), None);
        assert_eq!(Arguments::new(json!({"n": [1]})).number("n"), None);
    }

    #[test]
    fn raw_and_get_expose_the_original_value() {
        let arguments = Arguments::new(json!({"items": [1, 2]}));
        assert_eq!(arguments.get("items"), Some(&json!([1, 2])));
        assert_eq!(arguments.raw(), &json!({"items": [1, 2]}));
    }

    #[test]
    fn a_list_of_texts_drops_blanks_and_accepts_one_string() {
        let arguments = Arguments::new(json!({"l": ["a", " ", 3, "b "], "s": "one", "n": 4}));
        assert_eq!(arguments.texts("l"), vec!["a", "b"]);
        assert_eq!(arguments.texts("s"), vec!["one"]);
        assert!(arguments.texts("n").is_empty());
        assert!(arguments.texts("missing").is_empty());
    }
}
