//! The Stats view's figures (03 §7A): spend estimated at list price
//! ([`pricing`], ADR-0051), the year-long roll-ups behind the metrics and the
//! breakdown ([`rollup`], ADR-0052), the summary the view draws
//! ([`summary`]) and the history imported from Claude Code's own stats cache
//! ([`claude_cache`]).

pub mod claude_cache;
pub mod pricing;
pub mod rollup;
pub mod summary;

use crate::queries;

pub use summary::{StatsDay, StatsSummary, days, days_at, summary, summary_at};

/// Longest window the Stats commands answer (the tracker loads 53 weeks; the
/// roll-ups keep 400 days).
pub const STATS_MAX_DAYS: i64 = 400;

/// `YYYY-MM-DD` ↔ days since 1970-01-01.
pub fn date_of(day: i64) -> String {
    // Inverse of `queries::day_number` (civil from days).
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", if m <= 2 { y + 1 } else { y })
}

/// Why a window was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadWindow(pub String);

impl std::fmt::Display for BadWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BadWindow {}

/// Check a `from ..= to` window: both dates, ordered, at most
/// [`STATS_MAX_DAYS`]. Returns the day numbers.
pub fn window(from: &str, to: &str) -> Result<(i64, i64), BadWindow> {
    let f = queries::day_number(from)
        .filter(|_| from.len() == 10)
        .ok_or_else(|| BadWindow(format!("{from:?} is not a YYYY-MM-DD date")))?;
    let t = queries::day_number(to)
        .filter(|_| to.len() == 10)
        .ok_or_else(|| BadWindow(format!("{to:?} is not a YYYY-MM-DD date")))?;
    if t < f {
        return Err(BadWindow("the window ends before it starts".into()));
    }
    if t - f + 1 > STATS_MAX_DAYS {
        return Err(BadWindow(format!(
            "at most {STATS_MAX_DAYS} days at a time"
        )));
    }
    Ok((f, t))
}

/// The state-column word of a `sessions.state` (`unread` and `done` are
/// "finished").
pub fn state_word(state: &str) -> Option<&'static str> {
    Some(match state {
        "waiting" => "waiting",
        "working" => "working",
        "failed" => "failed",
        "unread" | "done" => "finished",
        "idle" => "idle",
        "untracked" => "untracked",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_through_day_numbers() {
        for d in [
            "1970-01-01",
            "2024-02-29",
            "2026-10-22",
            "2000-03-01",
            "1999-12-31",
        ] {
            assert_eq!(date_of(queries::day_number(d).unwrap()), d);
        }
        assert!(window("2026-10-01", "2026-10-22").is_ok());
        assert!(window("2026-10-22", "2026-10-01").is_err());
        assert!(window("2026-1-1", "2026-10-01").is_err());
        assert!(window("2024-01-01", "2026-10-01").is_err(), "too long");
        assert_eq!(state_word("done"), Some("finished"));
        assert_eq!(state_word("bogus"), None);
    }
}
