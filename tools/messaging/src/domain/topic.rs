use serde_json::{Value, json};

/// What `get-topic-attributes` says about a topic. The access policy and
/// the delivery policy are left out: long, and not what a bot needs.
#[must_use]
pub fn topic_facts(arn: &str, answer: &Value) -> Value {
    let attributes = answer["Attributes"].as_object();
    let text = |key: &str| attributes.and_then(|a| a.get(key)).and_then(Value::as_str);
    let number = |key: &str| text(key).and_then(|v| v.parse::<u64>().ok());
    let flag = |key: &str| text(key).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    json!({
        "arn": arn,
        "name": arn.rsplit(':').next().unwrap_or(arn),
        "displayName": text("DisplayName"),
        "fifo": flag("FifoTopic") || arn.ends_with(".fifo"),
        "contentBasedDeduplication": flag("ContentBasedDeduplication"),
        "subscriptionsConfirmed": number("SubscriptionsConfirmed"),
        "subscriptionsPending": number("SubscriptionsPending"),
        "subscriptionsDeleted": number("SubscriptionsDeleted"),
        "owner": text("Owner"),
    })
}

/// The subscriptions of `list-subscriptions-by-topic`.
#[must_use]
pub fn subscriptions(answer: &Value) -> Vec<Value> {
    answer["Subscriptions"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|s| {
                    json!({
                        "arn": s["SubscriptionArn"],
                        "protocol": s["Protocol"],
                        "endpoint": s["Endpoint"],
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_topic_and_its_subscriptions_are_summarised() {
        let facts = topic_facts(
            "arn:aws:sns:eu-west-1:123456789012:demo-topic.fifo",
            &json!({"Attributes": {"DisplayName": "Demo", "FifoTopic": "true", "SubscriptionsConfirmed": "2", "Owner": "123456789012", "Policy": "{...}"}}),
        );
        assert_eq!(facts["name"], "demo-topic.fifo");
        assert_eq!(facts["fifo"], true);
        assert_eq!(facts["subscriptionsConfirmed"], 2);
        assert!(facts.get("policy").is_none());
        let bare = topic_facts("arn:aws:sns:eu-west-1:123456789012:t", &json!({}));
        assert_eq!(bare["fifo"], false);
        assert_eq!(bare["displayName"], Value::Null);
        let subs = subscriptions(
            &json!({"Subscriptions": [{"SubscriptionArn": "s", "Protocol": "sqs", "Endpoint": "arn:aws:sqs:eu-west-1:123456789012:q"}]}),
        );
        assert_eq!(subs[0]["protocol"], "sqs");
        assert!(subscriptions(&json!({})).is_empty());
    }
}
