//! Routing (07 §3.6) and the Inbox read models: the overlay's
//! `inbox.changed` payload (contract `InboxChanged`: Q3 urgent rows first,
//! then recent unread `ai-normal` up to the tab cap, never `low` or
//! `unscored`), the app Inbox list by thread with its sections (03 §4.3:
//! Urgent, Snoozed, Main, Low priority), filters (03 §4.4) and cursor, and
//! the thread view (03 §4.5). Twins render once (the primary row) with
//! every account attached (07 §7.2).

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Tier;
use crate::db::Result;
use crate::queries::{Clock, urgent_inbox};

/// Rows in the overlay's Inbox tab (OV-16 cap; the overlay shows `overlay.max_rows`).
pub const TAB_CAP: usize = 50;

/// Section of a tier (07 §3.6).
pub fn section_for(tier: Option<Tier>) -> &'static str {
    match tier {
        Some(t) if t.is_urgent() => "urgent",
        Some(Tier::Low) => "low",
        _ => "main",
    }
}

/// The provider word of the contract (`gmail` | `slack`).
pub fn provider_word(provider: &str) -> &'static str {
    if provider == "slack" {
        "slack"
    } else {
        "gmail"
    }
}

/// An overlay Inbox row (contract `InboxRow`, 02 §7.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxRow {
    pub id: String,
    pub provider: String,
    pub sender_name: String,
    pub sender_avatar_url: Option<String>,
    pub channel_name: Option<String>,
    pub mentioned: bool,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub received_at: String,
    pub urgency_tier: String,
    pub urgency_why: Option<String>,
    pub is_read: bool,
    /// The account's name when more than one account exists.
    pub account_label: Option<String>,
    /// Additive: the thread (for "open in app").
    pub thread_id: Option<String>,
}

/// `inbox.changed` (contract `InboxChanged`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxChanged {
    pub rows: Vec<InboxRow>,
    pub urgent_count: i64,
    /// Unread messages that are not in the overlay (the "n unread in app" link).
    pub unread_in_app: i64,
}

const NOT_SNOOZED: &str = "NOT EXISTS (SELECT 1 FROM snoozes z
      WHERE z.fired_at IS NULL AND z.cancelled_at IS NULL AND z.deleted_at IS NULL AND z.until > :now
        AND ((z.target_kind = 'message' AND z.target_id = m.id)
          OR (z.target_kind = 'thread' AND z.target_id = m.thread_id)))";

/// Not a later twin of another row (07 §7.2 primary row = earliest `fetched_at`).
const PRIMARY: &str = "NOT EXISTS (SELECT 1 FROM messages p WHERE p.dedup_key = m.dedup_key
      AND (p.fetched_at < m.fetched_at OR (p.fetched_at = m.fetched_at AND p.id < m.id)))";

fn account_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?)
}

fn mentioned(kind: &str, is_direct: bool) -> bool {
    is_direct || matches!(kind, "dm" | "group_dm")
}

fn row_from(conn: &Connection, id: &str, multi: bool) -> Result<Option<InboxRow>> {
    Ok(conn
        .query_row(
            "SELECT m.id, m.provider, m.sender_name, m.sender_avatar_url, m.channel_name, m.kind,
                    m.is_direct, m.subject, m.snippet, m.received_at, m.urgency_tier, m.urgency_why,
                    m.is_read, a.display_name, m.thread_id
               FROM messages m JOIN accounts a ON a.id = m.account_id WHERE m.id = ?1",
            [id],
            |r| {
                let provider: String = r.get(1)?;
                let kind: String = r.get(5)?;
                Ok(InboxRow {
                    id: r.get(0)?,
                    provider: provider_word(&provider).into(),
                    sender_name: r.get(2)?,
                    sender_avatar_url: r.get(3)?,
                    channel_name: r.get(4)?,
                    mentioned: mentioned(&kind, r.get(6)?),
                    subject: r.get(7)?,
                    snippet: Some(r.get(8)?),
                    received_at: r.get(9)?,
                    urgency_tier: r.get(10)?,
                    urgency_why: r.get(11)?,
                    is_read: r.get(12)?,
                    account_label: if multi { r.get(13)? } else { None },
                    thread_id: r.get(14)?,
                })
            },
        )
        .optional()?)
}

/// `inbox.changed`; `None` (pushed as `null`) when no account exists.
pub fn inbox_changed(conn: &Connection, clock: &Clock) -> Result<Option<InboxChanged>> {
    let n_accounts = account_count(conn)?;
    if n_accounts == 0 {
        return Ok(None);
    }
    let multi = n_accounts > 1;
    let mut rows = Vec::new();
    let mut seen_keys: Vec<String> = Vec::new();
    for u in urgent_inbox(conn, clock, 500)? {
        if seen_keys.contains(&u.dedup_key) {
            continue;
        }
        seen_keys.push(u.dedup_key.clone());
        if rows.len() < TAB_CAP
            && let Some(r) = row_from(conn, &u.id, multi)?
        {
            rows.push(r);
        }
    }
    let urgent_count = seen_keys.len() as i64;
    let sql = format!(
        "SELECT m.id, m.dedup_key FROM messages m
          WHERE m.is_read = 0 AND m.is_archived = 0 AND m.urgency_tier = 'ai-normal'
            AND {NOT_SNOOZED} AND {PRIMARY}
          ORDER BY m.received_at DESC LIMIT :limit"
    );
    let normal: Vec<(String, String)> = {
        let mut st = conn.prepare(&sql)?;
        let r = st.query_map(
            rusqlite::named_params! { ":now": clock.now, ":limit": TAB_CAP as i64 },
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        r.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, key) in normal {
        if rows.len() >= TAB_CAP {
            break;
        }
        if seen_keys.contains(&key) {
            continue;
        }
        seen_keys.push(key);
        if let Some(r) = row_from(conn, &id, multi)? {
            rows.push(r);
        }
    }
    let total_unread: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM messages m WHERE m.is_read = 0 AND m.is_archived = 0
               AND {NOT_SNOOZED} AND {PRIMARY}"
        ),
        rusqlite::named_params! { ":now": clock.now },
        |r| r.get(0),
    )?;
    Ok(Some(InboxChanged {
        unread_in_app: (total_unread - rows.len() as i64).max(0),
        urgent_count,
        rows,
    }))
}

// ------------------------------------------------------------- inbox list

/// 03 §4.4 filters plus the section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxFilter {
    /// `all` (default) | `urgent` | `snoozed` | `main` | `low`.
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    /// Tiers (`unscored` = "not yet scored").
    #[serde(default)]
    pub tiers: Option<Vec<String>>,
    #[serde(default)]
    pub contact_id: Option<String>,
    /// Free text over `messages_fts`.
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub show_archived: bool,
}

/// An account chip on a row (twins carry several).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountChip {
    pub account_id: String,
    pub label: String,
    pub color: Option<String>,
}

/// One Inbox row = one thread (03 §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxThreadRow {
    pub thread_id: String,
    pub kind: String,
    pub provider: String,
    pub title: String,
    pub channel_name: Option<String>,
    pub sender_name: Option<String>,
    pub sender_avatar_url: Option<String>,
    pub mentioned: bool,
    /// Email: the subject; Slack: the first line of the last snippet.
    pub line2: String,
    pub last_message_at: Option<String>,
    pub unread_count: i64,
    pub message_count: i64,
    pub urgency_tier: Option<String>,
    pub urgency_why: Option<String>,
    /// `urgent` | `snoozed` | `main` | `low`.
    pub section: String,
    pub snoozed_until: Option<String>,
    /// A time snooze returned it today (03 §4.2 "resurfaced").
    pub resurfaced: bool,
    pub accounts: Vec<AccountChip>,
    pub has_attachments: bool,
    /// A write-back is queued (the "pending" glyph).
    pub pending_writeback: bool,
    pub is_archived: bool,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionCounts {
    pub urgent: i64,
    pub snoozed: i64,
    pub main: i64,
    pub low: i64,
    pub unscored: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxListResult {
    pub rows: Vec<InboxThreadRow>,
    pub next_cursor: Option<String>,
    pub counts: SectionCounts,
}

/// The most threads one listing considers.
const LIST_SCAN: i64 = 2_000;

/// The Inbox list (03 §4.1–§4.4): urgent, snoozed, main, low, in that
/// order; `cursor` is an opaque offset.
pub fn inbox_list(
    conn: &Connection,
    clock: &Clock,
    day_start: &str,
    filter: &InboxFilter,
    cursor: Option<&str>,
    limit: Option<i64>,
) -> Result<InboxListResult> {
    let mut wh: Vec<String> = Vec::new();
    let mut args: Vec<(String, String)> = vec![(":now".into(), clock.now.clone())];
    if !filter.show_archived {
        wh.push("t.is_archived = 0".into());
    }
    if let Some(a) = &filter.account_id {
        wh.push("t.account_id = :account_id".into());
        args.push((":account_id".into(), a.clone()));
    }
    let mut msg_conds: Vec<String> = Vec::new();
    if let Some(tiers) = &filter.tiers {
        let list: Vec<String> = tiers
            .iter()
            .filter(|t| Tier::parse(t).is_some())
            .map(|t| format!("'{t}'"))
            .collect();
        if !list.is_empty() {
            msg_conds.push(format!("x.urgency_tier IN ({})", list.join(",")));
        }
    }
    if let Some(c) = &filter.contact_id {
        msg_conds.push("x.sender_contact_id = :contact_id".into());
        args.push((":contact_id".into(), c.clone()));
    }
    if let Some(s) = &filter.source_id {
        msg_conds.push("x.source_id = :source_id".into());
        args.push((":source_id".into(), s.clone()));
    }
    if filter.unread_only {
        msg_conds.push("x.is_read = 0".into());
    }
    if let Some(f) = &filter.from {
        msg_conds.push("x.received_at >= :from".into());
        args.push((":from".into(), f.clone()));
    }
    if let Some(t) = &filter.to {
        msg_conds.push("x.received_at < :to".into());
        args.push((":to".into(), t.clone()));
    }
    if let Some(topic) = filter.topic.as_deref().filter(|t| !t.trim().is_empty()) {
        match crate::queries::build_fts(topic, crate::queries::fts::Fts::Messages, true)
            .and_then(|q| q.match_expr)
        {
            Some(expr) => {
                msg_conds.push(
                    "x.seq IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH :topic)"
                        .into(),
                );
                args.push((":topic".into(), expr));
            }
            None => msg_conds.push("0".into()),
        }
    }
    if !msg_conds.is_empty() {
        wh.push(format!(
            "EXISTS (SELECT 1 FROM messages x WHERE x.thread_id = t.id AND {})",
            msg_conds.join(" AND ")
        ));
    }
    let where_sql = if wh.is_empty() {
        "1".to_string()
    } else {
        wh.join(" AND ")
    };
    let sql = format!(
        "SELECT t.id FROM threads t WHERE {where_sql}
          ORDER BY t.last_message_at DESC LIMIT {LIST_SCAN}"
    );
    let ids: Vec<String> = {
        let mut st = conn.prepare(&sql)?;
        let named: Vec<(&str, &dyn rusqlite::ToSql)> = args
            .iter()
            .map(|(k, v)| (k.as_str(), v as &dyn rusqlite::ToSql))
            .collect();
        let used: Vec<(&str, &dyn rusqlite::ToSql)> = named
            .into_iter()
            .filter(|(k, _)| sql.contains(*k))
            .collect();
        let r = st.query_map(used.as_slice(), |r| r.get::<_, String>(0))?;
        r.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let multi = account_count(conn)? > 1;
    let mut buckets: [Vec<InboxThreadRow>; 4] = Default::default();
    let mut counts = SectionCounts::default();
    for id in ids {
        let Some(row) = thread_row(conn, clock, day_start, &id, multi)? else {
            continue;
        };
        let idx = match row.section.as_str() {
            "urgent" => 0,
            "snoozed" => 1,
            "main" => 2,
            _ => 3,
        };
        match idx {
            0 => counts.urgent += 1,
            1 => counts.snoozed += 1,
            2 => counts.main += 1,
            _ => counts.low += 1,
        }
        if row.urgency_tier.as_deref() == Some("unscored") {
            counts.unscored += 1;
        }
        buckets[idx].push(row);
    }
    // 07 §3.5: tier rank first, then newest (the scan is newest first and
    // the sort is stable).
    for b in buckets.iter_mut() {
        b.sort_by_key(|r| {
            r.urgency_tier
                .as_deref()
                .and_then(Tier::parse)
                .map_or(Tier::AiNormal.rank(), Tier::rank)
        });
    }
    let wanted = filter.section.as_deref().unwrap_or("all");
    let all: Vec<InboxThreadRow> = match wanted {
        "urgent" => std::mem::take(&mut buckets[0]),
        "snoozed" => std::mem::take(&mut buckets[1]),
        "main" => std::mem::take(&mut buckets[2]),
        "low" => std::mem::take(&mut buckets[3]),
        _ => buckets.into_iter().flatten().collect(),
    };
    let offset: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
    let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
    let page: Vec<InboxThreadRow> = all.iter().skip(offset).take(limit).cloned().collect();
    let next = (offset + page.len() < all.len()).then(|| (offset + page.len()).to_string());
    Ok(InboxListResult {
        rows: page,
        next_cursor: next,
        counts,
    })
}

/// `threads` columns read by [`thread_row`].
type ThreadCols = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<String>,
    Option<String>,
    bool,
    Option<String>,
    String,
);
/// The latest message of a thread: id, sender, avatar, direct, key, why, attachments.
type LatestCols = (
    String,
    String,
    Option<String>,
    bool,
    String,
    Option<String>,
    bool,
);
/// The thread view's head: thread columns and the account chip.
type HeadCols = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    i64,
    Option<String>,
    String,
    String,
    Option<String>,
);

/// One thread as an Inbox row, or `None` when it is a later twin of
/// another account's thread. `day_start` is the local day's start (UTC):
/// a snooze fired by time since then shows the "resurfaced" marker.
pub fn thread_row(
    conn: &Connection,
    clock: &Clock,
    day_start: &str,
    thread_id: &str,
    multi: bool,
) -> Result<Option<InboxThreadRow>> {
    let t: Option<ThreadCols> = conn
        .query_row(
            "SELECT t.kind, t.provider, t.title, t.channel_name, t.last_message_at, t.unread_count,
                    t.message_count, t.urgency_tier, t.last_snippet, t.is_archived, t.source_url, t.account_id
               FROM threads t WHERE t.id = ?1",
            [thread_id],
            |r| {
                Ok((
                    r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                    r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?,
                ))
            },
        )
        .optional()?;
    let Some((
        kind,
        provider,
        title,
        channel,
        last_at,
        unread,
        count,
        tier,
        last_snip,
        archived,
        src,
        account,
    )) = t
    else {
        return Ok(None);
    };
    // The latest message: sender, why, twins.
    let latest: Option<LatestCols> = conn
        .query_row(
            "SELECT m.id, m.sender_name, m.sender_avatar_url, m.is_direct, m.dedup_key, m.urgency_why,
                    m.has_attachments
               FROM messages m WHERE m.thread_id = ?1 ORDER BY m.received_at DESC LIMIT 1",
            [thread_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()?;
    let mut accounts: Vec<AccountChip> = Vec::new();
    let chip = |id: &str| -> Result<Option<AccountChip>> {
        Ok(conn
            .query_row(
                "SELECT id, display_name, color FROM accounts WHERE id = ?1",
                [id],
                |r| {
                    Ok(AccountChip {
                        account_id: r.get(0)?,
                        label: r.get(1)?,
                        color: r.get(2)?,
                    })
                },
            )
            .optional()?)
    };
    if multi && let Some(c) = chip(&account)? {
        accounts.push(c);
    }
    let mut why = None;
    let mut sender = (None, None, false, false);
    if let Some((mid, name, avatar, direct, key, w, att)) = &latest {
        let twins = super::dedup::twins(conn, key)?;
        if twins.first().is_some_and(|p| p.id != *mid) {
            return Ok(None);
        }
        for tw in twins.iter().skip(1) {
            if !accounts.iter().any(|a| a.account_id == tw.account_id)
                && let Some(c) = chip(&tw.account_id)?
            {
                accounts.push(c);
            }
        }
        why = w.clone();
        sender = (
            Some(name.clone()),
            avatar.clone(),
            mentioned(&kind, *direct),
            *att,
        );
    }
    // The why for the thread's tier: the latest message carrying it (07 §3.5).
    if let Some(tr) = &tier {
        let w: Option<String> = conn
            .query_row(
                "SELECT urgency_why FROM messages WHERE thread_id = ?1 AND urgency_tier = ?2
                  AND is_read = 0 ORDER BY received_at DESC LIMIT 1",
                params![thread_id, tr],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if w.is_some() {
            why = w;
        }
    }
    let snoozed_until: Option<String> = conn
        .query_row(
            "SELECT MAX(z.until) FROM snoozes z
              WHERE z.fired_at IS NULL AND z.cancelled_at IS NULL AND z.deleted_at IS NULL AND z.until > ?2
                AND ((z.target_kind = 'thread' AND z.target_id = ?1)
                  OR (z.target_kind = 'message' AND z.target_id IN (SELECT id FROM messages WHERE thread_id = ?1)))",
            params![thread_id, clock.now],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM connector_writebacks w WHERE (w.target_kind = 'thread' AND w.target_id = ?1)
            OR (w.target_kind = 'message' AND w.target_id IN (SELECT id FROM messages WHERE thread_id = ?1))",
        [thread_id],
        |r| r.get(0),
    )?;
    let urgent_unread: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND is_read = 0 AND is_archived = 0
            AND urgency_tier IN ('vip','direct','ai-urgent')",
        [thread_id],
        |r| r.get(0),
    )?;
    let resurfaced = snoozed_until.is_none()
        && conn.query_row(
            "SELECT COUNT(*) FROM snoozes z
              WHERE z.fired_at IS NOT NULL AND z.fired_at >= ?2 AND z.deleted_at IS NULL
                AND ((z.target_kind = 'thread' AND z.target_id = ?1)
                  OR (z.target_kind = 'message' AND z.target_id IN (SELECT id FROM messages WHERE thread_id = ?1)))",
            params![thread_id, day_start],
            |r| r.get::<_, i64>(0),
        )? > 0;
    let section = if snoozed_until.is_some() {
        "snoozed"
    } else if urgent_unread > 0 {
        "urgent"
    } else if tier.as_deref() == Some("low") {
        "low"
    } else {
        "main"
    };
    let line2 = if kind == "email" {
        title.clone()
    } else {
        last_snip
            .as_deref()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    };
    Ok(Some(InboxThreadRow {
        thread_id: thread_id.to_string(),
        provider: provider_word(&provider).into(),
        kind,
        title,
        channel_name: channel,
        sender_name: sender.0,
        sender_avatar_url: sender.1,
        mentioned: sender.2,
        line2,
        last_message_at: last_at,
        unread_count: unread,
        message_count: count,
        urgency_tier: tier,
        urgency_why: why,
        section: section.into(),
        snoozed_until,
        resurfaced,
        accounts,
        has_attachments: sender.3,
        pending_writeback: pending > 0,
        is_archived: archived,
        source_url: src,
    }))
}

// ------------------------------------------------------------ thread view

/// One message in the thread view (03 §4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessageRow {
    pub id: String,
    pub sender_name: String,
    pub sender_address: String,
    pub sender_avatar_url: Option<String>,
    pub sender_contact_id: Option<String>,
    pub vip: bool,
    pub received_at: String,
    pub subject: Option<String>,
    pub snippet: String,
    pub body_text: Option<String>,
    pub body_truncated: bool,
    pub has_attachments: bool,
    pub is_read: bool,
    pub is_archived: bool,
    pub urgency_tier: String,
    /// `urgent` | `normal` | `low`.
    pub urgency_level: String,
    pub urgency_why: Option<String>,
    pub urgency_source: Option<String>,
    pub source_url: String,
    /// `not synced to Gmail / Slack` after a dropped write-back.
    pub not_synced: bool,
    pub snoozed_until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadDetail {
    pub id: String,
    pub kind: String,
    pub provider: String,
    pub title: String,
    pub channel_name: Option<String>,
    pub participants: Value,
    pub account: AccountChip,
    pub source_url: Option<String>,
    pub message_count: i64,
    pub urgency_tier: Option<String>,
    pub messages: Vec<ThreadMessageRow>,
    /// Sessions, notes and messages the thread is about (03 §4.1;
    /// `crate::links`): explicit links, then mentions.
    pub links: Vec<crate::links::ItemLink>,
    /// Actions that stay in Mode for this account (capabilities, 06 §2.2).
    pub app_local_actions: Vec<String>,
}

/// The thread view; `None` when the thread is gone.
pub fn thread_detail(
    conn: &Connection,
    clock: &Clock,
    thread_id: &str,
) -> Result<Option<ThreadDetail>> {
    let head: Option<HeadCols> = conn
        .query_row(
            "SELECT t.kind, t.provider, t.title, t.channel_name, t.participants_json, t.source_url,
                    t.message_count, t.urgency_tier, a.id, a.display_name, a.color
               FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?1",
            [thread_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            },
        )
        .optional()?;
    let Some((
        kind,
        provider,
        title,
        channel,
        parts,
        src,
        count,
        tier,
        acc_id,
        acc_label,
        acc_color,
    )) = head
    else {
        return Ok(None);
    };
    let mut st = conn.prepare(
        "SELECT m.id, m.sender_name, m.sender_address, m.sender_avatar_url, m.sender_contact_id,
                COALESCE(c.vip, 0), m.received_at, m.subject, m.snippet, m.body_text, m.body_truncated,
                m.has_attachments, m.is_read, m.is_archived, m.urgency_tier, m.urgency_why,
                m.urgency_source, m.source_url, m.labels_json,
                (SELECT MAX(z.until) FROM snoozes z WHERE z.target_kind = 'message' AND z.target_id = m.id
                    AND z.fired_at IS NULL AND z.cancelled_at IS NULL AND z.deleted_at IS NULL AND z.until > ?2)
           FROM messages m LEFT JOIN contacts c ON c.id = m.sender_contact_id
          WHERE m.thread_id = ?1 ORDER BY m.received_at",
    )?;
    let msgs = st
        .query_map(params![thread_id, clock.now], |r| {
            let tier: String = r.get(14)?;
            let labels: String = r.get(18)?;
            Ok(ThreadMessageRow {
                id: r.get(0)?,
                sender_name: r.get(1)?,
                sender_address: r.get(2)?,
                sender_avatar_url: r.get(3)?,
                sender_contact_id: r.get(4)?,
                vip: r.get(5)?,
                received_at: r.get(6)?,
                subject: r.get(7)?,
                snippet: r.get(8)?,
                body_text: r.get(9)?,
                body_truncated: r.get(10)?,
                has_attachments: r.get(11)?,
                is_read: r.get(12)?,
                is_archived: r.get(13)?,
                urgency_level: Tier::parse(&tier)
                    .map(|t| t.level())
                    .unwrap_or("normal")
                    .into(),
                urgency_tier: tier,
                urgency_why: r.get(15)?,
                urgency_source: r.get(16)?,
                source_url: r.get(17)?,
                not_synced: labels.contains(crate::connectors::writeback::NOT_SYNCED_LABEL),
                snoozed_until: r.get(19)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let account_row = crate::repo::get_by_id::<crate::repo::Account>(conn, &acc_id)?;
    let app_local = account_row
        .map(|a| {
            let caps = crate::connectors::capabilities(&a);
            let mut v = Vec::new();
            for act in [
                crate::connectors::TriageAction::MarkRead,
                crate::connectors::TriageAction::MarkUnread,
                crate::connectors::TriageAction::Archive,
            ] {
                if !caps.supports(act) {
                    v.push(act.as_str().to_string());
                }
            }
            v.push("snooze".into());
            v
        })
        .unwrap_or_default();
    Ok(Some(ThreadDetail {
        id: thread_id.into(),
        provider: provider_word(&provider).into(),
        kind,
        title,
        channel_name: channel,
        participants: serde_json::from_str(&parts).unwrap_or(Value::Array(vec![])),
        account: AccountChip {
            account_id: acc_id,
            label: acc_label,
            color: acc_color,
        },
        source_url: src,
        message_count: count,
        urgency_tier: tier,
        messages: msgs,
        links: crate::links::thread_links(conn, thread_id)?,
        app_local_actions: app_local,
    }))
}
