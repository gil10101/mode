//! History from Claude Code's own `stats-cache.json` (ADR-0052): the days
//! before Mode was installed, so the tracker and the breakdown show the year
//! and not the weeks since the install. The file is Claude Code's, undocumented
//! and versioned by it, so the reader takes what it recognizes and ignores the
//! rest: `dailyActivity` (sessions and messages per day), `dailyModelTokens`
//! (tokens per model per day) and `modelUsage` (the all-time split of each
//! model's tokens by kind, used to apportion a day's total before pricing it).
//! Imported rows carry `origin = 'claude_stats'` and never replace a day Mode
//! counted itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, named_params};
use serde_json::Value;

use crate::db::Result;
use crate::queries::Clock;
use crate::stats::pricing::{self, Counts};

/// What one import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Days written (inserted or refreshed) to `activity_days`.
    pub days: usize,
    /// (day, model) rows written to `activity_models`.
    pub models: usize,
    /// Days left alone because Mode has its own roll-up for them.
    pub kept: usize,
}

/// `$CLAUDE_CONFIG_DIR/stats-cache.json`, else `~/.claude/stats-cache.json`.
pub fn path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("stats-cache.json"));
    }
    dirs::home_dir().map(|h| h.join(".claude").join("stats-cache.json"))
}

/// Import the file at `path` when it exists and parses; a missing or
/// unreadable file is an empty report, never an error.
pub fn import_file(
    conn: &Connection,
    path: &Path,
    clock: &Clock,
    keep_days: i64,
) -> Result<Report> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Report::default());
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Ok(Report::default());
    };
    import_value(conn, &v, clock, keep_days)
}

/// One day as the cache knows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CacheDay {
    pub sessions: i64,
    pub messages: i64,
    /// Tokens per model (the cache keeps one figure per model and day).
    pub tokens_by_model: BTreeMap<String, i64>,
}

/// The share of each token kind in a model's all-time usage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shares {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// The parts of the file the import uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub days: BTreeMap<String, CacheDay>,
    pub shares: BTreeMap<String, Shares>,
}

fn num(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        _ => 0,
    }
}

fn first_num(o: &serde_json::Map<String, Value>, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|k| o.get(*k))
        .map(|v| num(Some(v)))
        .unwrap_or(0)
}

/// `2026-10-16` from a date or an ISO instant.
fn day_of(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    let d = s.get(..10)?;
    (d.len() == 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-').then(|| d.to_string())
}

/// Entries of an array, or the values of an object keyed by day (the key
/// becomes the entry's `date` when it has none).
fn entries(v: Option<&Value>) -> Vec<serde_json::Map<String, Value>> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|e| e.as_object().cloned()).collect(),
        Some(Value::Object(o)) => o
            .iter()
            .filter_map(|(k, e)| {
                let mut m = e.as_object().cloned()?;
                m.entry("date").or_insert_with(|| Value::String(k.clone()));
                Some(m)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn looks_like_model(k: &str) -> bool {
    k.starts_with("claude") || k.contains("opus") || k.contains("sonnet") || k.contains("haiku")
}

/// Read the parts the import uses.
pub fn parse(v: &Value) -> Parsed {
    let mut out = Parsed::default();
    for e in entries(v.get("dailyActivity")) {
        let Some(day) = day_of(e.get("date").or_else(|| e.get("day"))) else {
            continue;
        };
        let d = out.days.entry(day).or_default();
        d.sessions += first_num(&e, &["sessionCount", "sessions"]);
        d.messages += first_num(&e, &["messageCount", "messages"]);
    }
    for e in entries(v.get("dailyModelTokens")) {
        let Some(day) = day_of(e.get("date").or_else(|| e.get("day"))) else {
            continue;
        };
        // Either `{date, tokensByModel: {model: n}}` (any object whose keys are model ids) or `{date, model, tokens}`.
        let mut found = false;
        for (k, val) in &e {
            if let Value::Object(by_model) = val {
                if by_model.keys().any(|m| looks_like_model(m)) {
                    for (m, n) in by_model {
                        let n = match n {
                            Value::Object(o) => first_num(o, &["tokens", "totalTokens", "total"]),
                            other => num(Some(other)),
                        };
                        *out.days
                            .entry(day.clone())
                            .or_default()
                            .tokens_by_model
                            .entry(m.clone())
                            .or_insert(0) += n;
                    }
                    found = true;
                }
            }
            let _ = k;
        }
        if !found {
            if let Some(m) = e.get("model").and_then(Value::as_str) {
                let n = first_num(&e, &["tokens", "totalTokens", "total"]);
                *out.days
                    .entry(day)
                    .or_default()
                    .tokens_by_model
                    .entry(m.to_string())
                    .or_insert(0) += n;
            }
        }
    }
    if let Some(Value::Object(mu)) = v.get("modelUsage") {
        for (m, u) in mu {
            let Some(o) = u.as_object() else { continue };
            let input = first_num(o, &["inputTokens", "input_tokens"]) as f64;
            let output = first_num(o, &["outputTokens", "output_tokens"]) as f64;
            let cr = first_num(o, &["cacheReadInputTokens", "cache_read_input_tokens"]) as f64;
            let cw = first_num(
                o,
                &["cacheCreationInputTokens", "cache_creation_input_tokens"],
            ) as f64;
            let total = input + output + cr + cw;
            if total > 0.0 {
                out.shares.insert(
                    m.clone(),
                    Shares {
                        input: input / total,
                        output: output / total,
                        cache_read: cr / total,
                        cache_write: cw / total,
                    },
                );
            }
        }
    }
    out
}

/// A day's model total split by the model's all-time shares (an estimate);
/// without shares it is counted as output, the most expensive kind, so the
/// estimate errs high rather than hides spend.
fn apportion(total: i64, shares: Option<&Shares>) -> Counts {
    match shares {
        Some(s) => {
            let input = (total as f64 * s.input).round() as i64;
            let output = (total as f64 * s.output).round() as i64;
            let cache_read = (total as f64 * s.cache_read).round() as i64;
            let cache_write = total - input - output - cache_read;
            Counts {
                input,
                output,
                cache_read,
                cache_write: cache_write.max(0),
                cache_write_1h: 0,
            }
        }
        None => Counts {
            output: total,
            ..Counts::default()
        },
    }
}

/// Write the parsed days that fall in the last `keep_days` and before today.
pub fn import_value(conn: &Connection, v: &Value, clock: &Clock, keep_days: i64) -> Result<Report> {
    let parsed = parse(v);
    let floor: String = conn.query_row(
        "SELECT date(?1, '-' || ?2 || ' days')",
        rusqlite::params![clock.today, keep_days.max(1)],
        |r| r.get(0),
    )?;
    let mut report = Report::default();
    let mut day_st = conn.prepare(
        "INSERT INTO activity_days(day, sessions_started, files_changed, tokens, rolled_at, messages, origin)
         VALUES (:day, :sessions, 0, :tokens, :now, :messages, 'claude_stats')
         ON CONFLICT(day) DO UPDATE SET sessions_started = excluded.sessions_started,
                                        tokens           = excluded.tokens,
                                        messages         = excluded.messages,
                                        rolled_at        = excluded.rolled_at
         WHERE activity_days.origin = 'claude_stats'",
    )?;
    let mut model_st = conn.prepare(
        "INSERT INTO activity_models(day, source, model, messages, input_tokens, output_tokens,
                                     cache_read_tokens, cache_write_tokens, spend_usd, origin, rolled_at)
         VALUES (:day, 'claude_code', :model, 0, :input, :output, :cache_read, :cache_write, :spend, 'claude_stats', :now)
         ON CONFLICT(day, source, model) DO UPDATE SET input_tokens       = excluded.input_tokens,
                                                       output_tokens      = excluded.output_tokens,
                                                       cache_read_tokens  = excluded.cache_read_tokens,
                                                       cache_write_tokens = excluded.cache_write_tokens,
                                                       spend_usd          = excluded.spend_usd,
                                                       rolled_at          = excluded.rolled_at
         WHERE activity_models.origin = 'claude_stats'",
    )?;
    for (day, d) in &parsed.days {
        if day.as_str() >= clock.today.as_str() || day.as_str() < floor.as_str() {
            continue;
        }
        let tokens: i64 = d.tokens_by_model.values().sum();
        let n = day_st.execute(named_params! {
            ":day": day, ":sessions": d.sessions, ":tokens": tokens, ":now": clock.now, ":messages": d.messages,
        })?;
        if n == 0 {
            report.kept += 1;
            continue;
        }
        report.days += 1;
        for (model, total) in &d.tokens_by_model {
            let counts = apportion(*total, parsed.shares.get(model));
            let spend = pricing::spend_of("claude_code", model, &counts);
            report.models += model_st.execute(named_params! {
                ":day": day, ":model": model, ":input": counts.input, ":output": counts.output,
                ":cache_read": counts.cache_read, ":cache_write": counts.cache_write, ":spend": spend, ":now": clock.now,
            })?;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_known_shapes_and_ignores_the_rest() {
        let v = json!({
            "version": 2,
            "totalSessions": 344,
            "dailyActivity": [
                {"date": "2026-09-01", "messageCount": 40, "sessionCount": 3, "toolCallCount": 12},
                {"date": "2026-09-02T10:00:00.000Z", "messages": 5, "sessions": 1}
            ],
            "dailyModelTokens": [
                {"date": "2026-09-01", "tokensByModel": {"claude-opus-4-5-20251101": 1000, "claude-haiku-4-5": 10}},
                {"date": "2026-09-02", "model": "claude-opus-4-5-20251101", "tokens": 50}
            ],
            "modelUsage": {
                "claude-opus-4-5-20251101": {"inputTokens": 10, "outputTokens": 20, "cacheReadInputTokens": 60, "cacheCreationInputTokens": 10}
            }
        });
        let p = parse(&v);
        assert_eq!(p.days["2026-09-01"].sessions, 3);
        assert_eq!(p.days["2026-09-01"].messages, 40);
        assert_eq!(
            p.days["2026-09-01"].tokens_by_model["claude-opus-4-5-20251101"],
            1000
        );
        assert_eq!(p.days["2026-09-01"].tokens_by_model["claude-haiku-4-5"], 10);
        assert_eq!(p.days["2026-09-02"].sessions, 1);
        assert_eq!(
            p.days["2026-09-02"].tokens_by_model["claude-opus-4-5-20251101"],
            50
        );
        let s = p.shares["claude-opus-4-5-20251101"];
        assert!((s.input - 0.1).abs() < 1e-9 && (s.cache_read - 0.6).abs() < 1e-9);
        // An object keyed by day works too.
        let v2 = json!({"dailyActivity": {"2026-09-03": {"messageCount": 7}}});
        assert_eq!(parse(&v2).days["2026-09-03"].messages, 7);
        assert!(parse(&json!({"dailyActivity": "nope"})).days.is_empty());
    }

    #[test]
    fn apportions_by_share_and_errs_high_without_one() {
        let s = Shares {
            input: 0.1,
            output: 0.2,
            cache_read: 0.6,
            cache_write: 0.1,
        };
        let c = apportion(1000, Some(&s));
        assert_eq!(
            (c.input, c.output, c.cache_read, c.cache_write),
            (100, 200, 600, 100)
        );
        let c = apportion(1000, None);
        assert_eq!((c.input, c.output), (0, 1000));
    }
}
