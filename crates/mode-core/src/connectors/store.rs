//! Storage mapping shared by the adapters (09 §5.3–§5.4): threads are
//! upserted by `(account_id, external_id)`, messages by `(account_id,
//! external_id)`, senders resolve to contacts through `contact_identities`
//! (created lazily), every new row starts `unscored` and goes through the
//! urgency engine's ingest stage in the same transaction (07 §3.1–§3.3).
//! Re-fetches keep their tier; provider read / archive state follows the
//! write-back conflict rule of 06 §2.5. Local triage (mark read / unread,
//! archive / unarchive) is applied here first and queued for write-back
//! when the account's capabilities allow it (06 §2.5, §6.8, §8.8).

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use super::writeback;
use super::{Provider, TriageAction};
use crate::db::{DbError, Result, format_timestamp};
use crate::repo::{self, CalendarEvent, Message, Thread};
use crate::urgency::{self, IngestOutcome, vips};

/// A thread as the provider describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewThread {
    pub external_id: String,
    /// `email` | `dm` | `group_dm` | `channel` | `channel_thread`.
    pub kind: String,
    pub title: String,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub participants: Value,
    pub source_url: Option<String>,
}

/// The sender and the identity it resolves through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sender {
    pub name: String,
    /// Lowercase email, or the Slack user id.
    pub address: String,
    pub avatar_url: Option<String>,
    /// `email` | `slack_user`.
    pub identity_kind: String,
    /// `''` for email, the Slack `team_id`.
    pub identity_scope: String,
}

/// A message as the provider describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMessage {
    pub account_id: String,
    pub source_id: Option<String>,
    pub provider: Provider,
    pub external_id: String,
    pub rfc_message_id: Option<String>,
    pub dedup_key: String,
    pub kind: String,
    pub thread: NewThread,
    pub sender: Sender,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub subject: Option<String>,
    pub snippet: String,
    pub body_text: Option<String>,
    pub body_truncated: bool,
    pub has_attachments: bool,
    pub labels: Vec<String>,
    pub is_direct: bool,
    /// Provider-side read state (`None` = not reported).
    pub is_read: Option<bool>,
    /// Provider-side archive state.
    pub is_archived: Option<bool>,
    /// The provider reported a change (Gmail history label event, Slack
    /// `last_read` moving), not just a snapshot: the provider then wins over
    /// a queued opposite write-back (06 §2.5 "Conflict").
    pub state_changed: bool,
    pub received_at: String,
    pub source_url: String,
}

/// What [`ingest`] did.
#[derive(Debug, Clone, Default)]
pub struct IngestReport {
    pub inserted: Vec<String>,
    pub updated: usize,
    pub thread_ids: Vec<String>,
    pub urgency: IngestOutcome,
}

fn upsert_thread(
    conn: &Connection,
    account_id: &str,
    provider: Provider,
    t: &NewThread,
    now: &str,
) -> Result<String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM threads WHERE account_id = ?1 AND external_id = ?2",
            params![account_id, t.external_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        conn.execute(
            "UPDATE threads SET channel_name = COALESCE(?2, channel_name),
                    participants_json = CASE WHEN ?3 = '[]' THEN participants_json ELSE ?3 END,
                    source_url = COALESCE(source_url, ?4), updated_at = ?5
              WHERE id = ?1",
            params![
                id,
                t.channel_name,
                t.participants.to_string(),
                t.source_url,
                now
            ],
        )?;
        return Ok(id);
    }
    let row = Thread {
        id: repo::new_id(),
        account_id: account_id.into(),
        provider: provider.as_str().into(),
        external_id: t.external_id.clone(),
        kind: t.kind.clone(),
        title: if t.title.trim().is_empty() {
            "(no subject)".into()
        } else {
            t.title.clone()
        },
        channel_id: t.channel_id.clone(),
        channel_name: t.channel_name.clone(),
        participants_json: t.participants.to_string(),
        source_url: t.source_url.clone(),
        created_at: now.into(),
        updated_at: now.into(),
        ..Default::default()
    };
    repo::insert(conn, &row)?;
    Ok(row.id)
}

/// Apply provider read / archive state to a stored message with the
/// conflict rule of 06 §2.5. Returns whether anything changed.
pub fn apply_provider_state(
    conn: &Connection,
    message_id: &str,
    is_read: Option<bool>,
    is_archived: Option<bool>,
    changed: bool,
    now: &str,
) -> Result<bool> {
    let cur: Option<(bool, bool)> = conn
        .query_row(
            "SELECT is_read, is_archived FROM messages WHERE id = ?1",
            [message_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((cur_read, cur_arch)) = cur else {
        return Ok(false);
    };
    let mut touched = false;
    let field = |state: Option<bool>, cur: bool, on: TriageAction| -> Result<Option<bool>> {
        let Some(s) = state else { return Ok(None) };
        let wants_s = if s { on } else { on.opposite() };
        // A queued action that the provider already reflects is done.
        if writeback::drop_matching(conn, "message", message_id, wants_s)? > 0 {
            return Ok((cur != s).then_some(s));
        }
        // A queued opposite action: keep the local state unless the provider
        // reported a change (then the provider wins and the queue drops it).
        if writeback::pending(conn, "message", message_id, wants_s.opposite())? {
            if changed {
                writeback::drop_matching(conn, "message", message_id, wants_s.opposite())?;
                return Ok((cur != s).then_some(s));
            }
            return Ok(None);
        }
        Ok((cur != s).then_some(s))
    };
    if let Some(r) = field(is_read, cur_read, TriageAction::MarkRead)? {
        conn.execute(
            "UPDATE messages SET is_read = ?2, read_at = CASE WHEN ?2 THEN ?3 ELSE NULL END,
                    read_origin = 'provider', updated_at = ?3 WHERE id = ?1",
            params![message_id, r, now],
        )?;
        touched = true;
    }
    if let Some(a) = field(is_archived, cur_arch, TriageAction::Archive)? {
        conn.execute(
            "UPDATE messages SET is_archived = ?2, archived_at = CASE WHEN ?2 THEN ?3 ELSE NULL END,
                    updated_at = ?3 WHERE id = ?1",
            params![message_id, a, now],
        )?;
        touched = true;
    }
    Ok(touched)
}

/// Upsert a batch, then run the urgency ingest stage on the new rows, all
/// inside the caller's transaction.
pub fn ingest(
    conn: &Connection,
    device_id: &str,
    msgs: &[NewMessage],
    ctx: &urgency::Ctx,
    body_days: i64,
) -> Result<IngestReport> {
    let now = ctx.now_str();
    let retained = format_timestamp(ctx.now + time::Duration::days(body_days.max(0)));
    let mut rep = IngestReport::default();
    for m in msgs {
        let thread_id = upsert_thread(conn, &m.account_id, m.provider, &m.thread, &now)?;
        if !rep.thread_ids.contains(&thread_id) {
            rep.thread_ids.push(thread_id.clone());
        }
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM messages WHERE account_id = ?1 AND external_id = ?2",
                params![m.account_id, m.external_id],
                |r| r.get(0),
            )
            .optional()?;
        let labels = serde_json::to_string(&m.labels).unwrap_or_else(|_| "[]".into());
        if let Some(id) = existing {
            // Edits overwrite text; the tier stays (07 §3.1).
            conn.execute(
                "UPDATE messages SET labels_json = ?2, snippet = ?3,
                        body_text = COALESCE(?4, body_text),
                        body_truncated = CASE WHEN ?4 IS NULL THEN body_truncated ELSE ?5 END,
                        body_retained_until = CASE WHEN ?4 IS NULL THEN body_retained_until ELSE ?6 END,
                        subject = COALESCE(?7, subject), has_attachments = ?8,
                        sender_avatar_url = COALESCE(?9, sender_avatar_url), updated_at = ?10
                  WHERE id = ?1",
                params![
                    id,
                    labels,
                    m.snippet,
                    m.body_text,
                    m.body_truncated,
                    retained,
                    m.subject,
                    m.has_attachments,
                    m.sender.avatar_url,
                    now
                ],
            )?;
            apply_provider_state(conn, &id, m.is_read, m.is_archived, m.state_changed, &now)?;
            rep.updated += 1;
            continue;
        }
        let contact = vips::resolve_or_create(
            conn,
            device_id,
            &m.sender.identity_kind,
            &m.sender.identity_scope,
            &m.sender.address,
            &m.sender.name,
            m.sender.avatar_url.as_deref(),
        )
        .ok();
        if let Some(c) = &contact {
            vips::count_message(conn, c, &m.received_at)?;
        }
        let is_read = m.is_read.unwrap_or(false);
        let is_archived = m.is_archived.unwrap_or(false);
        let row = Message {
            seq: None,
            id: repo::new_id(),
            account_id: m.account_id.clone(),
            thread_id: Some(thread_id),
            source_id: m.source_id.clone(),
            provider: m.provider.as_str().into(),
            external_id: m.external_id.clone(),
            rfc_message_id: m.rfc_message_id.clone(),
            dedup_key: m.dedup_key.clone(),
            kind: m.kind.clone(),
            sender_contact_id: contact,
            sender_name: if m.sender.name.trim().is_empty() {
                m.sender.address.clone()
            } else {
                m.sender.name.clone()
            },
            sender_address: m.sender.address.clone(),
            sender_avatar_url: m.sender.avatar_url.clone(),
            channel_id: m.channel_id.clone(),
            channel_name: m.channel_name.clone(),
            subject: m.subject.clone(),
            snippet: m.snippet.clone(),
            body_retained_until: m.body_text.as_ref().map(|_| retained.clone()),
            body_text: m.body_text.clone(),
            body_truncated: m.body_truncated,
            has_attachments: m.has_attachments,
            labels_json: labels,
            is_direct: m.is_direct,
            is_read,
            read_at: is_read.then(|| now.clone()),
            read_origin: is_read.then(|| "provider".to_string()),
            is_archived,
            archived_at: is_archived.then(|| now.clone()),
            urgency_tier: "unscored".into(),
            urgency_why: None,
            urgency_source: None,
            urgency_model: None,
            scored_at: None,
            source_url: m.source_url.clone(),
            received_at: m.received_at.clone(),
            fetched_at: now.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        repo::insert(conn, &row)?;
        rep.inserted.push(row.id);
    }
    rep.urgency = urgency::process_new(conn, device_id, &rep.inserted, ctx)?;
    urgency::rank::recompute_threads(conn, &rep.thread_ids, &now)?;
    Ok(rep)
}

/// Delete a message the provider deleted (Gmail `messageDeleted`).
pub fn delete_message(
    conn: &Connection,
    account_id: &str,
    external_id: &str,
) -> Result<Option<String>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT id, thread_id FROM messages WHERE account_id = ?1 AND external_id = ?2",
            params![account_id, external_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((id, thread)) = row else {
        return Ok(None);
    };
    conn.execute("DELETE FROM messages WHERE id = ?1", [&id])?;
    if let Some(t) = &thread {
        urgency::rank::recompute_threads(conn, std::slice::from_ref(t), &crate::db::now_rfc3339())?;
    }
    Ok(Some(id))
}

/// The id of a stored message by provider id.
pub fn message_id(
    conn: &Connection,
    account_id: &str,
    external_id: &str,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM messages WHERE account_id = ?1 AND external_id = ?2",
            params![account_id, external_id],
            |r| r.get(0),
        )
        .optional()?)
}

// ------------------------------------------------------------------ triage

/// What a local triage action did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageResult {
    /// Messages changed locally (twins included).
    pub messages: Vec<String>,
    /// Write-backs queued.
    pub queued: usize,
    /// The action stayed in Mode for at least one account (the detail
    /// panel's "marked read in Mode only").
    pub app_local: bool,
    pub threads: Vec<String>,
    /// Snoozes canceled by an archive (07 §6.1).
    pub snoozes_cancelled: usize,
}

struct Target {
    id: String,
    account_id: String,
    provider: String,
    external_id: String,
    thread_id: Option<String>,
    thread_external: Option<String>,
    channel_id: Option<String>,
    received_at: String,
}

fn targets(conn: &Connection, target_kind: &str, target_id: &str) -> Result<Vec<Target>> {
    let ids: Vec<String> = match target_kind {
        "message" => crate::urgency::dedup::group_of(conn, target_id)?,
        "thread" => {
            let mut st = conn.prepare("SELECT id FROM messages WHERE thread_id = ?1")?;
            let mut v: Vec<String> = st
                .query_map([target_id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // the twins of the thread's messages in other accounts
            let mut extra = Vec::new();
            for id in &v {
                for t in crate::urgency::dedup::group_of(conn, id)? {
                    if !v.contains(&t) && !extra.contains(&t) {
                        extra.push(t);
                    }
                }
            }
            v.extend(extra);
            v
        }
        other => {
            return Err(DbError::Invalid {
                what: "triage target",
                detail: format!("{other} is not a message or a thread"),
            });
        }
    };
    let mut out = Vec::new();
    for id in ids {
        let t = conn
            .query_row(
                "SELECT m.id, m.account_id, m.provider, m.external_id, m.thread_id, t.external_id,
                        m.channel_id, m.received_at
                   FROM messages m LEFT JOIN threads t ON t.id = m.thread_id WHERE m.id = ?1",
                [&id],
                |r| {
                    Ok(Target {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        provider: r.get(2)?,
                        external_id: r.get(3)?,
                        thread_id: r.get(4)?,
                        thread_external: r.get(5)?,
                        channel_id: r.get(6)?,
                        received_at: r.get(7)?,
                    })
                },
            )
            .optional()?;
        out.extend(t);
    }
    if out.is_empty() {
        return Err(DbError::NotFound {
            table: if target_kind == "thread" {
                "threads"
            } else {
                "messages"
            },
            key: target_id.into(),
        });
    }
    Ok(out)
}

/// Apply a triage action locally first, then queue write-backs per account
/// capability (06 §2.5): Gmail with `gmail.modify` (thread scope through
/// `threads.modify`), Slack mark read only for the newest message of a
/// conversation (Slack keeps one read cursor), everything else app-local.
pub fn triage(
    conn: &Connection,
    device_id: &str,
    target_kind: &str,
    target_id: &str,
    action: TriageAction,
    now: OffsetDateTime,
) -> Result<TriageResult> {
    let now_s = format_timestamp(now);
    let ts = targets(conn, target_kind, target_id)?;
    let mut res = TriageResult::default();
    for t in &ts {
        let n = match action {
            TriageAction::MarkRead => conn.execute(
                "UPDATE messages SET is_read = 1, read_at = ?2, read_origin = 'mode', updated_at = ?2
                  WHERE id = ?1 AND is_read = 0",
                params![t.id, now_s],
            )?,
            TriageAction::MarkUnread => conn.execute(
                "UPDATE messages SET is_read = 0, read_at = NULL, read_origin = 'mode', updated_at = ?2
                  WHERE id = ?1 AND is_read = 1",
                params![t.id, now_s],
            )?,
            TriageAction::Archive => conn.execute(
                "UPDATE messages SET is_archived = 1, archived_at = ?2, updated_at = ?2
                  WHERE id = ?1 AND is_archived = 0",
                params![t.id, now_s],
            )?,
            TriageAction::Unarchive => conn.execute(
                "UPDATE messages SET is_archived = 0, archived_at = NULL, updated_at = ?2
                  WHERE id = ?1 AND is_archived = 1",
                params![t.id, now_s],
            )?,
        };
        if n > 0 {
            res.messages.push(t.id.clone());
        }
        if let Some(th) = &t.thread_id
            && !res.threads.contains(th)
        {
            res.threads.push(th.clone());
        }
    }
    if action == TriageAction::Archive {
        res.snoozes_cancelled +=
            crate::urgency::snooze::cancel(conn, device_id, target_kind, target_id, now)?;
    }
    // Write-backs: one per account (thread scope for Gmail threads).
    let mut accounts_done: Vec<String> = Vec::new();
    for t in &ts {
        if accounts_done.contains(&t.account_id) && target_kind == "thread" {
            continue;
        }
        let Some(account) = repo::get_by_id::<repo::Account>(conn, &t.account_id)? else {
            continue;
        };
        let caps = super::capabilities(&account);
        if !caps.supports(action) {
            res.app_local = true;
            continue;
        }
        let (kind, tid, ext) = if t.provider == "google" && target_kind == "thread" {
            accounts_done.push(t.account_id.clone());
            (
                "thread",
                t.thread_id.clone().unwrap_or_default(),
                t.thread_external.clone().unwrap_or_default(),
            )
        } else if t.provider == "slack" {
            // Mark read moves the conversation's cursor: only the newest
            // message of the conversation may be written back.
            let newest: Option<String> = conn
                .query_row(
                    "SELECT id FROM messages WHERE account_id = ?1 AND channel_id IS ?2
                      ORDER BY received_at DESC LIMIT 1",
                    params![t.account_id, t.channel_id],
                    |r| r.get(0),
                )
                .optional()?;
            let is_newest = newest.as_deref() == Some(t.id.as_str())
                || conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND channel_id IS ?2
                           AND received_at > ?3 AND is_read = 0",
                    params![t.account_id, t.channel_id, t.received_at],
                    |r| r.get::<_, i64>(0),
                )? == 0;
            if target_kind == "thread" {
                if accounts_done.contains(&t.account_id) {
                    continue;
                }
                // the newest message of the thread stands for it
                let newest_in_thread: Option<String> = conn
                    .query_row(
                        "SELECT id FROM messages WHERE thread_id IS ?1 ORDER BY received_at DESC LIMIT 1",
                        params![t.thread_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if newest_in_thread.as_deref() != Some(t.id.as_str()) {
                    continue;
                }
                accounts_done.push(t.account_id.clone());
            }
            if !is_newest {
                res.app_local = true;
                continue;
            }
            ("message", t.id.clone(), t.external_id.clone())
        } else {
            ("message", t.id.clone(), t.external_id.clone())
        };
        if tid.is_empty() || ext.is_empty() {
            continue;
        }
        if let writeback::Enqueued::Queued(_) =
            writeback::enqueue(conn, &t.account_id, kind, &tid, action, &ext, now)?
        {
            res.queued += 1;
        }
    }
    urgency::rank::recompute_threads(conn, &res.threads, &now_s)?;
    Ok(res)
}

// --------------------------------------------------------------- calendar

/// An event as the provider describes it (06 §7.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewEvent {
    pub account_id: String,
    pub calendar_id: String,
    pub calendar_name: Option<String>,
    pub external_id: String,
    pub recurring_event_id: Option<String>,
    pub ical_uid: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub timezone: Option<String>,
    pub status: String,
    pub my_response: Option<String>,
    pub organizer: Option<String>,
    pub attendees: Value,
    pub join_url: Option<String>,
    pub conference: Option<String>,
    pub html_link: Option<String>,
    pub etag: Option<String>,
}

/// Upsert by `(account_id, calendar_id, external_id)`; an unchanged `etag`
/// is skipped. Returns whether the row changed.
pub fn upsert_event(conn: &Connection, e: &NewEvent, now: &str) -> Result<bool> {
    let cur: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT id, etag FROM calendar_events WHERE account_id = ?1 AND calendar_id = ?2 AND external_id = ?3",
            params![e.account_id, e.calendar_id, e.external_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((_, etag)) = &cur
        && etag.is_some()
        && *etag == e.etag
    {
        conn.execute(
            "UPDATE calendar_events SET synced_at = ?4 WHERE account_id = ?1 AND calendar_id = ?2 AND external_id = ?3",
            params![e.account_id, e.calendar_id, e.external_id, now],
        )?;
        return Ok(false);
    }
    let row = CalendarEvent {
        id: cur
            .as_ref()
            .map(|c| c.0.clone())
            .unwrap_or_else(repo::new_id),
        account_id: e.account_id.clone(),
        calendar_id: e.calendar_id.clone(),
        calendar_name: e.calendar_name.clone(),
        external_id: e.external_id.clone(),
        recurring_event_id: e.recurring_event_id.clone(),
        ical_uid: e.ical_uid.clone(),
        title: if e.title.trim().is_empty() {
            "(no title)".into()
        } else {
            e.title.clone()
        },
        description: e.description.clone(),
        location: e.location.clone(),
        starts_at: e.starts_at.clone(),
        ends_at: e.ends_at.clone(),
        all_day: e.all_day,
        start_date: e.start_date.clone(),
        end_date: e.end_date.clone(),
        timezone: e.timezone.clone(),
        status: e.status.clone(),
        my_response: e.my_response.clone(),
        organizer: e.organizer.clone(),
        attendees_json: e.attendees.to_string(),
        join_url: e.join_url.clone(),
        conference: e.conference.clone(),
        html_link: e.html_link.clone(),
        etag: e.etag.clone(),
        synced_at: now.into(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    if cur.is_some() {
        let created: String = conn.query_row(
            "SELECT created_at FROM calendar_events WHERE id = ?1",
            [&row.id],
            |r| r.get(0),
        )?;
        let row = CalendarEvent {
            created_at: created,
            ..row
        };
        repo::replace(conn, &row)?;
    } else {
        repo::insert(conn, &row)?;
    }
    Ok(true)
}

/// Delete events of an account outside the −30 / +90 day window (06 §7.2).
pub fn prune_events(conn: &Connection, account_id: &str, now: OffsetDateTime) -> Result<usize> {
    let lo = format_timestamp(now - time::Duration::days(30));
    let hi = format_timestamp(now + time::Duration::days(90));
    Ok(conn.execute(
        "DELETE FROM calendar_events WHERE account_id = ?1 AND (ends_at < ?2 OR starts_at > ?3)",
        params![account_id, lo, hi],
    )?)
}
