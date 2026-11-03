use anyhow::{Result, bail};
use serde_json::{Value, json};

const SERIES_MAX: usize = 50;
const POINTS_MAX: usize = 200;

/// A Prometheus answer, bounded: fifty series, two hundred points each.
/// A vector gives one value per series, a matrix a list of `[seconds, value]`,
/// a scalar or string its value.
pub fn series(response: &Value) -> Result<Value> {
    if response["status"] == "error" {
        bail!(
            "Prometheus rejected the query: {}",
            response["error"].as_str().unwrap_or("no reason given")
        );
    }
    let data = &response["data"];
    let kind = data["resultType"].as_str().unwrap_or_default();
    let result = data["result"].as_array().cloned().unwrap_or_default();
    let total = result.len();
    let shown: Vec<Value> = result
        .iter()
        .take(SERIES_MAX)
        .map(|item| match kind {
            "vector" => json!({
                "labels": item["metric"],
                "time": item["value"][0],
                "value": item["value"][1],
            }),
            _ => {
                let points = item["values"].as_array().cloned().unwrap_or_default();
                json!({
                    "labels": item["metric"],
                    "points": points.iter().take(POINTS_MAX).collect::<Vec<_>>(),
                    "pointsTruncated": points.len() > POINTS_MAX,
                })
            }
        })
        .collect();
    Ok(match kind {
        "scalar" | "string" => {
            json!({"type": kind, "time": data["result"][0], "value": data["result"][1]})
        }
        _ => json!({
            "type": kind,
            "count": total,
            "truncated": total > shown.len(),
            "series": shown,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_gives_one_value_per_series() {
        let answer = series(
            &json!({"status": "success", "data": {"resultType": "vector", "result": [
                {"metric": {"job": "a"}, "value": [1793700000.0, "1"]},
                {"metric": {"job": "b"}, "value": [1793700000.0, "0"]}
            ]}}),
        )
        .unwrap();
        assert_eq!(answer["type"], "vector");
        assert_eq!(answer["count"], 2);
        assert_eq!(answer["series"][1]["value"], "0");
        assert_eq!(answer["series"][1]["labels"]["job"], "b");
    }

    #[test]
    fn a_matrix_gives_points_and_is_bounded() {
        let points: Vec<Value> = (0..300).map(|i| json!([i, "1"])).collect();
        let result: Vec<Value> = (0..60)
            .map(|i| json!({"metric": {"i": i.to_string()}, "values": points}))
            .collect();
        let answer = series(&json!({"data": {"resultType": "matrix", "result": result}})).unwrap();
        assert_eq!(answer["type"], "matrix");
        assert_eq!(answer["count"], 60);
        assert_eq!(answer["truncated"], true);
        assert_eq!(answer["series"].as_array().unwrap().len(), 50);
        assert_eq!(answer["series"][0]["points"].as_array().unwrap().len(), 200);
        assert_eq!(answer["series"][0]["pointsTruncated"], true);
    }

    #[test]
    fn scalars_and_errors() {
        let scalar =
            series(&json!({"data": {"resultType": "scalar", "result": [1793700000.0, "2"]}}))
                .unwrap();
        assert_eq!(scalar["type"], "scalar");
        assert_eq!(scalar["value"], "2");
        assert!(
            series(&json!({"status": "error", "error": "bad_data: parse error"}))
                .unwrap_err()
                .to_string()
                .contains("parse error")
        );
        assert!(
            series(&json!({"status": "error"}))
                .unwrap_err()
                .to_string()
                .contains("no reason")
        );
        let empty = series(&json!({})).unwrap();
        assert_eq!(empty["count"], 0);
    }
}
