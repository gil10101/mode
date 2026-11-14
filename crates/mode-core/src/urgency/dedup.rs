//! De-duplication (07 §7): the dedup key stored on every message (one Inbox
//! row per key, twins across accounts) and the item keys of alert events
//! (one alert per rule per key). Twins: the primary row is the earliest
//! `fetched_at` (ties by `id`); tiers are copied between twins; triage and
//! snoozes fan out to all of them.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::Result;

/// `email:<rfc_message_id lowercased>`, or `email:<account_id>:<external_id>`
/// without the header (no cross-account collapse).
pub fn email_key(rfc_message_id: Option<&str>, account_id: &str, external_id: &str) -> String {
    match rfc_message_id
        .map(|m| {
            m.trim()
                .trim_matches(|c| c == '<' || c == '>')
                .to_lowercase()
        })
        .filter(|m| !m.is_empty())
    {
        Some(m) => format!("email:{m}"),
        None => format!("email:{account_id}:{external_id}"),
    }
}

/// `slack:<team_id>:<channel_id>:<ts>`.
pub fn slack_key(team_id: &str, channel_id: &str, ts: &str) -> String {
    format!("slack:{team_id}:{channel_id}:{ts}")
}

/// A pending approval: `session:<claude_session_id>:<tool_use_id>`.
pub fn session_permission_key(claude_session_id: &str, tool_use_id: &str) -> String {
    format!("session:{claude_session_id}:{tool_use_id}")
}

/// Other waits: `session:<claude_session_id>:<state_since>`.
pub fn session_wait_key(claude_session_id: &str, state_since: &str) -> String {
    format!("session:{claude_session_id}:{state_since}")
}

/// Finished / failed: `session:<claude_session_id>:end:<ended_at>`.
pub fn session_end_key(claude_session_id: &str, ended_at: &str) -> String {
    format!("session:{claude_session_id}:end:{ended_at}")
}

/// `cal:<ical_uid>`, else `cal:<account_id>:<calendar_id>:<external_id>`.
pub fn calendar_key(
    ical_uid: Option<&str>,
    account_id: &str,
    calendar_id: &str,
    external_id: &str,
) -> String {
    match ical_uid.filter(|u| !u.is_empty()) {
        Some(u) => format!("cal:{u}"),
        None => format!("cal:{account_id}:{calendar_id}:{external_id}"),
    }
}

/// `note:<id>:<when_date>`.
pub fn note_key(note_id: &str, when_date: &str) -> String {
    format!("note:{note_id}:{when_date}")
}

/// `usage:<percent>:<month>`.
pub fn usage_key(percent: i64, month: &str) -> String {
    format!("usage:{percent}:{month}")
}

/// One row of a twin group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Twin {
    pub id: String,
    pub account_id: String,
    pub thread_id: Option<String>,
    pub fetched_at: String,
}

/// Every row sharing `dedup_key`, primary first.
pub fn twins(conn: &Connection, dedup_key: &str) -> Result<Vec<Twin>> {
    let mut st = conn.prepare(
        "SELECT id, account_id, thread_id, fetched_at FROM messages
          WHERE dedup_key = ?1 ORDER BY fetched_at, id",
    )?;
    let rows = st.query_map([dedup_key], |r| {
        Ok(Twin {
            id: r.get(0)?,
            account_id: r.get(1)?,
            thread_id: r.get(2)?,
            fetched_at: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The ids of a message's twin group (itself included), primary first.
pub fn group_of(conn: &Connection, message_id: &str) -> Result<Vec<String>> {
    let key: Option<String> = conn
        .query_row(
            "SELECT dedup_key FROM messages WHERE id = ?1",
            [message_id],
            |r| r.get(0),
        )
        .optional()?;
    match key {
        Some(k) => Ok(twins(conn, &k)?.into_iter().map(|t| t.id).collect()),
        None => Ok(Vec::new()),
    }
}

/// Whether `message_id` is the primary row of its group.
pub fn is_primary(conn: &Connection, message_id: &str) -> Result<bool> {
    Ok(group_of(conn, message_id)?.first().map(String::as_str) == Some(message_id))
}

/// A twin of `message_id` (other row, same key) that already has a tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TieredTwin {
    pub tier: String,
    pub why: Option<String>,
    pub source: Option<String>,
    pub model: Option<String>,
    pub scored_at: Option<String>,
}

pub fn tiered_twin(conn: &Connection, message_id: &str) -> Result<Option<TieredTwin>> {
    Ok(conn
        .query_row(
            "SELECT t.urgency_tier, t.urgency_why, t.urgency_source, t.urgency_model, t.scored_at
               FROM messages m JOIN messages t ON t.dedup_key = m.dedup_key AND t.id <> m.id
              WHERE m.id = ?1 AND t.urgency_tier <> 'unscored'
              ORDER BY t.fetched_at, t.id LIMIT 1",
            params![message_id],
            |r| {
                Ok(TieredTwin {
                    tier: r.get(0)?,
                    why: r.get(1)?,
                    source: r.get(2)?,
                    model: r.get(3)?,
                    scored_at: r.get(4)?,
                })
            },
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_07_section_7_1() {
        assert_eq!(
            email_key(Some("<CAB12@Mail.Gmail.com>"), "a", "x"),
            "email:cab12@mail.gmail.com"
        );
        assert_eq!(email_key(None, "acc", "m1"), "email:acc:m1");
        assert_eq!(email_key(Some("  "), "acc", "m1"), "email:acc:m1");
        assert_eq!(slack_key("T1", "C1", "1.2"), "slack:T1:C1:1.2");
        assert_eq!(
            calendar_key(Some("uid@google.com"), "a", "c", "e"),
            "cal:uid@google.com"
        );
        assert_eq!(calendar_key(None, "a", "c", "e"), "cal:a:c:e");
        assert_eq!(session_end_key("s", "t"), "session:s:end:t");
        assert_eq!(usage_key(80, "2026-10"), "usage:80:2026-10");
        assert_eq!(note_key("n", "2026-10-22"), "note:n:2026-10-22");
    }
}
