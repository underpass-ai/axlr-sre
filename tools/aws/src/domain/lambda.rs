use serde_json::{Value, json};

const LOG_PREFIX: &str = "/aws/lambda/";

/// Where a function logs: `/aws/lambda/<name>` unless its `LoggingConfig`
/// says otherwise. `diverges` is true only when the configured group is
/// not the one the name gives: the API fills `LoggingConfig` with the
/// default on nearly every function, so presence alone says nothing.
#[must_use]
pub fn log_group_of(function: &str, configured: Option<&str>) -> (String, bool) {
    let default = format!("{LOG_PREFIX}{function}");
    match configured.map(str::trim).filter(|v| !v.is_empty()) {
        Some(name) if name != default => (name.to_string(), true),
        _ => (default, false),
    }
}

/// The row `lambda_functions` shows: no environment variables, no code
/// location, nothing that could carry a secret.
#[must_use]
pub fn function_summary(raw: &Value) -> Value {
    let name = raw["FunctionName"].as_str().unwrap_or("?");
    let (log_group, diverges) = log_group_of(name, raw["LoggingConfig"]["LogGroup"].as_str());
    json!({
        "name": name,
        "runtime": raw["Runtime"].as_str().or(raw["PackageType"].as_str()).unwrap_or("-"),
        "memoryMb": raw["MemorySize"],
        "timeoutSeconds": raw["Timeout"],
        "lastModified": raw["LastModified"],
        "state": raw["State"],
        "architectures": raw["Architectures"],
        "logGroup": log_group,
        "logGroupDiverges": diverges,
    })
}

/// The triggers of a function: the pull ones from its event source
/// mappings (SQS, Kinesis, DynamoDB, MSK), with the state that says whether
/// they are alive, and the push ones (SNS, API Gateway, EventBridge) read
/// from the statements of its resource policy, which is the only place they
/// are recorded.
#[must_use]
pub fn triggers_of(mappings: Option<&Value>, policy: Option<&Value>) -> Vec<Value> {
    let mut triggers: Vec<Value> = mappings
        .and_then(|m| m["EventSourceMappings"].as_array())
        .map(|items| {
            items
                .iter()
                .map(|m| {
                    json!({
                        "type": "event_source_mapping",
                        "sourceArn": m["EventSourceArn"],
                        "state": m["State"],
                        "batchSize": m["BatchSize"],
                        "uuid": m["UUID"],
                        "lastProcessingResult": m["LastProcessingResult"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let document: Option<Value> = policy
        .and_then(|p| p["Policy"].as_str())
        .and_then(|text| serde_json::from_str(text).ok());
    if let Some(statements) = document.as_ref().and_then(|d| d["Statement"].as_array()) {
        for statement in statements {
            let service = match &statement["Principal"] {
                Value::Object(map) => map
                    .get("Service")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Value::String(text) => Some(text.clone()),
                _ => None,
            };
            triggers.push(json!({
                "type": "push",
                "service": service,
                "sourceArn": statement["Condition"]["ArnLike"]["AWS:SourceArn"],
                "statementId": statement["Sid"],
            }));
        }
    }
    triggers
}

/// What `lambda_function` shows: the summary, the aliases with the version
/// each points at, the triggers, the handler and the layers. Environment
/// variables are left out on purpose.
#[must_use]
pub fn function_detail(
    configuration: &Value,
    aliases: Option<&Value>,
    mappings: Option<&Value>,
    policy: Option<&Value>,
) -> Value {
    let mut detail = function_summary(configuration);
    detail["arn"] = configuration["FunctionArn"].clone();
    detail["handler"] = configuration["Handler"].clone();
    detail["version"] = configuration["Version"].clone();
    detail["codeSize"] = configuration["CodeSize"].clone();
    detail["lastUpdateStatus"] = configuration["LastUpdateStatus"].clone();
    detail["layers"] = configuration["Layers"]
        .as_array()
        .map(|layers| {
            layers
                .iter()
                .filter_map(|l| l["Arn"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .map(|l| json!(l))
        .unwrap_or(json!([]));
    detail["aliases"] = aliases
        .and_then(|a| a["Aliases"].as_array())
        .map(|items| {
            items
                .iter()
                .map(|a| {
                    json!({
                        "name": a["Name"],
                        "version": a["FunctionVersion"],
                        "weights": a["RoutingConfig"]["AdditionalVersionWeights"],
                    })
                })
                .collect::<Vec<_>>()
        })
        .map(|a| json!(a))
        .unwrap_or(json!([]));
    detail["triggers"] = json!(triggers_of(mappings, policy));
    detail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_group_diverges_only_when_it_is_not_the_default() {
        assert_eq!(
            log_group_of("f", None),
            ("/aws/lambda/f".to_string(), false)
        );
        assert_eq!(
            log_group_of("f", Some("/aws/lambda/f")),
            ("/aws/lambda/f".to_string(), false)
        );
        assert_eq!(
            log_group_of("f", Some(" ")),
            ("/aws/lambda/f".to_string(), false)
        );
        assert_eq!(
            log_group_of("f", Some("/custom/f")),
            ("/custom/f".to_string(), true)
        );
    }

    #[test]
    fn a_summary_carries_no_environment_and_a_detail_adds_aliases_and_triggers() {
        let raw = json!({
            "FunctionName": "demo-fn", "FunctionArn": "arn:aws:lambda:eu-west-1:123456789012:function:demo-fn",
            "Runtime": "python3.12", "MemorySize": 256, "Timeout": 30, "LastModified": "2026-11-01T00:00:00.000+0000",
            "State": "Active", "Handler": "app.handler", "Version": "$LATEST", "CodeSize": 1234,
            "Environment": {"Variables": {"DB_PASSWORD": "hunter2"}},
            "LoggingConfig": {"LogGroup": "/custom/demo"}, "Layers": [{"Arn": "arn:layer:1"}]
        });
        let summary = function_summary(&raw);
        assert_eq!(summary["name"], "demo-fn");
        assert_eq!(summary["logGroup"], "/custom/demo");
        assert_eq!(summary["logGroupDiverges"], true);
        assert!(!summary.to_string().contains("hunter2"));
        let image = function_summary(&json!({"FunctionName": "i", "PackageType": "Image"}));
        assert_eq!(image["runtime"], "Image");
        assert_eq!(image["logGroupDiverges"], false);

        let aliases = json!({"Aliases": [{"Name": "live", "FunctionVersion": "12", "RoutingConfig": {"AdditionalVersionWeights": {"13": 0.1}}}]});
        let mappings = json!({"EventSourceMappings": [{"EventSourceArn": "arn:aws:sqs:eu-west-1:123456789012:demo-q", "State": "Enabled", "BatchSize": 10, "UUID": "u"}]});
        let policy = json!({"Policy": "{\"Statement\": [{\"Sid\": \"sns\", \"Principal\": {\"Service\": \"sns.amazonaws.com\"}, \"Condition\": {\"ArnLike\": {\"AWS:SourceArn\": \"arn:aws:sns:eu-west-1:123456789012:t\"}}}, {\"Sid\": \"plain\", \"Principal\": \"events.amazonaws.com\"}]}"});
        let detail = function_detail(&raw, Some(&aliases), Some(&mappings), Some(&policy));
        assert_eq!(detail["aliases"][0]["name"], "live");
        assert_eq!(detail["aliases"][0]["version"], "12");
        assert_eq!(detail["layers"][0], "arn:layer:1");
        assert_eq!(detail["handler"], "app.handler");
        assert_eq!(detail["triggers"].as_array().unwrap().len(), 3);
        assert_eq!(detail["triggers"][0]["type"], "event_source_mapping");
        assert_eq!(detail["triggers"][0]["state"], "Enabled");
        assert_eq!(detail["triggers"][1]["service"], "sns.amazonaws.com");
        assert_eq!(
            detail["triggers"][1]["sourceArn"],
            "arn:aws:sns:eu-west-1:123456789012:t"
        );
        assert_eq!(detail["triggers"][2]["service"], "events.amazonaws.com");
        assert!(!detail.to_string().contains("hunter2"));
        let bare = function_detail(
            &json!({"FunctionName": "b"}),
            None,
            None,
            Some(&json!({"Policy": "not json"})),
        );
        assert_eq!(bare["aliases"], json!([]));
        assert_eq!(bare["triggers"], json!([]));
        assert_eq!(bare["layers"], json!([]));
    }
}
