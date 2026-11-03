use jiff::{Span, Timestamp, ToSpan};

use super::InputError;

const MAX_DAYS: u32 = 14;

/// A time as a call gives it: RFC 3339, or how long ago (`15m`, `3h`, `1d`).
pub fn instant(text: &str, now: Timestamp) -> Result<Timestamp, InputError> {
    let text = text.trim();
    let invalid = || InputError::InvalidTime(text.chars().take(60).collect());
    if text.is_empty() || text.len() > 40 {
        return Err(invalid());
    }
    if let Some(unit) = text
        .chars()
        .last()
        .filter(|c| matches!(c, 's' | 'm' | 'h' | 'd'))
    {
        let digits = &text[..text.len() - 1];
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            let amount: i64 = digits.parse().map_err(|_| invalid())?;
            let unit_seconds = match unit {
                's' => 1,
                'm' => 60,
                'h' => 3_600,
                _ => 86_400,
            };
            let seconds = amount.checked_mul(unit_seconds).ok_or_else(invalid)?;
            let span = Span::new().try_seconds(seconds).map_err(|_| invalid())?;
            return now.checked_sub(span).map_err(|_| invalid());
        }
    }
    text.parse::<Timestamp>().map_err(|_| invalid())
}

/// A bounded window: start before end, 14 days at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    start: Timestamp,
    end: Timestamp,
}

impl TimeWindow {
    /// Both ends are cut to the second: the CLI takes seconds, and an
    /// answer with nanoseconds reads badly.
    pub fn new(start: Timestamp, end: Timestamp) -> Result<Self, InputError> {
        let start = Timestamp::from_second(start.as_second()).unwrap_or(start);
        let end = Timestamp::from_second(end.as_second()).unwrap_or(end);
        if start >= end {
            return Err(InputError::WindowOrder);
        }
        if end.as_second() - start.as_second() > i64::from(MAX_DAYS) * 86_400 {
            return Err(InputError::WindowTooLong(MAX_DAYS));
        }
        Ok(Self { start, end })
    }

    /// `end` is now when omitted; `start` is `default_seconds` before end.
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
                .checked_sub(default_seconds.seconds())
                .map_err(|_| InputError::WindowOrder)?,
        };
        Self::new(start, end)
    }

    #[must_use]
    pub fn start(&self) -> Timestamp {
        self.start
    }

    #[must_use]
    pub fn end(&self) -> Timestamp {
        self.end
    }

    /// What `get-metric-data` takes.
    #[must_use]
    pub fn start_iso(&self) -> String {
        self.start.to_string()
    }

    #[must_use]
    pub fn end_iso(&self) -> String {
        self.end.to_string()
    }

    /// What `filter-log-events` takes.
    #[must_use]
    pub fn start_ms(&self) -> i64 {
        self.start.as_millisecond()
    }

    #[must_use]
    pub fn end_ms(&self) -> i64 {
        self.end.as_millisecond()
    }

    #[must_use]
    pub fn seconds(&self) -> i64 {
        self.end.as_second() - self.start.as_second()
    }
}

/// A timestamp to the minute, UTC with its Z, as every answer shows times.
#[must_use]
pub fn to_minute(stamp: Timestamp) -> String {
    stamp.strftime("%Y-%m-%dT%H:%MZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        "2026-11-03T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn an_instant_is_rfc3339_or_how_long_ago() {
        assert_eq!(
            instant("2026-11-03T10:00:00Z", now()).unwrap().to_string(),
            "2026-11-03T10:00:00Z"
        );
        assert_eq!(
            instant("30m", now()).unwrap().to_string(),
            "2026-11-03T11:30:00Z"
        );
        assert_eq!(
            instant("3h", now()).unwrap().to_string(),
            "2026-11-03T09:00:00Z"
        );
        assert_eq!(
            instant("1d", now()).unwrap().to_string(),
            "2026-11-02T12:00:00Z"
        );
        assert_eq!(
            instant("90s", now()).unwrap().to_string(),
            "2026-11-03T11:58:30Z"
        );
        for bad in ["", "tomorrow", "3 h", "h", "2026-11-03", "99999999999999d"] {
            assert!(
                matches!(instant(bad, now()), Err(InputError::InvalidTime(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_window_defaults_bounds_and_converts() {
        let window = TimeWindow::parse(None, None, now(), 3 * 3600).unwrap();
        assert_eq!(window.start_iso(), "2026-11-03T09:00:00Z");
        assert_eq!(window.end_iso(), "2026-11-03T12:00:00Z");
        assert_eq!(window.end_ms() - window.start_ms(), 3 * 3600 * 1000);
        assert_eq!(window.seconds(), 3 * 3600);
        assert_eq!(window.start(), instant("3h", now()).unwrap());
        assert_eq!(window.end(), now());
        let explicit = TimeWindow::parse(Some("2h"), Some("1h"), now(), 60).unwrap();
        assert_eq!(explicit.seconds(), 3600);
        assert_eq!(
            TimeWindow::parse(Some("1h"), Some("2h"), now(), 60).unwrap_err(),
            InputError::WindowOrder
        );
        assert_eq!(
            TimeWindow::parse(Some("15d"), None, now(), 60).unwrap_err(),
            InputError::WindowTooLong(14)
        );
        assert!(TimeWindow::parse(Some("x"), None, now(), 60).is_err());
        assert_eq!(to_minute(now()), "2026-11-03T12:00Z");
        let fractional: Timestamp = "2026-11-03T12:00:00.5Z".parse().unwrap();
        let cut = TimeWindow::parse(Some("1h"), None, fractional, 60).unwrap();
        assert_eq!(cut.end_iso(), "2026-11-03T12:00:00Z");
        assert_eq!(cut.start_iso(), "2026-11-03T11:00:00Z");
    }
}
