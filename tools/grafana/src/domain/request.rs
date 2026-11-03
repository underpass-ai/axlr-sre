use jiff::Timestamp;

use super::{Direction, InputError, TimeWindow, validate_uid};

pub const LOG_LIMIT_MAX: i64 = 1000;
pub const SEARCH_LIMIT_MAX: i64 = 100;
const QUERY_MAX: usize = 4096;
const LABEL_MAX: usize = 128;
const SEARCH_MAX: usize = 200;

/// One GET to Grafana: path segments, each sent as a segment and never
/// interpolated, and query pairs. Every constructor validates its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    segments: Vec<String>,
    query: Vec<(String, String)>,
}

fn segments<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.into_iter().map(str::to_string).collect()
}

fn pair(name: &str, value: impl ToString) -> (String, String) {
    (name.to_string(), value.to_string())
}

fn query_text(text: &str, what: &'static str) -> Result<String, InputError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(InputError::Empty(what));
    }
    if text.len() > QUERY_MAX {
        return Err(InputError::TooLong {
            what,
            max: QUERY_MAX,
        });
    }
    Ok(text.to_string())
}

fn logql(text: &str) -> Result<String, InputError> {
    let text = query_text(text, "query")?;
    if !text.starts_with('{') || !text.contains('}') {
        return Err(InputError::NotAStreamSelector);
    }
    Ok(text)
}

fn label(text: &str) -> Result<String, InputError> {
    let text = text.trim();
    let shape = !text.is_empty()
        && text.len() <= LABEL_MAX
        && !text.as_bytes()[0].is_ascii_digit()
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if shape {
        Ok(text.to_string())
    } else {
        Err(InputError::InvalidLabel(text.to_string()))
    }
}

fn bounded(value: i64, what: &'static str, max: i64) -> Result<i64, InputError> {
    if (1..=max).contains(&value) {
        Ok(value)
    } else {
        Err(InputError::OutOfRange { what, min: 1, max })
    }
}

fn loki(uid: &str, tail: &[&str]) -> Result<Vec<String>, InputError> {
    let mut all = segments([
        "api",
        "datasources",
        "proxy",
        "uid",
        &validate_uid(uid)?,
        "loki",
        "api",
        "v1",
    ]);
    all.extend(tail.iter().map(|s| s.to_string()));
    Ok(all)
}

fn prometheus(uid: &str, tail: &str) -> Result<Vec<String>, InputError> {
    Ok(segments([
        "api",
        "datasources",
        "proxy",
        "uid",
        &validate_uid(uid)?,
        "api",
        "v1",
        tail,
    ]))
}

impl Request {
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    #[must_use]
    pub fn query(&self) -> &[(String, String)] {
        &self.query
    }

    /// The path as a person reads it, for errors and fakes.
    #[must_use]
    pub fn path(&self) -> String {
        format!("/{}", self.segments.join("/"))
    }

    #[must_use]
    pub fn health() -> Self {
        Self {
            segments: segments(["api", "health"]),
            query: Vec::new(),
        }
    }

    #[must_use]
    pub fn org() -> Self {
        Self {
            segments: segments(["api", "org"]),
            query: Vec::new(),
        }
    }

    #[must_use]
    pub fn datasources() -> Self {
        Self {
            segments: segments(["api", "datasources"]),
            query: Vec::new(),
        }
    }

    pub fn loki_query_range(
        uid: &str,
        query: &str,
        window: &TimeWindow,
        limit: i64,
        direction: Direction,
    ) -> Result<Self, InputError> {
        Ok(Self {
            segments: loki(uid, &["query_range"])?,
            query: vec![
                pair("query", logql(query)?),
                pair("start", window.start_nanos()),
                pair("end", window.end_nanos()),
                pair("limit", bounded(limit, "limit", LOG_LIMIT_MAX)?),
                pair("direction", direction.as_str()),
            ],
        })
    }

    pub fn loki_labels(uid: &str, window: &TimeWindow) -> Result<Self, InputError> {
        Ok(Self {
            segments: loki(uid, &["labels"])?,
            query: vec![
                pair("start", window.start_nanos()),
                pair("end", window.end_nanos()),
            ],
        })
    }

    pub fn loki_label_values(
        uid: &str,
        name: &str,
        window: &TimeWindow,
    ) -> Result<Self, InputError> {
        Ok(Self {
            segments: loki(uid, &["label", &label(name)?, "values"])?,
            query: vec![
                pair("start", window.start_nanos()),
                pair("end", window.end_nanos()),
            ],
        })
    }

    pub fn prometheus_query(uid: &str, query: &str, time: Timestamp) -> Result<Self, InputError> {
        Ok(Self {
            segments: prometheus(uid, "query")?,
            query: vec![
                pair("query", query_text(query, "query")?),
                pair("time", time),
            ],
        })
    }

    pub fn prometheus_query_range(
        uid: &str,
        query: &str,
        window: &TimeWindow,
    ) -> Result<Self, InputError> {
        Ok(Self {
            segments: prometheus(uid, "query_range")?,
            query: vec![
                pair("query", query_text(query, "query")?),
                pair("start", window.start()),
                pair("end", window.end()),
                pair("step", window.step_seconds()),
            ],
        })
    }

    /// Dashboards whose title contains `text`, or with `tag`, `limit` at most.
    pub fn search_dashboards(
        text: Option<&str>,
        tag: Option<&str>,
        limit: i64,
    ) -> Result<Self, InputError> {
        let mut query = vec![
            pair("type", "dash-db"),
            pair("limit", bounded(limit, "limit", SEARCH_LIMIT_MAX)?),
        ];
        for (name, value) in [("query", text), ("tag", tag)] {
            if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
                if value.len() > SEARCH_MAX || value.chars().any(char::is_control) {
                    return Err(InputError::TooLong {
                        what: "search",
                        max: SEARCH_MAX,
                    });
                }
                query.push(pair(name, value));
            }
        }
        Ok(Self {
            segments: segments(["api", "search"]),
            query,
        })
    }

    pub fn dashboard(uid: &str) -> Result<Self, InputError> {
        Ok(Self {
            segments: segments(["api", "dashboards", "uid", &validate_uid(uid)?]),
            query: Vec::new(),
        })
    }

    /// The alerts of an Alertmanager Grafana knows: its own (`grafana`) or a datasource's.
    pub fn alerts(uid: &str) -> Result<Self, InputError> {
        Ok(Self {
            segments: segments([
                "api",
                "alertmanager",
                &validate_uid(uid)?,
                "api",
                "v2",
                "alerts",
            ]),
            query: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> TimeWindow {
        TimeWindow::new(
            "2026-11-03T09:00:00Z".parse().unwrap(),
            "2026-11-03T10:00:00Z".parse().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn loki_requests_go_through_the_proxy_in_nanoseconds() {
        let request = Request::loki_query_range(
            "loki-a",
            " {app=\"web\"} |= \"error\" ",
            &window(),
            50,
            Direction::Backward,
        )
        .unwrap();
        assert_eq!(
            request.path(),
            "/api/datasources/proxy/uid/loki-a/loki/api/v1/query_range"
        );
        assert_eq!(
            request.query()[0],
            pair("query", "{app=\"web\"} |= \"error\"")
        );
        assert_eq!(request.query()[1], pair("start", "1793696400000000000"));
        assert_eq!(request.query()[3], pair("limit", "50"));
        assert_eq!(request.query()[4], pair("direction", "backward"));
        assert_eq!(
            Request::loki_labels("loki-a", &window()).unwrap().path(),
            "/api/datasources/proxy/uid/loki-a/loki/api/v1/labels"
        );
        let values = Request::loki_label_values("loki-a", "app", &window()).unwrap();
        assert_eq!(values.segments()[8..], ["label", "app", "values"]);
        assert_eq!(values.query().len(), 2);
    }

    #[test]
    fn prometheus_requests_use_rfc3339_and_a_step() {
        let instant =
            Request::prometheus_query("prom", "up", "2026-11-03T10:00:00Z".parse().unwrap())
                .unwrap();
        assert_eq!(
            instant.path(),
            "/api/datasources/proxy/uid/prom/api/v1/query"
        );
        assert_eq!(instant.query()[1], pair("time", "2026-11-03T10:00:00Z"));
        let range = Request::prometheus_query_range("prom", "rate(x[5m])", &window()).unwrap();
        assert_eq!(
            range.path(),
            "/api/datasources/proxy/uid/prom/api/v1/query_range"
        );
        assert_eq!(range.query()[1], pair("start", "2026-11-03T09:00:00Z"));
        assert_eq!(range.query()[3], pair("step", "36"));
    }

    #[test]
    fn dashboards_alerts_and_the_fixed_requests_have_their_paths() {
        let search = Request::search_dashboards(Some(" api "), Some("sre"), 10).unwrap();
        assert_eq!(search.path(), "/api/search");
        assert_eq!(
            search.query(),
            [
                pair("type", "dash-db"),
                pair("limit", "10"),
                pair("query", "api"),
                pair("tag", "sre")
            ]
        );
        assert_eq!(
            Request::search_dashboards(None, Some(" "), 25)
                .unwrap()
                .query()
                .len(),
            2
        );
        assert_eq!(
            Request::dashboard("abc-1").unwrap().path(),
            "/api/dashboards/uid/abc-1"
        );
        assert_eq!(
            Request::alerts("grafana").unwrap().path(),
            "/api/alertmanager/grafana/api/v2/alerts"
        );
        assert_eq!(Request::health().path(), "/api/health");
        assert_eq!(Request::org().path(), "/api/org");
        assert_eq!(Request::datasources().path(), "/api/datasources");
        assert!(Request::health().query().is_empty());
    }

    #[test]
    fn bad_input_is_refused_before_any_request() {
        let w = window();
        assert_eq!(
            Request::loki_query_range("loki", "", &w, 10, Direction::Forward),
            Err(InputError::Empty("query"))
        );
        assert_eq!(
            Request::loki_query_range("loki", "app=web", &w, 10, Direction::Forward),
            Err(InputError::NotAStreamSelector)
        );
        assert_eq!(
            Request::loki_query_range("loki", "{a=\"b\"}", &w, 0, Direction::Forward),
            Err(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: 1000
            })
        );
        assert!(
            Request::loki_query_range("loki", "{a=\"b\"}", &w, 1001, Direction::Forward).is_err()
        );
        assert!(
            Request::loki_query_range("bad uid", "{a=\"b\"}", &w, 1, Direction::Forward).is_err()
        );
        assert!(matches!(
            Request::loki_query_range(
                "loki",
                &format!("{{a=\"{}\"}}", "x".repeat(5000)),
                &w,
                1,
                Direction::Forward
            ),
            Err(InputError::TooLong { what: "query", .. })
        ));
        for bad in ["", "1abc", "a-b", "a b", &"a".repeat(129)] {
            assert!(
                matches!(
                    Request::loki_label_values("loki", bad, &w),
                    Err(InputError::InvalidLabel(_))
                ),
                "{bad}"
            );
        }
        assert_eq!(
            Request::prometheus_query("prom", " ", "2026-11-03T10:00:00Z".parse().unwrap()),
            Err(InputError::Empty("query"))
        );
        assert!(Request::prometheus_query_range("bad uid", "up", &w).is_err());
        assert!(Request::search_dashboards(None, None, 101).is_err());
        assert!(Request::search_dashboards(Some("a\nb"), None, 1).is_err());
        assert!(Request::search_dashboards(Some(&"x".repeat(201)), None, 1).is_err());
        assert!(Request::dashboard("../x").is_err());
        assert!(Request::alerts("a/b").is_err());
    }
}
