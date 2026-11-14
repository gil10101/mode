//! Ranking (07 §3.5): message order (tier rank, then newest first), the
//! thread tier (highest tier among the thread's unread messages, NULL when
//! none is unread), thread counters, pill candidacy (urgent, unread, not
//! archived, not snoozed, within `urgency.pill_message_max_age_hours`),
//! and the §2.2 invariant checks the tests run over a database.

use std::cmp::Ordering;

use rusqlite::{Connection, OptionalExtension, params};
use time::OffsetDateTime;

use super::Tier;
use crate::db::{Result, parse_timestamp};

/// Inbox order: tier rank, then `received_at` descending.
pub fn compare(a: (Tier, &str), b: (Tier, &str)) -> Ordering {
    a.0.rank().cmp(&b.0.rank()).then_with(|| b.1.cmp(a.1))
}

/// The highest tier among unread tiers (`None` when the list is empty).
pub fn thread_tier<'a>(unread: impl IntoIterator<Item = &'a str>) -> Option<Tier> {
    unread
        .into_iter()
        .filter_map(Tier::parse)
        .min_by_key(|t| (t.rank(), matches!(t, Tier::Unscored) as u8))
}

/// Recompute `threads` counters, `urgency_tier`, `last_message_at`,
/// `last_snippet` and `is_archived` once per batch (09 §5.3: not by trigger).
pub fn recompute_threads(conn: &Connection, thread_ids: &[String], now: &str) -> Result<usize> {
    let mut seen: Vec<&String> = Vec::new();
    let mut n = 0;
    for t in thread_ids {
        if seen.contains(&t) {
            continue;
        }
        seen.push(t);
        let unread: Vec<String> = {
            let mut st = conn.prepare(
                "SELECT urgency_tier FROM messages
                  WHERE thread_id = ?1 AND is_read = 0 AND is_archived = 0",
            )?;
            let rows = st.query_map([t], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let tier = thread_tier(unread.iter().map(String::as_str)).map(|x| x.as_str());
        let latest: Option<(String, String)> = conn
            .query_row(
                "SELECT received_at, snippet FROM messages WHERE thread_id = ?1
                  ORDER BY received_at DESC LIMIT 1",
                [t],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (count, unread_n, archived_n): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(is_read = 0 AND is_archived = 0), 0),
                    COALESCE(SUM(is_archived = 1), 0)
               FROM messages WHERE thread_id = ?1",
            [t],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        n += conn.execute(
            "UPDATE threads SET message_count = ?2, unread_count = ?3, urgency_tier = ?4,
                    last_message_at = COALESCE(?5, last_message_at),
                    last_snippet = COALESCE(?6, last_snippet),
                    is_archived = ?7, updated_at = ?8
              WHERE id = ?1",
            params![
                t,
                count,
                unread_n,
                tier,
                latest.as_ref().map(|l| l.0.clone()),
                latest.as_ref().map(|l| l.1.clone()),
                count > 0 && archived_n == count,
                now
            ],
        )?;
    }
    Ok(n)
}

/// 07 §3.5 pill candidacy for one message.
pub fn pill_candidate(
    tier: Tier,
    is_read: bool,
    is_archived: bool,
    snoozed: bool,
    received_at: &str,
    now: OffsetDateTime,
    max_age_hours: i64,
) -> bool {
    tier.is_urgent()
        && !is_read
        && !is_archived
        && !snoozed
        && parse_timestamp(received_at)
            .is_some_and(|r| r > now - time::Duration::hours(max_age_hours))
}

/// A violated §2.2 invariant, for the tests and Diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub invariant: &'static str,
    pub message_id: String,
}

/// Check I1 (one known tier per row), I5 (urgent tiers carry a why) and the
/// twin rule (twins share tier and why) over the whole table.
pub fn check_invariants(conn: &Connection) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    let mut st = conn.prepare("SELECT id, urgency_tier, urgency_why FROM messages")?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (id, tier, why) = row?;
        match Tier::parse(&tier) {
            None => out.push(Violation {
                invariant: "I1",
                message_id: id,
            }),
            Some(t) if t.is_urgent() && why.as_deref().is_none_or(|w| w.trim().is_empty()) => out
                .push(Violation {
                    invariant: "I5",
                    message_id: id,
                }),
            _ => {}
        }
    }
    let mut st = conn.prepare(
        "SELECT a.id FROM messages a JOIN messages b
            ON a.dedup_key = b.dedup_key AND a.id < b.id
         WHERE a.urgency_tier <> 'unscored' AND b.urgency_tier <> 'unscored'
           AND (a.urgency_tier <> b.urgency_tier OR COALESCE(a.urgency_why, '') <> COALESCE(b.urgency_why, ''))",
    )?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    for id in rows {
        out.push(Violation {
            invariant: "twins",
            message_id: id?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_is_tier_then_newest() {
        let mut v = [
            (Tier::Low, "2026-10-22T10:00:00Z"),
            (Tier::AiNormal, "2026-10-22T09:00:00Z"),
            (Tier::Vip, "2026-10-21T10:00:00Z"),
            (Tier::Unscored, "2026-10-22T11:00:00Z"),
            (Tier::Direct, "2026-10-22T08:00:00Z"),
            (Tier::Direct, "2026-10-22T09:30:00Z"),
        ];
        v.sort_by(|a, b| compare(*a, *b));
        let tiers: Vec<Tier> = v.iter().map(|x| x.0).collect();
        assert_eq!(
            tiers,
            vec![
                Tier::Vip,
                Tier::Direct,
                Tier::Direct,
                Tier::Unscored,
                Tier::AiNormal,
                Tier::Low
            ]
        );
        assert_eq!(v[1].1, "2026-10-22T09:30:00Z");
    }

    #[test]
    fn thread_tier_is_the_highest_unread() {
        assert_eq!(
            thread_tier(["low", "ai-urgent", "direct"]),
            Some(Tier::Direct)
        );
        assert_eq!(thread_tier(["unscored", "ai-normal"]), Some(Tier::AiNormal));
        assert_eq!(thread_tier([]), None);
    }

    #[test]
    fn pill_candidacy_expires_after_the_window() {
        let now = parse_timestamp("2026-10-22T12:00:00Z").unwrap();
        assert!(pill_candidate(
            Tier::AiUrgent,
            false,
            false,
            false,
            "2026-10-22T01:00:00Z",
            now,
            24
        ));
        assert!(!pill_candidate(
            Tier::Vip,
            false,
            false,
            false,
            "2026-10-21T11:00:00Z",
            now,
            24
        ));
        assert!(!pill_candidate(
            Tier::AiNormal,
            false,
            false,
            false,
            "2026-10-22T11:00:00Z",
            now,
            24
        ));
        assert!(!pill_candidate(
            Tier::Direct,
            false,
            false,
            true,
            "2026-10-22T11:00:00Z",
            now,
            24
        ));
    }
}
