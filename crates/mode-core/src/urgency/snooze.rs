//! Snooze (07 §6): always app-local, one active `snoozes` row per target,
//! presets resolved in the device's zone (§6.2), twins snoozed together
//! (§7.2), the every-minute sweep S2 (fires `until <= now`), and wake on
//! reply (§6.3: an inbound message into a snoozed thread fires it early with
//! `fired_reason = 'reply'`).

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime, PrimitiveDateTime, Time};

use crate::db::{DbError, Result, format_timestamp, parse_timestamp};
use crate::notes::fields::LocalTz;
use crate::repo::{self, Snooze};

/// `snoozes.preset` (07 §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    LaterToday,
    Tomorrow,
    NextWeek,
    PickDate,
    UntilReply,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::LaterToday => "later_today",
            Preset::Tomorrow => "tomorrow",
            Preset::NextWeek => "next_week",
            Preset::PickDate => "pick_date",
            Preset::UntilReply => "until_reply",
        }
    }
}

/// The working-day times of v1 (07 §6.2).
pub const MORNING: (u8, u8) = (9, 0);
pub const EVENING: (u8, u8) = (18, 0);
/// The safety cap of `until_reply`.
pub const UNTIL_REPLY_DAYS: i64 = 30;
/// `pick_date` must be at least this far ahead.
pub const PICK_MIN_MINUTES: i64 = 5;

fn at(tz: &dyn LocalTz, date: time::Date, hm: (u8, u8)) -> OffsetDateTime {
    let t = Time::from_hms(hm.0, hm.1, 0).unwrap_or(Time::MIDNIGHT);
    tz.to_utc(PrimitiveDateTime::new(date, t))
}

/// Resolve a preset to a UTC `until` (07 §6.2).
pub fn resolve(
    preset: Preset,
    now: OffsetDateTime,
    tz: &dyn LocalTz,
    pick: Option<OffsetDateTime>,
) -> std::result::Result<OffsetDateTime, String> {
    let local = tz.to_local(now);
    let today = local.date();
    let tomorrow = today + Duration::days(1);
    Ok(match preset {
        Preset::LaterToday => {
            let evening = at(tz, today, EVENING);
            if local.hour() >= EVENING.0 {
                at(tz, tomorrow, MORNING)
            } else if local.hour() >= 15 {
                evening
            } else {
                (now + Duration::hours(3)).min(evening)
            }
        }
        Preset::Tomorrow => at(tz, tomorrow, MORNING),
        Preset::NextWeek => {
            let from = today.weekday().number_days_from_monday() as i64;
            let days = 7 - from; // Monday → 7, Sunday → 1
            at(tz, today + Duration::days(days), MORNING)
        }
        Preset::PickDate => {
            let p = pick.ok_or("pick a date and time")?;
            if p < now + Duration::minutes(PICK_MIN_MINUTES) {
                return Err("pick a time at least 5 minutes from now".into());
            }
            p
        }
        Preset::UntilReply => now + Duration::days(UNTIL_REPLY_DAYS),
    })
}

/// `target_kind` values (09 §5.5).
pub fn valid_kind(kind: &str) -> bool {
    matches!(
        kind,
        "message" | "thread" | "note" | "session" | "calendar_event"
    )
}

/// The targets a snooze fans out to: twins of a message, and the threads of
/// a thread's twins in other accounts (07 §6.1 "Twins").
pub fn twin_targets(conn: &Connection, kind: &str, id: &str) -> Result<Vec<(String, String)>> {
    let mut out = vec![(kind.to_string(), id.to_string())];
    match kind {
        "message" => {
            for t in super::dedup::group_of(conn, id)? {
                if t != id {
                    out.push(("message".into(), t));
                }
            }
        }
        "thread" => {
            let mut st = conn.prepare(
                "SELECT DISTINCT o.thread_id FROM messages m
                   JOIN messages o ON o.dedup_key = m.dedup_key AND o.thread_id <> m.thread_id
                  WHERE m.thread_id = ?1 AND o.thread_id IS NOT NULL",
            )?;
            let rows = st.query_map([id], |r| r.get::<_, String>(0))?;
            for t in rows {
                let t = t?;
                if !out.iter().any(|(_, x)| *x == t) {
                    out.push(("thread".into(), t));
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Active rows for a target.
pub fn active_for(conn: &Connection, kind: &str, id: &str, now: &str) -> Result<Vec<Snooze>> {
    repo::query::<Snooze>(
        conn,
        "target_kind = ?1 AND target_id = ?2 AND fired_at IS NULL AND cancelled_at IS NULL
         AND deleted_at IS NULL AND until > ?3",
        &[&kind, &id, &now],
    )
}

pub fn is_snoozed(conn: &Connection, kind: &str, id: &str, now: &str) -> Result<bool> {
    Ok(!active_for(conn, kind, id, now)?.is_empty())
}

/// A snooze request (the `thread_snooze` command).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnoozeRequest {
    pub target_kind: String,
    pub target_id: String,
    #[serde(default)]
    pub preset: Option<Preset>,
    /// For `pick_date` (or a raw time without a preset): a 09 §4.1 timestamp.
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub wake_on_reply: Option<bool>,
}

fn cancel_rows(conn: &Connection, device_id: &str, rows: &[Snooze], now: &str) -> Result<usize> {
    let mut n = 0;
    for r in rows {
        if repo::patch::<Snooze>(conn, device_id, &r.id, &[("cancelled_at", &now)])? {
            n += 1;
        }
    }
    Ok(n)
}

/// Snooze a target and its twins; an existing active snooze is replaced
/// (canceled). Returns the rows created.
pub fn snooze(
    conn: &Connection,
    device_id: &str,
    req: &SnoozeRequest,
    now: OffsetDateTime,
    tz: &dyn LocalTz,
    default_wake_on_reply: bool,
) -> Result<Vec<Snooze>> {
    let invalid = |d: String| DbError::Invalid {
        what: "snooze",
        detail: d,
    };
    if !valid_kind(&req.target_kind) {
        return Err(invalid(format!("{} cannot be snoozed", req.target_kind)));
    }
    let pick = req.until.as_deref().and_then(parse_timestamp);
    if req.until.is_some() && pick.is_none() {
        return Err(invalid("until is not a timestamp".into()));
    }
    let preset = match (req.preset, pick) {
        (Some(p), _) => p,
        (None, Some(_)) => Preset::PickDate,
        (None, None) => return Err(invalid("choose a preset or a time".into())),
    };
    if preset == Preset::UntilReply && req.target_kind != "thread" {
        return Err(invalid("until reply applies to threads".into()));
    }
    let until = resolve(preset, now, tz, pick).map_err(invalid)?;
    let wake = if preset == Preset::UntilReply {
        true
    } else {
        req.wake_on_reply.unwrap_or(default_wake_on_reply)
    };
    let now_s = format_timestamp(now);
    let mut created = Vec::new();
    for (kind, id) in twin_targets(conn, &req.target_kind, &req.target_id)? {
        let old = active_for(conn, &kind, &id, &now_s)?;
        cancel_rows(conn, device_id, &old, &now_s)?;
        let mut row = Snooze {
            id: repo::new_id(),
            target_kind: kind,
            target_id: id,
            until: format_timestamp(until),
            preset: Some(preset.as_str().to_string()),
            wake_on_reply: wake,
            ..Default::default()
        };
        repo::create(conn, device_id, &mut row)?;
        created.push(row);
    }
    Ok(created)
}

/// Cancel snooze (the item returns at once, without the marker); twins too.
pub fn cancel(
    conn: &Connection,
    device_id: &str,
    kind: &str,
    id: &str,
    now: OffsetDateTime,
) -> Result<usize> {
    let now_s = format_timestamp(now);
    let mut n = 0;
    for (k, i) in twin_targets(conn, kind, id)? {
        let rows = active_for(conn, &k, &i, &now_s)?;
        n += cancel_rows(conn, device_id, &rows, &now_s)?;
    }
    Ok(n)
}

/// A snooze that fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fired {
    pub id: String,
    pub target_kind: String,
    pub target_id: String,
    /// `time` | `reply`.
    pub reason: String,
}

/// The sweep S2 (every minute, at startup and on wake): fire every row
/// whose `until` has passed. Resurfacing never emits `message.received`
/// on its own (§6.1).
pub fn sweep(conn: &Connection, device_id: &str, now: OffsetDateTime) -> Result<Vec<Fired>> {
    let now_s = format_timestamp(now);
    let due = repo::query::<Snooze>(
        conn,
        "fired_at IS NULL AND cancelled_at IS NULL AND deleted_at IS NULL AND until <= ?1
         ORDER BY until",
        &[&now_s],
    )?;
    let mut out = Vec::new();
    for s in due {
        if repo::patch::<Snooze>(
            conn,
            device_id,
            &s.id,
            &[("fired_at", &now_s), ("fired_reason", &"time")],
        )? {
            out.push(Fired {
                id: s.id,
                target_kind: s.target_kind,
                target_id: s.target_id,
                reason: "time".into(),
            });
        }
    }
    Ok(out)
}

/// Wake on reply (§6.3): `new_message_id` arrived (inbound) in `thread_id`.
/// Fires active snoozes with `wake_on_reply = 1` on the thread or on an
/// earlier message of it. Returns the snooze ids fired.
pub fn wake_on_reply(
    conn: &Connection,
    device_id: &str,
    thread_id: &str,
    new_message_id: &str,
    now: &str,
) -> Result<Vec<String>> {
    let rows = repo::query::<Snooze>(
        conn,
        "fired_at IS NULL AND cancelled_at IS NULL AND deleted_at IS NULL AND wake_on_reply = 1
         AND until > ?1
         AND ((target_kind = 'thread' AND target_id = ?2)
           OR (target_kind = 'message' AND target_id IN
                 (SELECT id FROM messages WHERE thread_id = ?2 AND id <> ?3)))",
        &[&now, &thread_id, &new_message_id],
    )?;
    let mut out = Vec::new();
    for s in rows {
        if repo::patch::<Snooze>(
            conn,
            device_id,
            &s.id,
            &[("fired_at", &now), ("fired_reason", &"reply")],
        )? {
            out.push(s.id);
        }
    }
    Ok(out)
}

/// Snoozes resurfaced today ("resurfaced" marker, 03 §4.2): fired by time
/// since `since`.
pub fn resurfaced_since(conn: &Connection, kind: &str, id: &str, since: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM snoozes WHERE target_kind = ?1 AND target_id = ?2
            AND fired_at IS NOT NULL AND fired_at >= ?3 AND deleted_at IS NULL",
        params![kind, id, since],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::fields::FixedTz;

    fn utc(s: &str) -> OffsetDateTime {
        parse_timestamp(s).unwrap()
    }

    #[test]
    fn presets_resolve_in_local_time() {
        let tz = FixedTz(120); // UTC+2
        // Wed 2026-10-21 10:00 local = 08:00Z → +3 h = 13:00 local
        let now = utc("2026-10-21T08:00:00Z");
        assert_eq!(
            resolve(Preset::LaterToday, now, &tz, None).unwrap(),
            utc("2026-10-21T11:00:00Z")
        );
        // 16:00 local → 18:00 local
        let now = utc("2026-10-21T14:00:00Z");
        assert_eq!(
            resolve(Preset::LaterToday, now, &tz, None).unwrap(),
            utc("2026-10-21T16:00:00Z")
        );
        // 14:30 local: +3 h = 17:30, still before 18:00
        let now = utc("2026-10-21T12:30:00Z");
        assert_eq!(
            resolve(Preset::LaterToday, now, &tz, None).unwrap(),
            utc("2026-10-21T15:30:00Z")
        );
        // 19:00 local → tomorrow 09:00 local
        let now = utc("2026-10-21T17:00:00Z");
        assert_eq!(
            resolve(Preset::LaterToday, now, &tz, None).unwrap(),
            utc("2026-10-22T07:00:00Z")
        );
        assert_eq!(
            resolve(Preset::Tomorrow, now, &tz, None).unwrap(),
            utc("2026-10-22T07:00:00Z")
        );
        // Wednesday → Monday 26th 09:00 local
        assert_eq!(
            resolve(Preset::NextWeek, now, &tz, None).unwrap(),
            utc("2026-10-26T07:00:00Z")
        );
        // On a Monday: the following Monday
        let monday = utc("2026-10-26T08:00:00Z");
        assert_eq!(
            resolve(Preset::NextWeek, monday, &tz, None).unwrap(),
            utc("2026-11-02T07:00:00Z")
        );
        assert!(resolve(Preset::PickDate, now, &tz, Some(now + Duration::minutes(2))).is_err());
        assert_eq!(
            resolve(Preset::UntilReply, now, &tz, None).unwrap(),
            now + Duration::days(30)
        );
    }
}
