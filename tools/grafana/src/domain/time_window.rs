use jiff::{SignedDuration, Timestamp};

use super::InputError;

const MAX_DAYS: u32 = 7;
const POINTS_WANTED: i64 = 100;
const STEP_MIN_SECONDS: i64 = 15;

/// A time, as a call writes it: RFC 3339, or how long ago (`15m`, `2h`, `1d`).
pub fn instant(text: &str, now: Timestamp) -> Result<Timestamp, InputError> {
    let text = text.trim();
    if let Some(seconds) = relative_seconds(text) {
        return now
            .checked_sub(SignedDuration::from_secs(seconds))
            .map_err(|_| InputError::InvalidTime(text.to_string()));
    }
    text.parse::<Timestamp>()
        .map_err(|_| InputError::InvalidTime(text.to_string()))
}

fn relative_seconds(text: &str) -> Option<i64> {
    let (digits, unit) = text.split_at(text.len().checked_sub(1)?);
    let factor = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => return None,
    };
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok().map(|n| n * factor)
}

/// The range of a query: at most seven days, `end` after `start`. Loki takes
/// nanoseconds, Prometheus RFC 3339; both come from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    start: Timestamp,
    end: Timestamp,
}

impl TimeWindow {
    /// `end` defaults to now, `start` to `default_seconds` before `end`.
    pub fn parse(
        start: Option<&str>,
        end: Option<&str>,
        now: Timestamp,
        default_seconds: i64,
    ) -> Result<Self, InputError> {
        let end = match end {
            Some(text) => instant(text, now)?,
            None => now,
        };
        let start = match start {
            Some(text) => instant(text, now)?,
            None => end
                .checked_sub(SignedDuration::from_secs(default_seconds))
                .map_err(|_| InputError::WindowOrder)?,
        };
        Self::new(start, end)
    }

    pub fn new(start: Timestamp, end: Timestamp) -> Result<Self, InputError> {
        if start >= end {
            return Err(InputError::WindowOrder);
        }
        if end.duration_since(start) > SignedDuration::from_hours(i64::from(MAX_DAYS) * 24) {
            return Err(InputError::WindowTooLong(MAX_DAYS));
        }
        Ok(Self { start, end })
    }

    #[must_use]
    pub fn start(&self) -> Timestamp {
        self.start
    }

    #[must_use]
    pub fn end(&self) -> Timestamp {
        self.end
    }

    #[must_use]
    pub fn start_nanos(&self) -> String {
        self.start.as_nanosecond().to_string()
    }

    #[must_use]
    pub fn end_nanos(&self) -> String {
        self.end.as_nanosecond().to_string()
    }

    /// A step that gives about a hundred points, never under fifteen seconds.
    #[must_use]
    pub fn step_seconds(&self) -> i64 {
        (self.end.duration_since(self.start).as_secs() / POINTS_WANTED).max(STEP_MIN_SECONDS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        "2026-11-03T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn an_instant_is_absolute_or_relative() {
        assert_eq!(
            instant("2026-11-03T09:00:00.5Z", now())
                .unwrap()
                .to_string(),
            "2026-11-03T09:00:00.5Z"
        );
        assert_eq!(
            instant("2026-11-03T11:00:00+02:00", now())
                .unwrap()
                .to_string(),
            "2026-11-03T09:00:00Z"
        );
        assert_eq!(
            instant(" 15m ", now()).unwrap().to_string(),
            "2026-11-03T09:45:00Z"
        );
        assert_eq!(
            instant("2h", now()).unwrap().to_string(),
            "2026-11-03T08:00:00Z"
        );
        assert_eq!(
            instant("1d", now()).unwrap().to_string(),
            "2026-11-02T10:00:00Z"
        );
        assert_eq!(
            instant("90s", now()).unwrap().to_string(),
            "2026-11-03T09:58:30Z"
        );
        for bad in [
            "",
            "m",
            "15",
            "15w",
            "1.5h",
            "yesterday",
            "9999999d",
            "2026-11-03",
        ] {
            assert_eq!(
                instant(bad, now()),
                Err(InputError::InvalidTime(bad.trim().into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_window_defaults_to_the_last_hour() {
        let window = TimeWindow::parse(None, None, now(), 3_600).unwrap();
        assert_eq!(window.start().to_string(), "2026-11-03T09:00:00Z");
        assert_eq!(window.end(), now());
        assert_eq!(window.start_nanos(), "1793696400000000000");
        assert_eq!(window.end_nanos(), "1793700000000000000");
        assert_eq!(window.step_seconds(), 36);
        let explicit = TimeWindow::parse(Some("30m"), Some("10m"), now(), 3_600).unwrap();
        assert_eq!(explicit.start().to_string(), "2026-11-03T09:30:00Z");
        assert_eq!(explicit.end().to_string(), "2026-11-03T09:50:00Z");
        assert_eq!(explicit.step_seconds(), 15);
        let only_end = TimeWindow::parse(None, Some("2026-11-01T00:00:00Z"), now(), 600).unwrap();
        assert_eq!(only_end.start().to_string(), "2026-10-31T23:50:00Z");
    }

    #[test]
    fn a_window_is_ordered_and_bounded() {
        assert_eq!(
            TimeWindow::parse(Some("10m"), Some("30m"), now(), 60),
            Err(InputError::WindowOrder)
        );
        assert_eq!(
            TimeWindow::parse(Some("1h"), Some("1h"), now(), 60),
            Err(InputError::WindowOrder)
        );
        assert_eq!(
            TimeWindow::parse(Some("8d"), None, now(), 60),
            Err(InputError::WindowTooLong(7))
        );
        assert!(TimeWindow::parse(Some("7d"), None, now(), 60).is_ok());
        assert!(matches!(
            TimeWindow::parse(Some("bad"), None, now(), 60),
            Err(InputError::InvalidTime(_))
        ));
        assert!(matches!(
            TimeWindow::parse(None, Some("bad"), now(), 60),
            Err(InputError::InvalidTime(_))
        ));
    }
}
