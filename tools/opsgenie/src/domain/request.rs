use serde_json::{Value, json};

use super::{AlertRef, InputError, Note};

pub const PAGE_SIZE: u32 = 100;
pub const MAX_PAGES: u32 = 20;
pub const LIST_LIMIT_MAX: i64 = 100;
pub const NOTES_LIMIT_MAX: i64 = 100;
const OPEN_QUERY: &str = "status: open";
const SOURCE: &str = "opsgenie-mcp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// One call to Opsgenie's API, built from validated parts: the segments are
/// pushed as path segments, the query as pairs, the body as JSON. Nothing a
/// caller typed is interpolated into a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    method: Method,
    segments: Vec<String>,
    query: Vec<(String, String)>,
    body: Option<Value>,
}

impl Request {
    fn get(segments: &[&str]) -> Self {
        Self {
            method: Method::Get,
            segments: segments.iter().map(|s| s.to_string()).collect(),
            query: Vec::new(),
            body: None,
        }
    }

    fn with(mut self, name: &str, value: impl ToString) -> Self {
        self.query.push((name.to_string(), value.to_string()));
        self
    }

    /// One page of the open alerts, oldest first so new ones append at the
    /// end while the pages are walked.
    #[must_use]
    pub fn open_alerts_page(page: u32) -> Self {
        Self::get(&["v2", "alerts"])
            .with("query", OPEN_QUERY)
            .with("limit", PAGE_SIZE)
            .with("offset", page * PAGE_SIZE)
            .with("sort", "createdAt")
            .with("order", "asc")
    }

    /// How many alerts are open: the cheapest call that needs the key.
    #[must_use]
    pub fn open_count() -> Self {
        Self::get(&["v2", "alerts", "count"]).with("query", OPEN_QUERY)
    }

    #[must_use]
    pub fn alert(reference: &AlertRef) -> Self {
        Self::get(&["v2", "alerts", reference.identifier()])
            .with("identifierType", reference.identifier_type())
    }

    pub fn alert_notes(alert_id: &str, limit: i64) -> Result<Self, InputError> {
        if !(1..=NOTES_LIMIT_MAX).contains(&limit) {
            return Err(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: NOTES_LIMIT_MAX,
            });
        }
        Ok(Self::get(&["v2", "alerts", alert_id, "notes"])
            .with("identifierType", "id")
            .with("limit", limit)
            .with("order", "desc"))
    }

    #[must_use]
    pub fn teams() -> Self {
        Self::get(&["v2", "teams"])
    }

    #[must_use]
    pub fn escalations() -> Self {
        Self::get(&["v2", "escalations"])
    }

    #[must_use]
    pub fn request_status(request_id: &str) -> Self {
        Self::get(&["v2", "alerts", "requests", request_id])
    }

    fn post(alert_id: &str, action: &str, note: Option<&Note>, user: Option<&str>) -> Self {
        let mut body = json!({"source": SOURCE});
        if let Some(note) = note {
            body["note"] = json!(note.as_str());
        }
        if let Some(user) = user {
            body["user"] = json!(user);
        }
        Self {
            method: Method::Post,
            segments: ["v2", "alerts", alert_id, action]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            query: vec![("identifierType".into(), "id".into())],
            body: Some(body),
        }
    }

    #[must_use]
    pub fn acknowledge(alert_id: &str, note: Option<&Note>, user: Option<&str>) -> Self {
        Self::post(alert_id, "acknowledge", note, user)
    }

    #[must_use]
    pub fn add_note(alert_id: &str, note: &Note, user: Option<&str>) -> Self {
        Self::post(alert_id, "notes", Some(note), user)
    }

    /// One call closes and carries the note: no half-done state.
    #[must_use]
    pub fn close(alert_id: &str, note: &Note, user: Option<&str>) -> Self {
        Self::post(alert_id, "close", Some(note), user)
    }

    #[must_use]
    pub fn method(&self) -> Method {
        self.method
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().map(String::as_str)
    }

    #[must_use]
    pub fn query(&self) -> &[(String, String)] {
        &self.query
    }

    #[must_use]
    pub fn body(&self) -> Option<&Value> {
        self.body.as_ref()
    }

    /// The path for messages and fakes, without the query.
    #[must_use]
    pub fn path(&self) -> String {
        format!("/{}", self.segments.join("/"))
    }

    #[must_use]
    pub fn is_write(&self) -> bool {
        self.method == Method::Post
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_carry_their_query_and_writes_their_body() {
        let page = Request::open_alerts_page(2);
        assert_eq!(page.path(), "/v2/alerts");
        assert_eq!(page.method(), Method::Get);
        assert!(!page.is_write());
        assert!(page.query().contains(&("offset".into(), "200".into())));
        assert!(page.query().contains(&("order".into(), "asc".into())));
        assert_eq!(Request::open_count().path(), "/v2/alerts/count");
        let alert = Request::alert(&AlertRef::Tiny("12".into()));
        assert_eq!(alert.path(), "/v2/alerts/12");
        assert_eq!(alert.query()[0], ("identifierType".into(), "tiny".into()));
        let notes = Request::alert_notes("id-1", 20).unwrap();
        assert_eq!(notes.path(), "/v2/alerts/id-1/notes");
        assert!(Request::alert_notes("id-1", 0).is_err());
        assert!(Request::alert_notes("id-1", 101).is_err());
        assert_eq!(Request::teams().path(), "/v2/teams");
        assert_eq!(Request::escalations().path(), "/v2/escalations");
        assert_eq!(
            Request::request_status("r-1").path(),
            "/v2/alerts/requests/r-1"
        );
        assert_eq!(page.segments().count(), 2);
        assert_eq!(page.body(), None);

        let note = Note::parse(Some("fixed")).unwrap();
        let close = Request::close("id-1", &note, Some("bot"));
        assert!(close.is_write());
        assert_eq!(close.path(), "/v2/alerts/id-1/close");
        assert_eq!(
            close.body().unwrap(),
            &json!({"source": "opsgenie-mcp", "note": "fixed", "user": "bot"})
        );
        let ack = Request::acknowledge("id-1", None, None);
        assert_eq!(ack.path(), "/v2/alerts/id-1/acknowledge");
        assert_eq!(ack.body().unwrap(), &json!({"source": "opsgenie-mcp"}));
        assert_eq!(
            Request::add_note("id-1", &note, None).path(),
            "/v2/alerts/id-1/notes"
        );
    }
}
