use serde_json::{Value, json};

pub const ROWS_MAX: i64 = 1_000;
pub const CELL_BYTES: usize = 4_096;
const ANSWER_BYTES: usize = 262_144;

/// Rows as the simple protocol gives them: every cell text or null.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// More rows existed than were asked for.
    pub truncated: bool,
    /// Some cell was cut at 4 KiB, or rows were dropped to fit 256 KiB.
    pub cut: bool,
}

impl RowSet {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "columns": self.columns,
            "count": self.rows.len(),
            "truncated": self.truncated,
            "cut": self.cut,
            "rows": self.rows,
        })
    }

    /// The rows as objects, for the fixed reads whose columns are known.
    #[must_use]
    pub fn objects(&self) -> Vec<Value> {
        self.rows
            .iter()
            .map(|row| {
                Value::Object(
                    self.columns
                        .iter()
                        .zip(row)
                        .map(|(c, v)| (c.clone(), v.clone().map_or(Value::Null, Value::String)))
                        .collect(),
                )
            })
            .collect()
    }

    /// The first cell of the first row, for one-value reads.
    #[must_use]
    pub fn scalar(&self) -> Option<&str> {
        self.rows
            .first()
            .and_then(|r| r.first())
            .and_then(|c| c.as_deref())
    }
}

/// Keeps `max_rows` rows (a row past that only sets `truncated`), cuts
/// every cell at 4 KiB and drops rows past 256 KiB in all.
#[must_use]
pub fn bounded_rows(
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
    max_rows: usize,
) -> RowSet {
    let truncated = rows.len() > max_rows;
    let mut cut = false;
    let mut bytes = 0usize;
    let mut kept = Vec::new();
    for row in rows.into_iter().take(max_rows) {
        let row: Vec<Option<String>> = row
            .into_iter()
            .map(|cell| {
                cell.map(|text| {
                    if text.len() > CELL_BYTES {
                        cut = true;
                        let mut end = CELL_BYTES;
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}…", &text[..end])
                    } else {
                        text
                    }
                })
            })
            .collect();
        let size: usize = row
            .iter()
            .map(|c| c.as_ref().map_or(4, String::len) + 3)
            .sum();
        if bytes + size > ANSWER_BYTES {
            cut = true;
            break;
        }
        bytes += size;
        kept.push(row);
    }
    RowSet {
        columns,
        rows: kept,
        truncated,
        cut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_bounded_by_count_cell_and_answer_size() {
        let rows = vec![
            vec![Some("1".to_string()), Some("x".repeat(5000))],
            vec![Some("2".to_string()), None],
            vec![Some("3".to_string()), Some("z".to_string())],
        ];
        let set = bounded_rows(vec!["id".into(), "body".into()], rows, 2);
        assert_eq!(set.rows.len(), 2);
        assert!(set.truncated && set.cut);
        assert!(set.rows[0][1].as_ref().unwrap().ends_with('…'));
        assert_eq!(set.rows[1][1], None);
        assert_eq!(set.to_json()["count"], 2);
        assert_eq!(set.objects()[1]["body"], Value::Null);
        assert_eq!(set.objects()[0]["id"], "1");
        assert_eq!(set.scalar(), Some("1"));
        let heavy: Vec<Vec<Option<String>>> =
            (0..100).map(|_| vec![Some("m".repeat(4000))]).collect();
        let set = bounded_rows(vec!["m".into()], heavy, 1000);
        assert!(set.cut && !set.truncated);
        assert!(set.rows.len() < 100);
        assert_eq!(RowSet::default().scalar(), None);
    }
}
