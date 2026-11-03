use jiff::Timestamp;
use serde_json::{Value, json};

use super::time_window::to_minute;

const POINTS_MAX: usize = 200;

/// One metric's points, as `get-metric-data` answered them for one query id.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    points: Vec<(Timestamp, f64)>,
}

impl Series {
    #[must_use]
    pub fn new(mut points: Vec<(Timestamp, f64)>) -> Self {
        points.sort_by_key(|(stamp, _)| *stamp);
        Self { points }
    }

    #[must_use]
    pub fn points(&self) -> &[(Timestamp, f64)] {
        &self.points
    }

    #[must_use]
    pub fn last_value(&self) -> Option<f64> {
        self.points.last().map(|(_, value)| *value)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    #[must_use]
    pub fn all_zero(&self) -> bool {
        !self.points.is_empty() && self.points.iter().all(|(_, v)| *v == 0.0)
    }

    /// The summary an answer carries. Three shapes, and confusing them leads
    /// to opposite conclusions: `no data` (CloudWatch has not one point: not
    /// invoked, or the name is not exact; not a failure), `all zero (N points)`
    /// (there were points and every one is zero), or the points where the
    /// value changes plus the last one. `keep_zeros` returns an all-zero
    /// series as points, for a caller who chose the metric and wants the zeros.
    #[must_use]
    pub fn compact(&self, keep_zeros: bool) -> Value {
        if self.points.is_empty() {
            return json!("no data");
        }
        if !keep_zeros && self.all_zero() {
            return json!(format!("all zero ({} points)", self.points.len()));
        }
        let mut changes: Vec<&(Timestamp, f64)> = Vec::new();
        for (index, point) in self.points.iter().enumerate() {
            if index == 0 || point.1 != self.points[index - 1].1 {
                changes.push(point);
            }
        }
        let last = self.points.last().expect("not empty");
        if changes.last().map(|p| p.0) != Some(last.0) {
            changes.push(last);
        }
        let truncated = changes.len() > POINTS_MAX;
        let shown: Vec<Value> = changes
            .iter()
            .take(POINTS_MAX)
            .map(|(stamp, value)| json!([to_minute(*stamp), round(*value)]))
            .collect();
        if truncated {
            json!({"points": shown, "truncated": true, "total": changes.len()})
        } else {
            Value::Array(shown)
        }
    }
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// The series of one query id out of a `get-metric-data` answer.
#[must_use]
pub fn series_of(answer: &Value, id: &str) -> Series {
    let result = answer["MetricDataResults"]
        .as_array()
        .and_then(|results| results.iter().find(|r| r["Id"] == id));
    let Some(result) = result else {
        return Series::new(Vec::new());
    };
    let stamps = result["Timestamps"].as_array().cloned().unwrap_or_default();
    let values = result["Values"].as_array().cloned().unwrap_or_default();
    let points = stamps
        .iter()
        .zip(values.iter())
        .filter_map(|(stamp, value)| {
            let stamp = stamp.as_str()?.parse::<Timestamp>().ok()?;
            Some((stamp, value.as_f64()?))
        })
        .collect();
    Series::new(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[f64]) -> Series {
        Series::new(
            values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    (
                        Timestamp::from_second(1_793_707_200 + i as i64 * 60).unwrap(),
                        *v,
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn empty_is_no_data_and_zeros_are_summarised_unless_kept() {
        assert_eq!(series(&[]).compact(false), json!("no data"));
        assert_eq!(series(&[]).compact(true), json!("no data"));
        assert!(series(&[]).is_empty());
        assert_eq!(
            series(&[0.0, 0.0]).compact(false),
            json!("all zero (2 points)")
        );
        assert_eq!(
            series(&[0.0, 0.0]).compact(true).as_array().unwrap().len(),
            2
        );
        assert!(series(&[0.0]).all_zero());
        assert_eq!(series(&[1.0, 2.0]).last_value(), Some(2.0));
    }

    #[test]
    fn only_changes_and_the_last_point_travel_in_time_order() {
        let compact = series(&[1.0, 1.0, 2.0, 2.0, 2.0]).compact(false);
        let points = compact.as_array().unwrap();
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], json!(["2026-11-03T12:00Z", 1.0]));
        assert_eq!(points[1], json!(["2026-11-03T12:02Z", 2.0]));
        assert_eq!(points[2], json!(["2026-11-03T12:04Z", 2.0]));
        let unordered = Series::new(vec![
            (Timestamp::from_second(1_793_707_260).unwrap(), 5.0),
            (Timestamp::from_second(1_793_707_200).unwrap(), 1.23456),
        ]);
        assert_eq!(unordered.points()[0].1, 1.23456);
        assert_eq!(
            unordered.compact(false)[0],
            json!(["2026-11-03T12:00Z", 1.235])
        );
        let long: Vec<f64> = (0..300).map(|i| i as f64).collect();
        let compact = series(&long).compact(false);
        assert_eq!(compact["truncated"], true);
        assert_eq!(compact["total"], 300);
        assert_eq!(compact["points"].as_array().unwrap().len(), POINTS_MAX);
    }

    #[test]
    fn the_series_of_a_query_id_is_read_newest_first_or_not() {
        let answer = json!({"MetricDataResults": [
            {"Id": "errors", "Timestamps": ["2026-11-03T12:05:00+00:00", "2026-11-03T12:00:00+00:00"], "Values": [2.0, 1.0]},
            {"Id": "broken", "Timestamps": ["nope"], "Values": [1.0]}
        ]});
        let errors = series_of(&answer, "errors");
        assert_eq!(errors.points().len(), 2);
        assert_eq!(errors.points()[0].1, 1.0);
        assert!(series_of(&answer, "broken").is_empty());
        assert!(series_of(&answer, "missing").is_empty());
        assert!(series_of(&json!({}), "x").is_empty());
    }
}
