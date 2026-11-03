use serde_json::{Value, json};

use super::{
    InputError, TimeWindow, alarm_prefix, dimension, filter_pattern, log_group, metric_name,
    namespace, period, statistic,
};

pub const PAGE_MAX: i64 = 100;
pub const FUNCTIONS_MAX: i64 = 100;
pub const ALARMS_MAX: i64 = 100;
pub const GROUPS_MAX: i64 = 100;
pub const EVENTS_MAX: i64 = 1000;
const CONNECT_TIMEOUT_SECONDS: u32 = 10;
const READ_TIMEOUT_SECONDS: u32 = 60;

/// The six metrics of a function's health: (query id, metric, statistic).
pub const HEALTH_METRICS: [(&str, &str, &str); 6] = [
    ("invocations", "Invocations", "Sum"),
    ("errors", "Errors", "Sum"),
    ("throttles", "Throttles", "Sum"),
    ("duration_avg_ms", "Duration", "Average"),
    ("duration_p99_ms", "Duration", "p99"),
    ("concurrent_max", "ConcurrentExecutions", "Maximum"),
];

/// One metric to ask `get-metric-data` for, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricQuery {
    id: String,
    namespace: String,
    metric: String,
    dimensions: Vec<(String, String)>,
    stat: String,
    period: i64,
}

impl MetricQuery {
    pub fn new(
        id: &str,
        namespace_text: &str,
        metric_text: &str,
        dimensions: &[(String, String)],
        stat: &str,
        period_seconds: i64,
    ) -> Result<Self, InputError> {
        let id = id.trim();
        let valid_id = id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && id.len() <= 255;
        if !valid_id {
            return Err(InputError::Invalid {
                what: "id",
                value: id.chars().take(60).collect(),
                rule: "a lowercase letter then letters, digits and `_`",
            });
        }
        if dimensions.len() > 30 {
            return Err(InputError::OutOfRange {
                what: "dimensions",
                min: 0,
                max: 30,
            });
        }
        Ok(Self {
            id: id.to_string(),
            namespace: namespace(namespace_text)?,
            metric: metric_name(metric_text)?,
            dimensions: dimensions
                .iter()
                .map(|(n, v)| dimension(n, v))
                .collect::<Result<_, _>>()?,
            stat: statistic(stat)?,
            period: period(period_seconds)?,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "Id": self.id,
            "MetricStat": {
                "Metric": {
                    "Namespace": self.namespace,
                    "MetricName": self.metric,
                    "Dimensions": self.dimensions.iter().map(|(n, v)| json!({"Name": n, "Value": v})).collect::<Vec<_>>(),
                },
                "Period": self.period,
                "Stat": self.stat,
            },
            "ReturnData": true,
        })
    }
}

/// One `aws` run, built from validated parts: the service, the operation,
/// the arguments, the region. The adapter adds the profile, the output
/// format, the pager and the timeouts; nothing a caller typed becomes a flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    region: Option<String>,
    args: Vec<String>,
}

impl Invocation {
    fn new(region: &str, args: &[&str]) -> Self {
        Self {
            region: Some(region.to_string()),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// A run built by a sibling crate whose own domain validated every
    /// argument: no token of `args` may start with `-` unless it is one of
    /// that crate's flags. `region` is validated here.
    pub fn raw(region: &str, args: Vec<String>) -> Result<Self, InputError> {
        Ok(Self {
            region: Some(super::region(region)?),
            args,
        })
    }

    /// A paged listing: `--max-items` and, with a token, `--starting-token`.
    #[must_use]
    pub fn paged(self, max_items: i64, token: Option<&str>) -> Self {
        let with_max = self.arg("--max-items", max_items);
        match token {
            Some(token) => with_max.arg("--starting-token", token),
            None => with_max,
        }
    }

    fn arg(mut self, flag: &str, value: impl ToString) -> Self {
        self.args.push(flag.to_string());
        self.args.push(value.to_string());
        self
    }

    /// `aws --version`: no region, no credentials.
    #[must_use]
    pub fn version() -> Self {
        Self {
            region: None,
            args: vec!["--version".into()],
        }
    }

    /// Who the CLI is: account, user id, ARN.
    #[must_use]
    pub fn caller_identity(region: &str) -> Self {
        Self::new(region, &["sts", "get-caller-identity"])
    }

    /// Several metrics in one call, oldest point first.
    pub fn metric_data(
        region: &str,
        queries: &[MetricQuery],
        window: &TimeWindow,
    ) -> Result<Self, InputError> {
        if queries.is_empty() {
            return Err(InputError::Empty("metric"));
        }
        let data = Value::Array(queries.iter().map(MetricQuery::to_json).collect());
        Ok(Self::new(region, &["cloudwatch", "get-metric-data"])
            .arg("--metric-data-queries", data)
            .arg("--start-time", window.start_iso())
            .arg("--end-time", window.end_iso())
            .arg("--scan-by", "TimestampAscending")
            .arg("--no-paginate", "")
            .without_empty())
    }

    fn without_empty(mut self) -> Self {
        self.args.retain(|a| !a.is_empty());
        self
    }

    pub fn alarms(
        region: &str,
        state: Option<&str>,
        prefix: Option<&str>,
        max_items: i64,
        token: Option<&str>,
    ) -> Result<Self, InputError> {
        if !(1..=ALARMS_MAX).contains(&max_items) {
            return Err(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: ALARMS_MAX,
            });
        }
        let mut invocation = Self::new(region, &["cloudwatch", "describe-alarms"]);
        if let Some(state) = state {
            let state = state.trim().to_ascii_uppercase().replace(['-', ' '], "_");
            if !["ALARM", "OK", "INSUFFICIENT_DATA"].contains(&state.as_str()) {
                return Err(InputError::UnknownChoice {
                    what: "state",
                    value: state.chars().take(20).collect(),
                    allowed: "ALARM, OK, INSUFFICIENT_DATA",
                });
            }
            invocation = invocation.arg("--state-value", state);
        }
        if let Some(prefix) = prefix.map(str::trim).filter(|p| !p.is_empty()) {
            invocation = invocation.arg("--alarm-name-prefix", alarm_prefix(prefix)?);
        }
        Ok(invocation.paged(max_items, token))
    }

    pub fn functions(
        region: &str,
        max_items: i64,
        token: Option<&str>,
    ) -> Result<Self, InputError> {
        if !(1..=FUNCTIONS_MAX).contains(&max_items) {
            return Err(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: FUNCTIONS_MAX,
            });
        }
        Ok(Self::new(region, &["lambda", "list-functions"]).paged(max_items, token))
    }

    #[must_use]
    pub fn function_configuration(region: &str, function: &str) -> Self {
        Self::new(region, &["lambda", "get-function-configuration"])
            .arg("--function-name", function)
    }

    #[must_use]
    pub fn aliases(region: &str, function: &str) -> Self {
        Self::new(region, &["lambda", "list-aliases"])
            .arg("--function-name", function)
            .paged(50, None)
    }

    #[must_use]
    pub fn event_source_mappings(region: &str, function: &str) -> Self {
        Self::new(region, &["lambda", "list-event-source-mappings"])
            .arg("--function-name", function)
            .paged(50, None)
    }

    #[must_use]
    pub fn policy(region: &str, function: &str) -> Self {
        Self::new(region, &["lambda", "get-policy"]).arg("--function-name", function)
    }

    pub fn log_groups(
        region: &str,
        prefix: Option<&str>,
        max_items: i64,
        token: Option<&str>,
    ) -> Result<Self, InputError> {
        if !(1..=GROUPS_MAX).contains(&max_items) {
            return Err(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: GROUPS_MAX,
            });
        }
        let mut invocation = Self::new(region, &["logs", "describe-log-groups"]);
        if let Some(prefix) = prefix.map(str::trim).filter(|p| !p.is_empty()) {
            invocation = invocation.arg("--log-group-name-prefix", log_group(prefix)?);
        }
        Ok(invocation.paged(max_items, token))
    }

    pub fn log_events(
        region: &str,
        group: &str,
        window: &TimeWindow,
        filter: Option<&str>,
        max_items: i64,
        token: Option<&str>,
    ) -> Result<Self, InputError> {
        if !(1..=EVENTS_MAX).contains(&max_items) {
            return Err(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: EVENTS_MAX,
            });
        }
        let mut invocation = Self::new(region, &["logs", "filter-log-events"])
            .arg("--log-group-name", log_group(group)?)
            .arg("--start-time", window.start_ms())
            .arg("--end-time", window.end_ms());
        if let Some(filter) = filter.map(str::trim).filter(|f| !f.is_empty()) {
            invocation = invocation.arg("--filter-pattern", filter_pattern(filter)?);
        }
        // The server side is bounded too: without it one call pulls up to
        // 10 000 events before --max-items trims them.
        Ok(invocation
            .arg("--page-size", max_items)
            .paged(max_items, token))
    }

    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// `service operation`, for messages.
    #[must_use]
    pub fn verb(&self) -> String {
        self.args
            .iter()
            .take(2)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The global flags the adapter adds to every run with a region.
    #[must_use]
    pub fn global_args(region: &str, profile: Option<&str>) -> Vec<String> {
        let mut args = vec![
            "--region".to_string(),
            region.to_string(),
            "--output".into(),
            "json".into(),
            "--no-cli-pager".into(),
            "--cli-connect-timeout".into(),
            CONNECT_TIMEOUT_SECONDS.to_string(),
            "--cli-read-timeout".into(),
            READ_TIMEOUT_SECONDS.to_string(),
        ];
        if let Some(profile) = profile {
            args.push("--profile".into());
            args.push(profile.to_string());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> TimeWindow {
        TimeWindow::new(
            "2026-11-03T09:00:00Z".parse().unwrap(),
            "2026-11-03T12:00:00Z".parse().unwrap(),
        )
        .unwrap()
    }

    fn has(invocation: &Invocation, flag: &str, value: &str) -> bool {
        invocation
            .args()
            .windows(2)
            .any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn a_metric_query_is_validated_and_serialised() {
        let query = MetricQuery::new(
            "errors",
            "AWS/Lambda",
            "Errors",
            &[("FunctionName".into(), "demo-fn".into())],
            "sum",
            300,
        )
        .unwrap();
        assert_eq!(query.id(), "errors");
        let json = query.to_json();
        assert_eq!(json["MetricStat"]["Stat"], "Sum");
        assert_eq!(
            json["MetricStat"]["Metric"]["Dimensions"][0]["Value"],
            "demo-fn"
        );
        assert_eq!(json["MetricStat"]["Period"], 300);
        for (id, ns, metric, stat, period) in [
            ("Errors", "AWS/Lambda", "Errors", "Sum", 300),
            ("e-1", "AWS/Lambda", "Errors", "Sum", 300),
            ("e", "", "Errors", "Sum", 300),
            ("e", "AWS/Lambda", "", "Sum", 300),
            ("e", "AWS/Lambda", "Errors", "median", 300),
            ("e", "AWS/Lambda", "Errors", "Sum", 45),
        ] {
            assert!(
                MetricQuery::new(id, ns, metric, &[], stat, period).is_err(),
                "{id} {ns} {metric} {stat} {period}"
            );
        }
        let many: Vec<(String, String)> = (0..31).map(|i| (format!("d{i}"), "v".into())).collect();
        assert!(MetricQuery::new("e", "AWS/Lambda", "Errors", &many, "Sum", 60).is_err());
        assert!(
            MetricQuery::new(
                "e",
                "AWS/Lambda",
                "Errors",
                &[("-x".into(), "v".into())],
                "Sum",
                60
            )
            .is_err()
        );
    }

    #[test]
    fn metric_data_carries_the_queries_the_window_and_ascending_order() {
        let query = MetricQuery::new("e", "AWS/Lambda", "Errors", &[], "Sum", 60).unwrap();
        let invocation = Invocation::metric_data("eu-west-1", &[query], &window()).unwrap();
        assert_eq!(invocation.region(), Some("eu-west-1"));
        assert_eq!(invocation.verb(), "cloudwatch get-metric-data");
        assert!(has(&invocation, "--start-time", "2026-11-03T09:00:00Z"));
        assert!(has(&invocation, "--scan-by", "TimestampAscending"));
        assert!(invocation.args().contains(&"--no-paginate".to_string()));
        assert!(!invocation.args().iter().any(String::is_empty));
        let queries: Value = serde_json::from_str(
            &invocation.args()[invocation
                .args()
                .iter()
                .position(|a| a == "--metric-data-queries")
                .unwrap()
                + 1],
        )
        .unwrap();
        assert_eq!(queries[0]["Id"], "e");
        assert!(Invocation::metric_data("eu-west-1", &[], &window()).is_err());
    }

    #[test]
    fn lists_are_paged_with_max_items_and_a_token() {
        let alarms =
            Invocation::alarms("eu-west-1", Some("alarm"), Some("prod-"), 50, Some("t1")).unwrap();
        assert!(has(&alarms, "--state-value", "ALARM"));
        assert!(has(&alarms, "--alarm-name-prefix", "prod-"));
        assert!(has(&alarms, "--max-items", "50"));
        assert!(has(&alarms, "--starting-token", "t1"));
        assert!(Invocation::alarms("eu-west-1", Some("red"), None, 50, None).is_err());
        assert!(Invocation::alarms("eu-west-1", None, Some("-x"), 50, None).is_err());
        assert!(Invocation::alarms("eu-west-1", None, None, 0, None).is_err());
        let plain = Invocation::alarms("eu-west-1", None, Some(" "), 10, None).unwrap();
        assert!(!plain.args().contains(&"--alarm-name-prefix".to_string()));

        let functions = Invocation::functions("eu-west-1", 20, None).unwrap();
        assert_eq!(functions.verb(), "lambda list-functions");
        assert!(!functions.args().contains(&"--starting-token".to_string()));
        assert!(Invocation::functions("eu-west-1", 101, None).is_err());
        assert!(has(
            &Invocation::function_configuration("eu-west-1", "f"),
            "--function-name",
            "f"
        ));
        assert!(has(
            &Invocation::aliases("eu-west-1", "f"),
            "--max-items",
            "50"
        ));
        assert_eq!(
            Invocation::event_source_mappings("eu-west-1", "f").verb(),
            "lambda list-event-source-mappings"
        );
        assert_eq!(
            Invocation::policy("eu-west-1", "f").verb(),
            "lambda get-policy"
        );

        let groups = Invocation::log_groups("eu-west-1", Some("/aws/lambda/"), 10, None).unwrap();
        assert!(has(&groups, "--log-group-name-prefix", "/aws/lambda/"));
        assert!(Invocation::log_groups("eu-west-1", Some("a b"), 10, None).is_err());
        assert!(Invocation::log_groups("eu-west-1", None, 0, None).is_err());

        let events = Invocation::log_events(
            "eu-west-1",
            "/aws/lambda/f",
            &window(),
            Some("ERROR"),
            200,
            Some("t"),
        )
        .unwrap();
        assert!(has(&events, "--start-time", "1793696400000"));
        assert!(has(&events, "--end-time", "1793707200000"));
        assert!(has(&events, "--filter-pattern", "ERROR"));
        assert!(has(&events, "--starting-token", "t"));
        assert!(has(&events, "--page-size", "200"));
        assert!(
            !Invocation::log_groups("eu-west-1", None, 100, None)
                .unwrap()
                .args()
                .contains(&"--page-size".to_string())
        );
        assert!(
            Invocation::log_events("eu-west-1", "bad group", &window(), None, 10, None).is_err()
        );
        assert!(
            Invocation::log_events("eu-west-1", "/g", &window(), Some("-x"), 10, None).is_err()
        );
        assert!(Invocation::log_events("eu-west-1", "/g", &window(), None, 1001, None).is_err());
    }

    #[test]
    fn a_raw_invocation_checks_its_region_and_pages() {
        let raw = Invocation::raw("eu-west-1", vec!["sqs".into(), "list-queues".into()]).unwrap();
        assert_eq!(raw.verb(), "sqs list-queues");
        assert!(has(
            &raw.clone().paged(5, Some("t")),
            "--starting-token",
            "t"
        ));
        assert!(Invocation::raw("mars", vec![]).is_err());
    }

    #[test]
    fn version_and_identity_and_the_global_flags() {
        assert_eq!(Invocation::version().region(), None);
        assert_eq!(Invocation::version().verb(), "--version");
        assert_eq!(
            Invocation::caller_identity("us-east-1").verb(),
            "sts get-caller-identity"
        );
        let globals = Invocation::global_args("eu-west-1", Some("demo"));
        assert_eq!(&globals[..2], ["--region", "eu-west-1"]);
        assert!(globals.contains(&"--no-cli-pager".to_string()));
        assert_eq!(&globals[globals.len() - 2..], ["--profile", "demo"]);
        assert!(!Invocation::global_args("eu-west-1", None).contains(&"--profile".to_string()));
    }
}
