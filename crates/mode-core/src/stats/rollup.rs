//! S9 (09 §5.5, ADR-0052): roll every completed local day into the year-long
//! tables before retention prunes the live rows. `activity_days` holds the
//! tracker's value and the day metrics, `activity_models` the tokens and the
//! spend per model and source, `activity_hours` the messages per local hour.
//!
//! Only days from `live_days` ago on are re-rolled: a session started on a
//! later day cannot have been pruned yet (retention goes by `ended_at`), so
//! those days are still complete in the live tables, while an older day's
//! roll-up stays as it was written when the day was whole.

use std::collections::BTreeMap;

use rusqlite::{Connection, named_params};

use crate::db::Result;
use crate::queries::Clock;
use crate::stats::pricing::{self, Counts, LONG_PROMPT_TOKENS};

/// Rows written by one roll-up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rolled {
    pub days: usize,
    pub models: usize,
    pub hours: usize,
}

/// Roll the completed days up to yesterday, from `live_days` ago or from the
/// day after the last day rolled, whichever is earlier (a sweep that ran late
/// still rolls what the live tables kept). Idempotent.
pub fn rollup(conn: &Connection, clock: &Clock, live_days: i64) -> Result<Rolled> {
    let from_day: String = conn.query_row(
        "SELECT MIN(date(?1, 'localtime', '-' || ?2 || ' days'),
                    COALESCE((SELECT date(MAX(day), '+1 day') FROM activity_days WHERE origin = 'mode'), '0001-01-01'))",
        rusqlite::params![clock.now, live_days.max(1)],
        |r| r.get(0),
    )?;
    Ok(Rolled {
        days: rollup_days(conn, clock, &from_day)?,
        models: rollup_models(conn, clock, &from_day)?,
        hours: rollup_hours(conn, clock, &from_day)?,
    })
}

fn rollup_days(conn: &Connection, clock: &Clock, from_day: &str) -> Result<usize> {
    Ok(conn.execute(
        "INSERT INTO activity_days(day, sessions_started, files_changed, tokens, rolled_at,
                                   messages, agents_started, lines_added, lines_removed, longest_session_s, origin)
         SELECT day, SUM(sessions_started), SUM(files_changed), SUM(tokens), :now,
                SUM(messages), SUM(agents_started), SUM(lines_added), SUM(lines_removed), MAX(duration_s), 'mode'
           FROM (SELECT date(s.started_at, 'localtime') AS day, 1 AS sessions_started, 0 AS files_changed, 0 AS tokens,
                        0 AS messages, 0 AS agents_started, 0 AS lines_added, 0 AS lines_removed,
                        CAST(MAX(0, (julianday(COALESCE(s.ended_at, s.last_event_at)) - julianday(s.started_at)) * 86400) AS INTEGER) AS duration_s
                   FROM sessions s WHERE s.parent_session_id IS NULL
                 UNION ALL
                 SELECT date(s.started_at, 'localtime'), 0, 0, 0, 0, 1, 0, 0, 0
                   FROM sessions s WHERE s.parent_session_id IS NOT NULL
                 UNION ALL
                 SELECT date(f.first_seen_at, 'localtime'), 0, 1, 0, 0, 0, f.lines_added, f.lines_removed, 0
                   FROM file_changes f
                 UNION ALL
                 SELECT date(u.sampled_at, 'localtime'), 0, 0,
                        u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens, 1, 0, 0, 0, 0
                   FROM usage_samples u WHERE u.source = 'claude_code')
          WHERE day < :today AND day >= :from_day
          GROUP BY day
         ON CONFLICT(day) DO UPDATE SET sessions_started  = excluded.sessions_started,
                                        files_changed     = excluded.files_changed,
                                        tokens            = excluded.tokens,
                                        rolled_at         = excluded.rolled_at,
                                        messages          = excluded.messages,
                                        agents_started    = excluded.agents_started,
                                        lines_added       = excluded.lines_added,
                                        lines_removed     = excluded.lines_removed,
                                        longest_session_s = excluded.longest_session_s,
                                        origin            = 'mode'",
        named_params! { ":now": clock.now, ":today": clock.today, ":from_day": from_day },
    )?)
}

/// One live group of samples: a day, a source, a model and whether the
/// prompts were long (the tier of Haiku 5.5).
pub(crate) struct LiveGroup {
    pub day: String,
    pub source: String,
    pub model: String,
    pub messages: i64,
    pub counts: Counts,
    pub long: bool,
}

/// The live samples between two local days, grouped for pricing.
pub(crate) fn live_groups(
    conn: &Connection,
    from_day: &str,
    to_day_excl: &str,
    session: Option<&str>,
) -> Result<Vec<LiveGroup>> {
    let mut st = conn.prepare(
        "SELECT date(u.sampled_at, 'localtime') AS day, u.source, u.model,
                (u.input_tokens + u.cache_read_tokens + u.cache_write_tokens) > :long AS long,
                COUNT(*), SUM(u.input_tokens), SUM(u.output_tokens), SUM(u.cache_read_tokens),
                SUM(u.cache_write_tokens), SUM(u.cache_write_1h_tokens)
           FROM usage_samples u
          WHERE date(u.sampled_at, 'localtime') >= :from_day AND date(u.sampled_at, 'localtime') < :to_day
            AND (:session IS NULL OR u.session_id = :session)
          GROUP BY day, u.source, u.model, long
          ORDER BY day, u.source, u.model, long",
    )?;
    let out = st
        .query_map(
            named_params! { ":long": LONG_PROMPT_TOKENS, ":from_day": from_day, ":to_day": to_day_excl, ":session": session },
            |r| {
                Ok(LiveGroup {
                    day: r.get(0)?,
                    source: r.get(1)?,
                    model: r.get(2)?,
                    long: r.get::<_, i64>(3)? != 0,
                    messages: r.get(4)?,
                    counts: Counts {
                        input: r.get(5)?,
                        output: r.get(6)?,
                        cache_read: r.get(7)?,
                        cache_write: r.get(8)?,
                        cache_write_1h: r.get(9)?,
                    },
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// The spend of one group: the long-prompt tier when its prompts were long.
pub(crate) fn group_spend(g: &LiveGroup) -> Option<f64> {
    if matches!(g.source.as_str(), "ollama" | "lm_studio") {
        return Some(0.0);
    }
    pricing::rates(&g.model, g.long).map(|r| pricing::spend(&g.counts, &r))
}

/// Merge the live groups of one (day, source, model): counts added, spend
/// `None` as soon as one group is unpriced.
#[derive(Debug, Clone, Default)]
pub(crate) struct Merged {
    pub messages: i64,
    pub counts: Counts,
    pub spend: Option<f64>,
    pub priced_parts: usize,
}

impl Merged {
    pub(crate) fn add(&mut self, messages: i64, c: &Counts, spend: Option<f64>) {
        self.messages += messages;
        self.counts.input += c.input;
        self.counts.output += c.output;
        self.counts.cache_read += c.cache_read;
        self.counts.cache_write += c.cache_write;
        self.counts.cache_write_1h += c.cache_write_1h;
        self.spend = match (self.spend, spend, self.priced_parts) {
            (Some(a), Some(b), _) => Some(a + b),
            (None, Some(b), 0) => Some(b),
            _ => None,
        };
        self.priced_parts += 1;
    }
}

fn rollup_models(conn: &Connection, clock: &Clock, from_day: &str) -> Result<usize> {
    let mut merged: BTreeMap<(String, String, String), Merged> = BTreeMap::new();
    for g in live_groups(conn, from_day, &clock.today, None)? {
        let spend = group_spend(&g);
        merged
            .entry((g.day.clone(), g.source.clone(), g.model.clone()))
            .or_default()
            .add(g.messages, &g.counts, spend);
    }
    let mut st = conn.prepare(
        "INSERT INTO activity_models(day, source, model, messages, input_tokens, output_tokens,
                                     cache_read_tokens, cache_write_tokens, spend_usd, origin, rolled_at)
         VALUES (:day, :source, :model, :messages, :input, :output, :cache_read, :cache_write, :spend, 'mode', :now)
         ON CONFLICT(day, source, model) DO UPDATE SET messages           = excluded.messages,
                                                       input_tokens       = excluded.input_tokens,
                                                       output_tokens      = excluded.output_tokens,
                                                       cache_read_tokens  = excluded.cache_read_tokens,
                                                       cache_write_tokens = excluded.cache_write_tokens,
                                                       spend_usd          = excluded.spend_usd,
                                                       origin             = 'mode',
                                                       rolled_at          = excluded.rolled_at",
    )?;
    let mut n = 0;
    for ((day, source, model), m) in merged {
        n += st.execute(named_params! {
            ":day": day, ":source": source, ":model": model, ":messages": m.messages,
            ":input": m.counts.input, ":output": m.counts.output, ":cache_read": m.counts.cache_read,
            ":cache_write": m.counts.cache_write, ":spend": m.spend, ":now": clock.now,
        })?;
    }
    Ok(n)
}

fn rollup_hours(conn: &Connection, clock: &Clock, from_day: &str) -> Result<usize> {
    Ok(conn.execute(
        "INSERT INTO activity_hours(day, hour, messages, tokens, origin, rolled_at)
         SELECT date(u.sampled_at, 'localtime') AS day,
                CAST(strftime('%H', u.sampled_at, 'localtime') AS INTEGER) AS hour,
                COUNT(*),
                SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens),
                'mode', :now
           FROM usage_samples u
          WHERE u.source = 'claude_code'
            AND date(u.sampled_at, 'localtime') < :today AND date(u.sampled_at, 'localtime') >= :from_day
          GROUP BY day, hour
         ON CONFLICT(day, hour) DO UPDATE SET messages  = excluded.messages,
                                              tokens    = excluded.tokens,
                                              origin    = 'mode',
                                              rolled_at = excluded.rolled_at",
        named_params! { ":now": clock.now, ":today": clock.today, ":from_day": from_day },
    )?)
}

/// Drop roll-up rows older than `keep_days` (the 400-day retention). Returns rows deleted.
pub fn prune(conn: &Connection, clock: &Clock, keep_days: i64) -> Result<usize> {
    let mut n = 0;
    for table in ["activity_days", "activity_models", "activity_hours"] {
        n += conn.execute(
            &format!("DELETE FROM {table} WHERE day < date(:today, '-' || :days || ' days')"),
            named_params! { ":today": clock.today, ":days": keep_days },
        )?;
    }
    Ok(n)
}
