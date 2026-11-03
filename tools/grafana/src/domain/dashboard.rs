use serde_json::{Value, json};

const PANELS_MAX: usize = 100;

/// Search results, summarised: uid, title, folder, tags, url.
#[must_use]
pub fn dashboard_summaries(response: &Value) -> Vec<Value> {
    response
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            json!({
                "uid": item["uid"],
                "title": item["title"],
                "folder": item["folderTitle"],
                "tags": item["tags"],
                "url": item["url"],
            })
        })
        .collect()
}

/// One dashboard without its JSON: its panels, the rows' children included,
/// each with its type, datasource and, when asked, its query expressions.
#[must_use]
pub fn dashboard_detail(response: &Value, with_queries: bool) -> Value {
    let dashboard = &response["dashboard"];
    let mut panels = Vec::new();
    for panel in dashboard["panels"].as_array().into_iter().flatten() {
        push_panel(&mut panels, panel, None, with_queries);
        for child in panel["panels"].as_array().into_iter().flatten() {
            push_panel(&mut panels, child, panel["title"].as_str(), with_queries);
        }
    }
    let total = panels.len();
    panels.truncate(PANELS_MAX);
    json!({
        "uid": dashboard["uid"],
        "title": dashboard["title"],
        "folder": response["meta"]["folderTitle"],
        "tags": dashboard["tags"],
        "version": dashboard["version"],
        "url": response["meta"]["url"],
        "variables": dashboard["templating"]["list"].as_array().into_iter().flatten().filter_map(|v| v["name"].as_str()).collect::<Vec<_>>(),
        "panelCount": total,
        "panelsTruncated": total > PANELS_MAX,
        "panels": panels,
    })
}

fn push_panel(panels: &mut Vec<Value>, panel: &Value, row: Option<&str>, with_queries: bool) {
    let datasource = match &panel["datasource"] {
        Value::String(name) => json!(name),
        Value::Object(reference) => json!(
            reference
                .get("uid")
                .or(reference.get("type"))
                .cloned()
                .unwrap_or(Value::Null)
        ),
        _ => Value::Null,
    };
    let mut summary = json!({
        "id": panel["id"],
        "title": panel["title"],
        "type": panel["type"],
        "row": row,
        "datasource": datasource,
    });
    if with_queries {
        summary["queries"] = json!(
            panel["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|target| target["expr"].as_str())
                .collect::<Vec<_>>()
        );
    }
    panels.push(summary);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_results_are_summarised() {
        let items = dashboard_summaries(
            &json!([{"uid": "a", "title": "API", "folderTitle": "SRE", "tags": ["x"], "url": "/d/a/api", "extra": 1}]),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["folder"], "SRE");
        assert!(items[0].get("extra").is_none());
        assert!(dashboard_summaries(&json!({"message": "x"})).is_empty());
    }

    #[test]
    fn a_detail_walks_the_rows_and_names_the_datasources() {
        let response = json!({"meta": {"folderTitle": "SRE", "url": "/d/a"}, "dashboard": {"uid": "a", "title": "API", "tags": [], "version": 3,
        "templating": {"list": [{"name": "namespace"}, {"name": "pod"}]},
        "panels": [
            {"id": 1, "title": "Latency", "type": "timeseries", "datasource": {"uid": "prom", "type": "prometheus"}, "targets": [{"expr": "histogram_quantile(0.99, x)"}, {"refId": "B"}]},
            {"id": 2, "title": "Errors", "type": "row", "panels": [
                {"id": 3, "title": "5xx", "type": "stat", "datasource": "Prometheus", "targets": [{"expr": "sum(rate(http_5xx[5m]))"}]},
                {"id": 4, "title": "Logs", "type": "logs", "datasource": {"type": "loki"}}
            ]}
        ]}});
        let detail = dashboard_detail(&response, true);
        assert_eq!(detail["title"], "API");
        assert_eq!(detail["variables"], json!(["namespace", "pod"]));
        assert_eq!(detail["panelCount"], 4);
        assert_eq!(detail["panels"][0]["datasource"], "prom");
        assert_eq!(
            detail["panels"][0]["queries"],
            json!(["histogram_quantile(0.99, x)"])
        );
        assert_eq!(detail["panels"][2]["row"], "Errors");
        assert_eq!(detail["panels"][2]["datasource"], "Prometheus");
        assert_eq!(detail["panels"][3]["datasource"], "loki");
        assert_eq!(detail["panels"][3]["queries"], json!([]));
        let without = dashboard_detail(&response, false);
        assert!(without["panels"][0].get("queries").is_none());
    }

    #[test]
    fn many_panels_are_bounded() {
        let panels: Vec<Value> = (0..150).map(|i| json!({"id": i, "type": "stat"})).collect();
        let detail = dashboard_detail(&json!({"dashboard": {"panels": panels}}), false);
        assert_eq!(detail["panelCount"], 150);
        assert_eq!(detail["panelsTruncated"], true);
        assert_eq!(detail["panels"].as_array().unwrap().len(), 100);
        assert!(detail["panels"][0]["datasource"].is_null());
    }
}
