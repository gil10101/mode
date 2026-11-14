//! Important people (07 §5): the contacts with `vip = 1` (one person across emails and
//! Slack workspaces through `contact_identities`) and important channels
//! (`account_sources.vip`). Marking re-tiers every message of the contact in
//! the retention window to `vip` in one pass (no model call, no alert);
//! unmarking re-runs the deterministic stage and re-queues the last 24 h,
//! older rows become `ai-normal` with the why `No longer important` (§3.5).
//!
//! Also the contact resolution the connectors use on ingest: an identity
//! `(kind, scope, value)` resolves to its contact, created lazily.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::{Ctx, TierSource, why};
use crate::db::{DbError, Result, format_timestamp};
use crate::repo::{self, Contact, ContactIdentity};

/// One identity of a contact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRow {
    /// `email` | `slack_user`.
    pub kind: String,
    pub value: String,
    /// `''` for email, the Slack `team_id`.
    pub scope: String,
}

/// A row of the important people list (Settings → Urgency).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VipRow {
    pub contact_id: String,
    pub display_name: String,
    pub vip_note: Option<String>,
    pub avatar_url: Option<String>,
    pub message_count: i64,
    pub identities: Vec<IdentityRow>,
}

fn identities(conn: &Connection, contact_id: &str) -> Result<Vec<IdentityRow>> {
    let mut st = conn.prepare(
        "SELECT kind, value, scope FROM contact_identities
          WHERE contact_id = ?1 AND deleted_at IS NULL ORDER BY is_primary DESC, created_at",
    )?;
    let rows = st.query_map([contact_id], |r| {
        Ok(IdentityRow {
            kind: r.get(0)?,
            value: r.get(1)?,
            scope: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The important people list, by name.
pub fn list(conn: &Connection) -> Result<Vec<VipRow>> {
    let contacts = repo::query::<Contact>(
        conn,
        "vip = 1 AND deleted_at IS NULL ORDER BY display_name COLLATE NOCASE",
        &[],
    )?;
    contacts
        .into_iter()
        .map(|c| {
            Ok(VipRow {
                identities: identities(conn, &c.id)?,
                contact_id: c.id,
                display_name: c.display_name,
                vip_note: c.vip_note,
                avatar_url: c.avatar_url,
                message_count: c.message_count,
            })
        })
        .collect()
}

/// The contact owning `(kind, scope, value)`, if any.
pub fn find_contact(
    conn: &Connection,
    kind: &str,
    scope: &str,
    value: &str,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT i.contact_id FROM contact_identities i JOIN contacts c ON c.id = i.contact_id
              WHERE i.kind = ?1 AND i.scope = ?2 AND i.value = ?3
                AND i.deleted_at IS NULL AND c.deleted_at IS NULL",
            params![kind, scope, value],
            |r| r.get(0),
        )
        .optional()?)
}

/// Resolve an identity to its contact, creating both lazily (09 §5.3:
/// "Contacts are created lazily from senders"). Emails are lowercased.
pub fn resolve_or_create(
    conn: &Connection,
    device_id: &str,
    kind: &str,
    scope: &str,
    value: &str,
    display_name: &str,
    avatar_url: Option<&str>,
) -> Result<String> {
    let value = if kind == "email" {
        value.trim().to_lowercase()
    } else {
        value.trim().to_string()
    };
    if value.is_empty() {
        return Err(DbError::Invalid {
            what: "identity",
            detail: "empty sender".into(),
        });
    }
    if let Some(id) = find_contact(conn, kind, scope, &value)? {
        // A contact created from a bare address (Add important person by email) takes
        // the sender's name the first time a message brings one.
        let name = display_name.trim();
        if !name.is_empty() && !name.eq_ignore_ascii_case(&value) {
            conn.execute(
                "UPDATE contacts SET display_name = ?2 WHERE id = ?1 AND display_name = ?3",
                params![id, name, value],
            )?;
        }
        return Ok(id);
    }
    let name = if display_name.trim().is_empty() {
        value.clone()
    } else {
        display_name.trim().to_string()
    };
    let mut c = Contact {
        id: repo::new_id(),
        display_name: name,
        primary_email: (kind == "email").then(|| value.clone()),
        avatar_url: avatar_url.map(str::to_string),
        ..Default::default()
    };
    repo::create(conn, device_id, &mut c)?;
    let mut i = ContactIdentity {
        id: repo::new_id(),
        contact_id: c.id.clone(),
        kind: kind.to_string(),
        value,
        scope: scope.to_string(),
        is_primary: true,
        ..Default::default()
    };
    repo::create(conn, device_id, &mut i)?;
    Ok(c.id)
}

/// Count a message toward its contact (`message_count`, `last_seen_at`).
/// A counter, not a user edit: `rev` is not bumped, nothing is logged.
pub fn count_message(conn: &Connection, contact_id: &str, at: &str) -> Result<()> {
    conn.execute(
        "UPDATE contacts SET message_count = message_count + 1,
                last_seen_at = MAX(COALESCE(last_seen_at, ''), ?2)
          WHERE id = ?1",
        params![contact_id, at],
    )?;
    Ok(())
}

/// `vip_add`: by contact id, by the sender of a message, or by an email
/// address / Slack user (created when unknown).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VipAdd {
    #[serde(default)]
    pub contact_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub slack_user: Option<String>,
    #[serde(default)]
    pub slack_team: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub vip_note: Option<String>,
}

fn invalid(d: &str) -> DbError {
    DbError::Invalid {
        what: "vip",
        detail: d.into(),
    }
}

fn contact_for(conn: &Connection, device_id: &str, a: &VipAdd) -> Result<String> {
    if let Some(id) = &a.contact_id {
        return repo::get_live::<Contact>(conn, id)?
            .map(|c| c.id)
            .ok_or_else(|| DbError::NotFound {
                table: "contacts",
                key: id.clone(),
            });
    }
    if let Some(mid) = &a.message_id {
        type SenderCols = (Option<String>, String, String, String, Option<String>);
        let row: Option<SenderCols> = conn
            .query_row(
                "SELECT m.sender_contact_id, m.provider, m.sender_address, m.sender_name, a.team_id
                   FROM messages m JOIN accounts a ON a.id = m.account_id WHERE m.id = ?1",
                [mid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (cid, provider, addr, name, team) = row.ok_or_else(|| DbError::NotFound {
            table: "messages",
            key: mid.clone(),
        })?;
        if let Some(cid) = cid {
            return Ok(cid);
        }
        return if provider == "slack" {
            resolve_or_create(
                conn,
                device_id,
                "slack_user",
                team.as_deref().unwrap_or(""),
                &addr,
                &name,
                None,
            )
        } else {
            resolve_or_create(conn, device_id, "email", "", &addr, &name, None)
        };
    }
    let name = a.display_name.clone().unwrap_or_default();
    if let Some(e) = a.email.as_deref().filter(|e| e.contains('@')) {
        return resolve_or_create(conn, device_id, "email", "", e, &name, None);
    }
    if let Some(u) = a.slack_user.as_deref().filter(|u| !u.is_empty()) {
        return resolve_or_create(
            conn,
            device_id,
            "slack_user",
            a.slack_team.as_deref().unwrap_or(""),
            u,
            &name,
            None,
        );
    }
    Err(invalid(
        "name a contact, a message, an email address or a Slack user",
    ))
}

/// Re-run the deterministic stage for the rows of `where_sql` (bound to
/// `key`) in the window; user overrides are never touched (I6). Rows that
/// lose their tier: the last 24 h go back to `unscored`, older ones become
/// `ai-normal` / `No longer important`. Returns the rows changed.
fn retier(
    conn: &Connection,
    ctx: &Ctx,
    where_sql: &str,
    key: &str,
    window_days: i64,
) -> Result<usize> {
    let since = format_timestamp(ctx.now - time::Duration::days(window_days));
    let recent = format_timestamp(ctx.now - time::Duration::hours(24));
    let now = ctx.now_str();
    let ids: Vec<String> = {
        let sql = format!(
            "SELECT id FROM messages WHERE {where_sql} AND received_at >= ?2
               AND (urgency_source IS NULL OR urgency_source <> 'user')"
        );
        let mut st = conn.prepare(&sql)?;
        let rows = st.query_map(params![key, since], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut n = 0;
    let mut threads = Vec::new();
    for id in ids {
        let Some(c) = super::candidate(conn, &id)? else {
            continue;
        };
        if c.from_self() {
            continue;
        }
        let was_vip = c.urgency_tier == "vip";
        match super::deterministic(&c, ctx) {
            Some(d) if d.tier.as_str() == c.urgency_tier => continue,
            Some(d) => {
                conn.execute(
                    "UPDATE messages SET urgency_tier = ?2, urgency_why = ?3, urgency_source = ?4,
                            urgency_model = NULL, scored_at = ?5, updated_at = ?5 WHERE id = ?1",
                    params![id, d.tier.as_str(), d.why, TierSource::Rule.as_str(), now],
                )?;
            }
            None if was_vip => {
                if c.received_at >= recent {
                    conn.execute(
                        "UPDATE messages SET urgency_tier = 'unscored', urgency_why = NULL,
                                urgency_source = NULL, urgency_model = NULL, scored_at = NULL,
                                updated_at = ?2 WHERE id = ?1",
                        params![id, now],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE messages SET urgency_tier = 'ai-normal', urgency_why = ?2,
                                urgency_source = 'rule', urgency_model = NULL, scored_at = ?3,
                                updated_at = ?3 WHERE id = ?1",
                        params![id, why::VIP_REMOVED, now],
                    )?;
                }
            }
            None => continue,
        }
        if let Some(t) = c.thread_id {
            threads.push(t);
        }
        n += 1;
    }
    super::rank::recompute_threads(conn, &threads, &now)?;
    Ok(n)
}

/// What an important-person change did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VipChange {
    pub contact_id: String,
    pub retiered: usize,
}

/// Mark a contact as an important person (07 §5): `vip = 1`, `vip_note`, then re-tier its
/// messages in the retention window. Never alerts.
pub fn add(
    conn: &Connection,
    device_id: &str,
    a: &VipAdd,
    ctx: &Ctx,
    window_days: i64,
) -> Result<VipChange> {
    let id = contact_for(conn, device_id, a)?;
    let note = a
        .vip_note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    match &note {
        Some(n) => {
            repo::patch::<Contact>(conn, device_id, &id, &[("vip", &true), ("vip_note", n)])?
        }
        None => repo::patch::<Contact>(conn, device_id, &id, &[("vip", &true)])?,
    };
    let retiered = retier(conn, ctx, "sender_contact_id = ?1", &id, window_days)?;
    Ok(VipChange {
        contact_id: id,
        retiered,
    })
}

/// Unmark (07 §5 "Removal"): `vip = 0`, nothing else cleared.
pub fn remove(
    conn: &Connection,
    device_id: &str,
    contact_id: &str,
    ctx: &Ctx,
    window_days: i64,
) -> Result<VipChange> {
    if !repo::patch::<Contact>(conn, device_id, contact_id, &[("vip", &false)])? {
        return Err(DbError::NotFound {
            table: "contacts",
            key: contact_id.into(),
        });
    }
    let retiered = retier(conn, ctx, "sender_contact_id = ?1", contact_id, window_days)?;
    Ok(VipChange {
        contact_id: contact_id.into(),
        retiered,
    })
}

/// Important channels (07 §5, `account_sources.vip`).
pub fn set_source_vip(
    conn: &Connection,
    source_id: &str,
    vip: bool,
    ctx: &Ctx,
    window_days: i64,
) -> Result<usize> {
    let n = conn.execute(
        "UPDATE account_sources SET vip = ?2, updated_at = ?3 WHERE id = ?1",
        params![source_id, vip, ctx.now_str()],
    )?;
    if n == 0 {
        return Err(DbError::NotFound {
            table: "account_sources",
            key: source_id.into(),
        });
    }
    retier(conn, ctx, "source_id = ?1", source_id, window_days)
}

/// `now` helper for callers without a context.
pub fn now_str(now: OffsetDateTime) -> String {
    format_timestamp(now)
}
