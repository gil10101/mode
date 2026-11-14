//! The Stats view's figures for a window of local days (03 §7A): the spend
//! tiles, the metrics and the breakdown by model, agent and day. Recent days
//! come from the live tables, older ones from the roll-ups (09 §13 Q11–Q13),
//! split at the live cutoff (`retention.sessions_days` ago) so nothing counts
//! twice.

use std::collections::BTreeMap;

use rusqlite::{Connection, named_params};
use serde::Serialize;

use crate::db::{DbError, Result};
use crate::queries::{self, Clock};
use crate::settings;
use crate::stats::pricing::{self, Counts, LONG_PROMPT_TOKENS, PRICES_AS_OF};
use crate::stats::rollup::{self, Merged};
use crate::stats::{BadWindow, date_of, state_word, window};

/// A bad window or a database error.
#[derive(Debug)]
pub enum StatsError {
    Window(BadWindow),
    Db(DbError),
}

impl From<BadWindow> for StatsError {
    fn from(e: BadWindow) -> Self {
        StatsError::Window(e)
    }
}

impl From<DbError> for StatsError {
    fn from(e: DbError) -> Self {
        StatsError::Db(e)
    }
}

impl std::fmt::Display for StatsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatsError::Window(e) => e.fmt(f),
            StatsError::Db(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for StatsError {}

/// `StatsDay`: one local day of Q11 (the tracker).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatsDay {
    pub date: String,
    pub sessions: i64,
    pub files: i64,
    /// Claude Code tokens of the day (tooltip only).
    pub tokens: i64,
}

/// The local day `days` days before now (negative) or after.
fn local_day_shift(conn: &Connection, now: &str, days: i64) -> Result<String> {
    Ok(conn.query_row(
        "SELECT date(?1, 'localtime', ?2 || ' days')",
        rusqlite::params![now, days],
        |r| r.get(0),
    )?)
}

/// Q11 for `from ..= to`, every day present (zeros where nothing happened).
pub fn days(
    conn: &Connection,
    from: &str,
    to: &str,
) -> std::result::Result<Vec<StatsDay>, StatsError> {
    days_at(conn, from, to, &Clock::current(conn)?)
}

/// [`days`] at a given clock (tests).
pub fn days_at(
    conn: &Connection,
    from: &str,
    to: &str,
    clock: &Clock,
) -> std::result::Result<Vec<StatsDay>, StatsError> {
    let (f, t) = window(from, to)?;
    let live_days = settings::retention_in(conn)?.sessions_days.max(1);
    let cutoff_day = local_day_shift(conn, &clock.now, -live_days)?;
    let rows: BTreeMap<String, queries::ActivityRow> =
        queries::activity_days(conn, from, &cutoff_day)?
            .into_iter()
            .filter(|r| r.day.as_str() <= to)
            .map(|r| (r.day.clone(), r))
            .collect();
    Ok((f..=t)
        .map(|n| {
            let date = date_of(n);
            match rows.get(&date) {
                Some(r) => StatsDay {
                    date,
                    sessions: r.sessions_started,
                    files: r.files_changed,
                    tokens: r.tokens,
                },
                None => StatsDay {
                    date,
                    sessions: 0,
                    files: 0,
                    tokens: 0,
                },
            }
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenSplit {
    pub model: String,
    pub source: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SummaryDay {
    pub date: String,
    pub sessions: i64,
    /// Claude Code assistant messages.
    pub messages: i64,
    /// Every source.
    pub tokens: i64,
    pub spend_usd: Option<f64>,
    pub split: Vec<TokenSplit>,
}

/// Estimates at list price (ADR-0051): `None` when the window has tokens but
/// none of its models has a listed price.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Spend {
    pub window: Option<f64>,
    pub today: Option<f64>,
    pub month: Option<f64>,
    pub all: Option<f64>,
    /// Models in the window without a listed price (their tokens are counted, their spend is not).
    pub unpriced_models: Vec<String>,
    pub prices_as_of: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct TokensSplit {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelRow {
    pub model: String,
    pub source: String,
    pub messages: i64,
    pub tokens: i64,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub spend_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub avatar_variant: String,
    pub parent_id: Option<String>,
    pub model: Option<String>,
    pub state: String,
    pub messages: i64,
    pub tokens: i64,
    pub spend_usd: Option<f64>,
    pub duration_s: i64,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Favorite {
    pub model: String,
    /// Its share of the window's Claude Code tokens, 0 ..= 1.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeakHour {
    pub hour: u8,
    pub messages: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct Streak {
    pub current: i64,
    pub longest: i64,
}

/// `StatsSummary` (03 §7A): everything the view draws for one window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatsSummary {
    pub from: String,
    pub to: String,
    pub spend: Spend,
    /// Every source.
    pub tokens: i64,
    pub tokens_split: TokensSplit,
    /// Claude Code assistant messages.
    pub messages: i64,
    /// Top-level sessions started in the window.
    pub sessions: i64,
    /// Subagent sessions started in the window.
    pub agents: i64,
    pub files: i64,
    pub lines_added: i64,
    pub lines_removed: i64,
    /// Days with any activity, of the window's elapsed days.
    pub active_days: i64,
    pub window_days: i64,
    /// Over the last 400 days, not the window: current ends today or yesterday.
    pub streak: Streak,
    pub longest_session_s: i64,
    /// By Claude Code tokens.
    pub favorite_model: Option<Favorite>,
    /// The local hour with the most Claude Code messages.
    pub peak_hour: Option<PeakHour>,
    /// cache reads / (input + cache reads + cache writes), every source; `None` without input.
    pub cache_hit: Option<f64>,
    pub by_model: Vec<ModelRow>,
    pub by_agent: Vec<AgentRow>,
    pub days: Vec<SummaryDay>,
    /// Claude Code messages per local hour, 0 ..= 23.
    pub hours: Vec<i64>,
    pub by_state: BTreeMap<&'static str, i64>,
}

/// One day's usage while the days are assembled: tokens, spend (priced parts added, `None`
/// once a part is unpriced and nothing priced came before), whether a priced part was seen,
/// the split.
type DayUsage = (i64, Option<f64>, bool, Vec<TokenSplit>);

/// Usage of a window keyed by (day, source, model): live groups priced now,
/// rolled rows as they were priced.
fn usage_between(
    conn: &Connection,
    from: &str,
    to_excl: &str,
    cutoff_day: &str,
) -> Result<BTreeMap<(String, String, String), Merged>> {
    let mut out: BTreeMap<(String, String, String), Merged> = BTreeMap::new();
    let live_from = if from < cutoff_day { cutoff_day } else { from };
    if live_from < to_excl {
        for g in rollup::live_groups(conn, live_from, to_excl, None)? {
            let spend = rollup::group_spend(&g);
            out.entry((g.day.clone(), g.source.clone(), g.model.clone()))
                .or_default()
                .add(g.messages, &g.counts, spend);
        }
    }
    if from < cutoff_day {
        let mut st = conn.prepare(
            "SELECT day, source, model, messages, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, spend_usd
               FROM activity_models
              WHERE day >= :from AND day < :to AND day < :cutoff",
        )?;
        let rows = st.query_map(
            named_params! { ":from": from, ":to": to_excl, ":cutoff": cutoff_day },
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    Counts {
                        input: r.get(4)?,
                        output: r.get(5)?,
                        cache_read: r.get(6)?,
                        cache_write: r.get(7)?,
                        cache_write_1h: 0,
                    },
                    r.get::<_, Option<f64>>(8)?,
                ))
            },
        )?;
        for row in rows {
            let (day, source, model, messages, counts, spend) = row?;
            out.entry((day, source, model))
                .or_default()
                .add(messages, &counts, spend);
        }
    }
    Ok(out)
}

/// The spend of a window: the priced parts added, `None` when tokens exist but nothing is priced.
fn spend_total(usage: &BTreeMap<(String, String, String), Merged>) -> Option<f64> {
    let mut total = 0.0;
    let mut priced = false;
    let mut tokens = 0;
    for m in usage.values() {
        tokens += m.counts.input + m.counts.output + m.counts.cache_read + m.counts.cache_write;
        if let Some(s) = m.spend {
            total += s;
            priced = true;
        }
    }
    if priced || tokens == 0 {
        Some(total)
    } else {
        None
    }
}

fn spend_between(
    conn: &Connection,
    from: &str,
    to_excl: &str,
    cutoff_day: &str,
) -> Result<Option<f64>> {
    Ok(spend_total(&usage_between(
        conn, from, to_excl, cutoff_day,
    )?))
}

#[derive(Debug, Default, Clone, Copy)]
struct DayAct {
    sessions: i64,
    files: i64,
    agents: i64,
    lines_added: i64,
    lines_removed: i64,
    longest_s: i64,
    /// Claude Code assistant messages (`activity_days.messages` for rolled and imported days).
    messages: i64,
}

/// Q12: sessions, files, agents, lines, messages and the longest session per local day.
fn activity_between(
    conn: &Connection,
    from: &str,
    to: &str,
    cutoff_day: &str,
) -> Result<BTreeMap<String, DayAct>> {
    let mut st = conn.prepare(
        "WITH live(day, sessions, files, agents, lines_added, lines_removed, duration_s, messages) AS (
           SELECT date(s.started_at, 'localtime'), 1, 0, 0, 0, 0,
                  CAST(MAX(0, (julianday(COALESCE(s.ended_at, s.last_event_at)) - julianday(s.started_at)) * 86400) AS INTEGER), 0
             FROM sessions s
            WHERE s.parent_session_id IS NULL AND date(s.started_at, 'localtime') >= :cutoff
           UNION ALL
           SELECT date(s.started_at, 'localtime'), 0, 0, 1, 0, 0, 0, 0
             FROM sessions s
            WHERE s.parent_session_id IS NOT NULL AND date(s.started_at, 'localtime') >= :cutoff
           UNION ALL
           SELECT date(f.first_seen_at, 'localtime'), 0, 1, 0, f.lines_added, f.lines_removed, 0, 0
             FROM file_changes f
            WHERE date(f.first_seen_at, 'localtime') >= :cutoff
           UNION ALL
           SELECT date(u.sampled_at, 'localtime'), 0, 0, 0, 0, 0, 0, 1
             FROM usage_samples u
            WHERE u.source = 'claude_code' AND date(u.sampled_at, 'localtime') >= :cutoff
         ),
         rolled(day, sessions, files, agents, lines_added, lines_removed, duration_s, messages) AS (
           SELECT day, sessions_started, files_changed, agents_started, lines_added, lines_removed, longest_session_s, messages
             FROM activity_days
            WHERE day < :cutoff
         )
         SELECT day, SUM(sessions), SUM(files), SUM(agents), SUM(lines_added), SUM(lines_removed), MAX(duration_s), SUM(messages)
           FROM (SELECT * FROM live UNION ALL SELECT * FROM rolled)
          WHERE day >= :from AND day <= :to
          GROUP BY day",
    )?;
    let rows = st.query_map(
        named_params! { ":cutoff": cutoff_day, ":from": from, ":to": to },
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                DayAct {
                    sessions: r.get(1)?,
                    files: r.get(2)?,
                    agents: r.get(3)?,
                    lines_added: r.get(4)?,
                    lines_removed: r.get(5)?,
                    longest_s: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    messages: r.get(7)?,
                },
            ))
        },
    )?;
    rows.collect::<std::result::Result<BTreeMap<_, _>, _>>()
        .map_err(Into::into)
}

/// Q13: Claude Code messages per local hour.
fn hours_between(
    conn: &Connection,
    from: &str,
    to_excl: &str,
    cutoff_day: &str,
) -> Result<[i64; 24]> {
    let mut hours = [0i64; 24];
    let live_from = if from < cutoff_day { cutoff_day } else { from };
    let mut st = conn.prepare(
        "SELECT CAST(strftime('%H', u.sampled_at, 'localtime') AS INTEGER) AS hour, COUNT(*)
           FROM usage_samples u
          WHERE u.source = 'claude_code'
            AND date(u.sampled_at, 'localtime') >= :live_from AND date(u.sampled_at, 'localtime') < :to
          GROUP BY hour
         UNION ALL
         SELECT hour, SUM(messages) FROM activity_hours
          WHERE day >= :from AND day < :to AND day < :cutoff
          GROUP BY hour",
    )?;
    let rows = st.query_map(
        named_params! { ":live_from": live_from, ":from": from, ":to": to_excl, ":cutoff": cutoff_day },
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    for row in rows {
        let (h, n) = row?;
        if (0..24).contains(&h) {
            hours[h as usize] += n;
        }
    }
    Ok(hours)
}

/// The sessions with usage in the window (live rows only), by tokens.
fn agents_between(
    conn: &Connection,
    from: &str,
    to_excl: &str,
    limit: usize,
) -> Result<Vec<AgentRow>> {
    struct Group {
        session: String,
        model: String,
        long: bool,
        messages: i64,
        counts: Counts,
    }
    let mut st = conn.prepare(
        "SELECT u.session_id, u.model,
                (u.input_tokens + u.cache_read_tokens + u.cache_write_tokens) > :long AS long,
                COUNT(*), SUM(u.input_tokens), SUM(u.output_tokens), SUM(u.cache_read_tokens),
                SUM(u.cache_write_tokens), SUM(u.cache_write_1h_tokens)
           FROM usage_samples u
          WHERE u.session_id IS NOT NULL AND u.source = 'claude_code'
            AND date(u.sampled_at, 'localtime') >= :from AND date(u.sampled_at, 'localtime') < :to
          GROUP BY u.session_id, u.model, long",
    )?;
    let groups = st
        .query_map(
            named_params! { ":long": LONG_PROMPT_TOKENS, ":from": from, ":to": to_excl },
            |r| {
                Ok(Group {
                    session: r.get(0)?,
                    model: r.get(1)?,
                    long: r.get::<_, i64>(2)? != 0,
                    messages: r.get(3)?,
                    counts: Counts {
                        input: r.get(4)?,
                        output: r.get(5)?,
                        cache_read: r.get(6)?,
                        cache_write: r.get(7)?,
                        cache_write_1h: r.get(8)?,
                    },
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut per: BTreeMap<String, (Merged, BTreeMap<String, i64>)> = BTreeMap::new();
    for g in groups {
        let spend = pricing::rates(&g.model, g.long).map(|r| pricing::spend(&g.counts, &r));
        let e = per.entry(g.session.clone()).or_default();
        e.0.add(g.messages, &g.counts, spend);
        *e.1.entry(g.model).or_insert(0) +=
            g.counts.input + g.counts.output + g.counts.cache_read + g.counts.cache_write;
    }
    let mut st = conn.prepare(
        "SELECT s.id, s.display_name, s.avatar_variant, s.parent_session_id, s.model, s.state, s.started_at,
                CAST(MAX(0, (julianday(COALESCE(s.ended_at, s.last_event_at)) - julianday(s.started_at)) * 86400) AS INTEGER)
           FROM sessions s WHERE s.id = :id",
    )?;
    let mut out = Vec::with_capacity(per.len());
    for (id, (m, models)) in &per {
        let row = st
            .query_row(named_params! { ":id": id }, |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })
            .ok();
        let Some((name, variant, parent, model, state, started_at, duration_s)) = row else {
            continue;
        };
        // The model the session used most, else what the session row says.
        let used = models
            .iter()
            .max_by_key(|(_, t)| **t)
            .map(|(m, _)| m.clone());
        out.push(AgentRow {
            id: id.clone(),
            name,
            avatar_variant: variant,
            parent_id: parent,
            model: used.or(model),
            state,
            messages: m.messages,
            tokens: m.counts.input + m.counts.output + m.counts.cache_read + m.counts.cache_write,
            spend_usd: m.spend,
            duration_s,
            started_at,
        });
    }
    out.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| a.started_at.cmp(&b.started_at))
    });
    out.truncate(limit);
    Ok(out)
}

/// `StatsSummary` for the local days `from ..= to`.
pub fn summary(
    conn: &Connection,
    from: &str,
    to: &str,
) -> std::result::Result<StatsSummary, StatsError> {
    summary_at(conn, from, to, &Clock::current(conn)?)
}

/// [`summary`] at a given clock: today, the month and the live cutoff follow it (tests).
pub fn summary_at(
    conn: &Connection,
    from: &str,
    to: &str,
    clock: &Clock,
) -> std::result::Result<StatsSummary, StatsError> {
    let (f, t) = window(from, to)?;
    let live_days = settings::retention_in(conn)?.sessions_days.max(1);
    let cutoff_day = local_day_shift(conn, &clock.now, -live_days)?;
    let to_excl = date_of(t + 1);

    let usage = usage_between(conn, from, &to_excl, &cutoff_day)?;
    let acts = activity_between(conn, from, to, &cutoff_day)?;
    let hours = hours_between(conn, from, &to_excl, &cutoff_day)?;
    let by_agent = agents_between(conn, from, &to_excl, 100)?;

    // The days, with the usage split per day (messages come from the day activity: imported
    // history has them per day, not per model).
    let mut per_day_usage: BTreeMap<&str, DayUsage> = BTreeMap::new();
    for ((day, source, model), m) in &usage {
        let e = per_day_usage
            .entry(day.as_str())
            .or_insert((0, Some(0.0), false, Vec::new()));
        let tokens = m.counts.input + m.counts.output + m.counts.cache_read + m.counts.cache_write;
        e.0 += tokens;
        match m.spend {
            Some(s) => {
                e.1 = Some(e.1.unwrap_or(0.0) + s);
                e.2 = true;
            }
            None => {
                if !e.2 {
                    e.1 = None;
                }
            }
        }
        e.3.push(TokenSplit {
            model: model.clone(),
            source: source.clone(),
            tokens,
        });
    }
    let days: Vec<SummaryDay> = (f..=t)
        .map(|n| {
            let date = date_of(n);
            let act = acts.get(&date).copied().unwrap_or_default();
            let (tokens, spend, _, mut split) =
                per_day_usage
                    .remove(date.as_str())
                    .unwrap_or((0, Some(0.0), false, Vec::new()));
            split.sort_by_key(|p| std::cmp::Reverse(p.tokens));
            SummaryDay {
                date,
                sessions: act.sessions,
                messages: act.messages,
                tokens,
                spend_usd: spend,
                split,
            }
        })
        .collect();

    // By model and source.
    let mut models: BTreeMap<(String, String), Merged> = BTreeMap::new();
    for ((_, source, model), m) in &usage {
        models
            .entry((model.clone(), source.clone()))
            .or_default()
            .add(m.messages, &m.counts, m.spend);
    }
    let mut by_model: Vec<ModelRow> = models
        .into_iter()
        .map(|((model, source), m)| ModelRow {
            model,
            source,
            messages: m.messages,
            tokens: m.counts.input + m.counts.output + m.counts.cache_read + m.counts.cache_write,
            input: m.counts.input,
            output: m.counts.output,
            cache_read: m.counts.cache_read,
            cache_write: m.counts.cache_write,
            spend_usd: m.spend,
        })
        .collect();
    by_model.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.model.cmp(&b.model)));
    let mut unpriced: Vec<String> = by_model
        .iter()
        .filter(|r| r.spend_usd.is_none() && r.tokens > 0)
        .map(|r| r.model.clone())
        .collect();
    unpriced.dedup();

    let mut split = TokensSplit::default();
    for r in &by_model {
        split.input += r.input;
        split.output += r.output;
        split.cache_read += r.cache_read;
        split.cache_write += r.cache_write;
    }
    let tokens = split.input + split.output + split.cache_read + split.cache_write;
    let prompt = split.input + split.cache_read + split.cache_write;
    let cache_hit = (prompt > 0).then(|| split.cache_read as f64 / prompt as f64);
    let cc_tokens: i64 = by_model
        .iter()
        .filter(|r| r.source == "claude_code")
        .map(|r| r.tokens)
        .sum();
    let favorite_model = by_model
        .iter()
        .filter(|r| r.source == "claude_code" && r.tokens > 0)
        .max_by_key(|r| r.tokens)
        .map(|r| Favorite {
            model: r.model.clone(),
            share: if cc_tokens > 0 {
                r.tokens as f64 / cc_tokens as f64
            } else {
                0.0
            },
        });
    let messages: i64 = acts.values().map(|a| a.messages).sum();
    let peak_hour = hours
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .max_by_key(|(_, n)| **n)
        .map(|(h, n)| PeakHour {
            hour: h as u8,
            messages: *n,
        });

    // The spend tiles: the window, today, this month, the last 400 days.
    let month_start = format!("{}-01", &clock.today[..7]);
    let all_from = date_of(
        queries::day_number(&clock.today).unwrap_or(t) - (crate::stats::STATS_MAX_DAYS - 1),
    );
    let spend = Spend {
        window: spend_total(&usage),
        today: spend_between(conn, &clock.today, &clock.tomorrow, &cutoff_day)?,
        month: spend_between(conn, &month_start, &clock.tomorrow, &cutoff_day)?,
        all: spend_between(conn, &all_from, &clock.tomorrow, &cutoff_day)?,
        unpriced_models: unpriced,
        prices_as_of: PRICES_AS_OF,
    };

    // Counts over the window.
    let sessions: i64 = acts.values().map(|a| a.sessions).sum();
    let agents: i64 = acts.values().map(|a| a.agents).sum();
    let files: i64 = acts.values().map(|a| a.files).sum();
    let lines_added: i64 = acts.values().map(|a| a.lines_added).sum();
    let lines_removed: i64 = acts.values().map(|a| a.lines_removed).sum();
    let longest_session_s = acts.values().map(|a| a.longest_s).max().unwrap_or(0);
    let last_elapsed = if clock.today.as_str() < to {
        clock.today.as_str()
    } else {
        to
    };
    let window_days = (queries::day_number(last_elapsed).unwrap_or(t) - f + 1).max(0);
    let active_days = days
        .iter()
        .filter(|d| d.date.as_str() <= last_elapsed)
        .filter(|d| d.sessions + d.messages + acts.get(&d.date).map(|a| a.files).unwrap_or(0) > 0)
        .count() as i64;

    // The streak over the whole tracker range, not the window.
    let streak = {
        let from_day = date_of(
            queries::day_number(&clock.today).unwrap_or(t) - (crate::stats::STATS_MAX_DAYS - 1),
        );
        let rows = queries::activity_days(conn, &from_day, &cutoff_day)?;
        let (current, longest) = queries::streaks(&rows, &clock.today);
        Streak { current, longest }
    };

    let (start, end) = queries::local_day_bounds(conn, from, to)?;
    let mut by_state: BTreeMap<&'static str, i64> = [
        "waiting",
        "working",
        "failed",
        "finished",
        "idle",
        "untracked",
    ]
    .into_iter()
    .map(|w| (w, 0))
    .collect();
    for (state, n) in queries::sessions_by_state(conn, &start, &end)? {
        if let Some(w) = state_word(&state) {
            *by_state.entry(w).or_insert(0) += n;
        }
    }

    Ok(StatsSummary {
        from: from.to_string(),
        to: to.to_string(),
        spend,
        tokens,
        tokens_split: split,
        messages,
        sessions,
        agents,
        files,
        lines_added,
        lines_removed,
        active_days,
        window_days,
        streak,
        longest_session_s,
        favorite_model,
        peak_hour,
        cache_hit,
        by_model,
        by_agent,
        days,
        hours: hours.to_vec(),
        by_state,
    })
}
