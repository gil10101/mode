//! The durable write-back queue (06 §2.5) on `connector_writebacks`: one
//! row per (account, target, action), coalesced (a later opposite action
//! cancels the queued one before it is sent); drained with the retry ladder
//! immediately → 30 s → 2 min → 10 min → 1 h → hourly; dropped after 24 h
//! (the message then reads "not synced to Gmail / Slack"); paused accounts
//! are skipped; a 401 goes through refresh inside the adapter's [`Api`].
//!
//! [`Api`]: super::Api

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use super::{ConnectorError, TriageAction};
use crate::db::{Db, Result, format_timestamp, parse_timestamp};
use crate::repo::{self, Account, ConnectorWriteback};

/// The retry ladder (06 §2.5): immediately, then 30 s, 2 min, 10 min, 1 h,
/// then once per hour.
pub const LADDER_SECS: [i64; 5] = [0, 30, 120, 600, 3600];
/// Rows older than this are dropped.
pub const DROP_AFTER_HOURS: i64 = 24;
/// The label a message gets when its write-back was dropped.
pub const NOT_SYNCED_LABEL: &str = "mode:not_synced";

/// The delay before attempt `attempts + 1` (after `attempts` failures).
pub fn delay_after(attempts: i64) -> Duration {
    let i = attempts.max(0) as usize;
    Duration::seconds(*LADDER_SECS.get(i).unwrap_or(&3600))
}

/// The next attempt time, or `None` when the row has aged out (24 h).
pub fn next_attempt(
    attempts: i64,
    created_at: OffsetDateTime,
    now: OffsetDateTime,
) -> Option<OffsetDateTime> {
    let at = now + delay_after(attempts);
    (at - created_at <= Duration::hours(DROP_AFTER_HOURS)).then_some(at)
}

/// What [`enqueue`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enqueued {
    Queued(String),
    /// A queued opposite action was canceled instead (nothing to send).
    Cancelled,
    /// The same action is already queued.
    Duplicate,
}

/// Queue an action, coalescing with what is already queued for the target.
pub fn enqueue(
    conn: &Connection,
    account_id: &str,
    target_kind: &str,
    target_id: &str,
    action: TriageAction,
    external_id: &str,
    now: OffsetDateTime,
) -> Result<Enqueued> {
    let opposite = conn.execute(
        "DELETE FROM connector_writebacks WHERE account_id = ?1 AND target_kind = ?2
            AND target_id = ?3 AND action = ?4",
        params![
            account_id,
            target_kind,
            target_id,
            action.opposite().as_str()
        ],
    )?;
    if opposite > 0 {
        return Ok(Enqueued::Cancelled);
    }
    let same: Option<String> = conn
        .query_row(
            "SELECT id FROM connector_writebacks WHERE account_id = ?1 AND target_kind = ?2
                AND target_id = ?3 AND action = ?4",
            params![account_id, target_kind, target_id, action.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    if same.is_some() {
        return Ok(Enqueued::Duplicate);
    }
    let now_s = format_timestamp(now);
    let row = ConnectorWriteback {
        id: repo::new_id(),
        account_id: account_id.into(),
        target_kind: target_kind.into(),
        target_id: target_id.into(),
        action: action.as_str().into(),
        external_id: external_id.into(),
        attempts: 0,
        next_attempt_at: Some(now_s.clone()),
        last_error: None,
        created_at: now_s,
    };
    repo::insert(conn, &row)?;
    Ok(Enqueued::Queued(row.id))
}

/// Whether `action` is queued for a target (or the thread of a message).
pub fn pending(
    conn: &Connection,
    target_kind: &str,
    target_id: &str,
    action: TriageAction,
) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM connector_writebacks WHERE target_kind = ?1 AND target_id = ?2 AND action = ?3",
            params![target_kind, target_id, action.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Drop queued rows of `action` for a target (the provider already shows
/// it, or the provider won a conflict). Returns how many.
pub fn drop_matching(
    conn: &Connection,
    target_kind: &str,
    target_id: &str,
    action: TriageAction,
) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM connector_writebacks WHERE target_kind = ?1 AND target_id = ?2 AND action = ?3",
        params![target_kind, target_id, action.as_str()],
    )?)
}

/// Rows due now for accounts that are `active` (paused / broken accounts
/// keep their queue but are not drained).
pub fn due(conn: &Connection, now: OffsetDateTime) -> Result<Vec<ConnectorWriteback>> {
    repo::query::<ConnectorWriteback>(
        conn,
        "(next_attempt_at IS NULL OR next_attempt_at <= ?1)
         AND account_id IN (SELECT id FROM accounts WHERE status = 'active')
         ORDER BY created_at",
        &[&format_timestamp(now)],
    )
}

/// The outcome of one attempt, as the drain records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attempt {
    Done,
    /// Retry later on the ladder.
    Retry(String),
    /// Unsupported now (scope lost): the action stays local.
    Unsupported,
    /// Rate limited: retry after this many seconds without counting it.
    Wait(u64),
}

impl From<std::result::Result<(), ConnectorError>> for Attempt {
    fn from(r: std::result::Result<(), ConnectorError>) -> Self {
        match r {
            Ok(()) => Attempt::Done,
            Err(ConnectorError::Unsupported) | Err(ConnectorError::MissingScope(_)) => {
                Attempt::Unsupported
            }
            Err(ConnectorError::NotFound) => Attempt::Done,
            Err(ConnectorError::RateLimited { retry_after_secs }) => {
                Attempt::Wait(retry_after_secs)
            }
            Err(e) => Attempt::Retry(crate::redact::scrub(&e.to_string()).into_owned()),
        }
    }
}

/// What a drain did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainReport {
    pub sent: usize,
    pub retried: usize,
    pub dropped: usize,
    pub unsupported: usize,
    /// Messages that now carry the "not synced" label.
    pub not_synced: Vec<String>,
}

fn mark_not_synced(conn: &Connection, row: &ConnectorWriteback, now: &str) -> Result<Vec<String>> {
    let ids: Vec<String> = if row.target_kind == "thread" {
        let mut st = conn.prepare("SELECT id FROM messages WHERE thread_id = ?1")?;
        st.query_map([&row.target_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        vec![row.target_id.clone()]
    };
    for id in &ids {
        conn.execute(
            "UPDATE messages SET labels_json = json_insert(labels_json, '$[#]', ?2), updated_at = ?3
              WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM json_each(labels_json) WHERE value = ?2)",
            params![id, NOT_SYNCED_LABEL, now],
        )?;
    }
    Ok(ids)
}

/// Record one attempt's outcome on its row.
pub fn record(
    conn: &Connection,
    row: &ConnectorWriteback,
    outcome: &Attempt,
    now: OffsetDateTime,
) -> Result<DrainReport> {
    let now_s = format_timestamp(now);
    let mut rep = DrainReport::default();
    match outcome {
        Attempt::Done => {
            conn.execute("DELETE FROM connector_writebacks WHERE id = ?1", [&row.id])?;
            rep.sent = 1;
        }
        Attempt::Unsupported => {
            conn.execute("DELETE FROM connector_writebacks WHERE id = ?1", [&row.id])?;
            rep.unsupported = 1;
        }
        Attempt::Wait(secs) => {
            conn.execute(
                "UPDATE connector_writebacks SET next_attempt_at = ?2 WHERE id = ?1",
                params![
                    row.id,
                    format_timestamp(now + Duration::seconds(*secs as i64))
                ],
            )?;
        }
        Attempt::Retry(err) => {
            let attempts = row.attempts + 1;
            let created = parse_timestamp(&row.created_at).unwrap_or(now);
            match next_attempt(attempts, created, now) {
                Some(at) => {
                    conn.execute(
                        "UPDATE connector_writebacks SET attempts = ?2, next_attempt_at = ?3, last_error = ?4
                          WHERE id = ?1",
                        params![row.id, attempts, format_timestamp(at), err],
                    )?;
                    rep.retried = 1;
                }
                None => {
                    conn.execute("DELETE FROM connector_writebacks WHERE id = ?1", [&row.id])?;
                    rep.not_synced = mark_not_synced(conn, row, &now_s)?;
                    rep.dropped = 1;
                }
            }
        }
    }
    Ok(rep)
}

/// Drop rows older than 24 h that never got sent (an account stayed
/// paused or offline): the messages read "not synced".
pub fn expire(conn: &Connection, now: OffsetDateTime) -> Result<DrainReport> {
    let cutoff = format_timestamp(now - Duration::hours(DROP_AFTER_HOURS));
    let old = repo::query::<ConnectorWriteback>(conn, "created_at < ?1", &[&cutoff])?;
    let mut rep = DrainReport::default();
    let now_s = format_timestamp(now);
    for r in old {
        conn.execute("DELETE FROM connector_writebacks WHERE id = ?1", [&r.id])?;
        rep.not_synced.extend(mark_not_synced(conn, &r, &now_s)?);
        rep.dropped += 1;
    }
    Ok(rep)
}

/// Drain the due rows: `send` performs one write-back for an account (the
/// service passes the adapter call). Each outcome is recorded on its own
/// write so a crash mid-drain loses nothing.
pub fn drain(
    db: &Db,
    now: OffsetDateTime,
    mut send: impl FnMut(&Account, &ConnectorWriteback) -> std::result::Result<(), ConnectorError>,
) -> Result<DrainReport> {
    let mut total = db.write(move |c| expire(c, now))?;
    let rows = db.read(|c| due(c, now))?;
    for row in rows {
        let Some(account) = db.read(|c| repo::get_by_id::<Account>(c, &row.account_id))? else {
            continue;
        };
        let outcome: Attempt = send(&account, &row).into();
        let r = row.clone();
        let rep = db.write(move |c| record(c, &r, &outcome, now))?;
        total.sent += rep.sent;
        total.retried += rep.retried;
        total.dropped += rep.dropped;
        total.unsupported += rep.unsupported;
        total.not_synced.extend(rep.not_synced);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_and_24h_drop() {
        let t0 = parse_timestamp("2026-10-22T00:00:00Z").unwrap();
        assert_eq!(delay_after(0), Duration::ZERO);
        assert_eq!(delay_after(1), Duration::seconds(30));
        assert_eq!(delay_after(2), Duration::seconds(120));
        assert_eq!(delay_after(3), Duration::seconds(600));
        assert_eq!(delay_after(4), Duration::seconds(3600));
        assert_eq!(delay_after(9), Duration::seconds(3600));
        assert!(next_attempt(5, t0, t0 + Duration::hours(22)).is_some());
        assert!(next_attempt(5, t0, t0 + Duration::hours(23) + Duration::minutes(30)).is_none());
    }
}
