//! Accounts (06 §3): N per provider; one row per `(provider, external_id)`
//! (re-connecting the same identity updates tokens, scopes and services in
//! place and keeps every message, thread, link and snooze); sources seeded
//! at connect (§3.4); pause / resume / re-authorize / remove with the
//! cascade, the upstream revoke (best effort) and the keyring delete
//! (§3.3). [`AccountTokens`] is the [`TokenSource`] of a stored account:
//! refresh before expiry and after a 401, under the entry's mutex, with
//! Slack's single-use rotation written back at once (§5, §6.10, §8.9).

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::http::HttpClient;
use super::oauth::{self, OAuthClient, TokenSet};
use super::{Capabilities, ConnectorError, Identity, Provider, SourceCandidate, TokenSource};
use crate::db::{Db, Result as DbResult, format_timestamp, parse_timestamp};
use crate::keyring::{Keyring, names};
use crate::repo::{self, Account, AccountSource};

/// The account chip palette tokens, assigned in turn (06 §3.1 `color`).
pub const PALETTE: &[&str] = &[
    "account-1",
    "account-2",
    "account-3",
    "account-4",
    "account-5",
    "account-6",
];

/// What the Integrations view shows per account (06 §9A; contract
/// `AccountSummary` plus the detail-page fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRow {
    pub id: String,
    pub provider: String,
    pub services: Vec<String>,
    pub display_name: String,
    pub email: Option<String>,
    pub handle: Option<String>,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    pub avatar_url: Option<String>,
    pub status: String,
    pub status_detail: Option<String>,
    pub last_sync_at: Option<String>,
    pub connected_at: String,
    pub color: Option<String>,
    pub position: i64,
    pub scopes: Vec<String>,
    pub capabilities: Capabilities,
    /// "re-authorize within 1 day" (Google Testing 7-day expiry, 06 §6.2),
    /// "Calendar only — re-authorize to add Gmail", "write-back off — …".
    pub notices: Vec<String>,
    pub sources: usize,
}

/// The Google Testing pre-warning: 6 days after `connected_at` (06 §6.2).
pub const GOOGLE_PREWARN_DAYS: i64 = 6;

pub fn row(conn: &Connection, a: &Account, now: OffsetDateTime) -> DbResult<AccountRow> {
    let scopes = super::json_strings(&a.scopes_json);
    let services = super::json_strings(&a.services_json);
    let caps = super::capabilities(a);
    let mut notices = Vec::new();
    if a.provider == "google" {
        if parse_timestamp(&a.connected_at)
            .is_some_and(|t| now - t >= time::Duration::days(GOOGLE_PREWARN_DAYS))
            && a.status == "active"
        {
            notices.push("re-authorize within 1 day".into());
        }
        if !services.iter().any(|s| s == "gmail") && services.iter().any(|s| s == "calendar") {
            notices.push("Calendar only — re-authorize to add Gmail".into());
        }
        if services.iter().any(|s| s == "gmail") && !caps.mark_read_writeback {
            notices.push("write-back off — re-authorize to enable".into());
        }
    } else if !caps.mark_read_writeback {
        notices.push("re-authorize to enable mark read write-back".into());
    }
    let sources: i64 = conn.query_row(
        "SELECT COUNT(*) FROM account_sources WHERE account_id = ?1 AND enabled = 1",
        [&a.id],
        |r| r.get(0),
    )?;
    Ok(AccountRow {
        id: a.id.clone(),
        provider: a.provider.clone(),
        services,
        display_name: a.display_name.clone(),
        email: a.email.clone(),
        handle: a.handle.clone(),
        team_id: a.team_id.clone(),
        team_name: a.team_name.clone(),
        avatar_url: a.avatar_url.clone(),
        status: a.status.clone(),
        status_detail: a.status_detail.clone(),
        last_sync_at: a.last_sync_at.clone(),
        connected_at: a.connected_at.clone(),
        color: a.color.clone(),
        position: a.position,
        scopes,
        capabilities: caps,
        notices,
        sources: sources as usize,
    })
}

pub fn list(conn: &Connection, now: OffsetDateTime) -> DbResult<Vec<AccountRow>> {
    repo::query::<Account>(conn, "1 ORDER BY position, display_name", &[])?
        .iter()
        .map(|a| row(conn, a, now))
        .collect()
}

/// Insert or update by `(provider, external_id)`; returns the row and
/// whether it was new.
pub fn upsert(
    conn: &Connection,
    provider: Provider,
    id: &Identity,
    now: OffsetDateTime,
) -> DbResult<(Account, bool)> {
    let now_s = format_timestamp(now);
    let existing: Option<Account> = repo::query::<Account>(
        conn,
        "provider = ?1 AND external_id = ?2",
        &[&provider.as_str(), &id.external_id],
    )?
    .into_iter()
    .next();
    let services = serde_json::to_string(&id.services).unwrap_or_else(|_| "[]".into());
    let scopes = serde_json::to_string(&id.scopes).unwrap_or_else(|_| "[]".into());
    if let Some(mut a) = existing {
        a.email = id.email.clone().or(a.email);
        a.handle = id.handle.clone().or(a.handle);
        a.team_id = id.team_id.clone().or(a.team_id);
        a.team_name = id.team_name.clone().or(a.team_name);
        a.avatar_url = id.avatar_url.clone().or(a.avatar_url);
        a.services_json = services;
        a.scopes_json = scopes;
        a.status = "active".into();
        a.status_detail = None;
        a.connected_at = now_s.clone();
        a.updated_at = now_s;
        if let Some(url) = &id.team_url {
            let mut cursor: serde_json::Value =
                serde_json::from_str(&a.cursor_json).unwrap_or_else(|_| serde_json::json!({}));
            cursor["team_url"] = serde_json::Value::String(url.clone());
            a.cursor_json = cursor.to_string();
        }
        repo::replace(conn, &a)?;
        return Ok((a, false));
    }
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
    let account_id = repo::new_id();
    let cursor = match &id.team_url {
        Some(u) => serde_json::json!({ "team_url": u }).to_string(),
        None => "{}".into(),
    };
    let a = Account {
        keyring_ref: names::account(&account_id),
        id: account_id,
        provider: provider.as_str().into(),
        external_id: id.external_id.clone(),
        display_name: id.display_name.clone(),
        email: id.email.clone(),
        handle: id.handle.clone(),
        team_id: id.team_id.clone(),
        team_name: id.team_name.clone(),
        avatar_url: id.avatar_url.clone(),
        services_json: services,
        scopes_json: scopes,
        status: "active".into(),
        status_detail: None,
        cursor_json: cursor,
        color: Some(PALETTE[(n as usize) % PALETTE.len()].to_string()),
        position: n,
        connected_at: now_s.clone(),
        last_sync_at: None,
        last_error_at: None,
        created_at: now_s.clone(),
        updated_at: now_s,
    };
    repo::insert(conn, &a)?;
    Ok((a, true))
}

/// Seed sources (06 §3.4); existing rows (and their cursors) are kept.
/// Returns how many were added.
pub fn seed_sources(
    conn: &Connection,
    account_id: &str,
    cands: &[SourceCandidate],
    now: &str,
) -> DbResult<usize> {
    let mut n = 0;
    for c in cands {
        n += conn.execute(
            "INSERT OR IGNORE INTO account_sources
               (id, account_id, kind, external_id, name, enabled, vip, poll_seconds, cursor,
                last_polled_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, NULL, NULL, ?7, ?7)",
            params![
                repo::new_id(),
                account_id,
                c.kind.as_str(),
                c.external_id,
                c.name,
                c.enabled || c.kind.always_on(),
                now
            ],
        )?;
    }
    Ok(n)
}

/// Every source of an account.
pub fn sources(conn: &Connection, account_id: &str) -> DbResult<Vec<AccountSource>> {
    repo::query::<AccountSource>(conn, "account_id = ?1 ORDER BY kind, name", &[&account_id])
}

/// Enable / disable a source; always-on Slack sources refuse to turn off.
pub fn set_source_enabled(
    conn: &Connection,
    source_id: &str,
    enabled: bool,
    now: &str,
) -> DbResult<AccountSource> {
    let s = repo::must_get::<AccountSource>(conn, source_id)?;
    if !enabled && super::SourceKind::parse(&s.kind).is_some_and(|k| k.always_on()) {
        return Err(crate::db::DbError::Invalid {
            what: "source",
            detail: "DMs, group DMs and mentions are always watched".into(),
        });
    }
    conn.execute(
        "UPDATE account_sources SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
        params![source_id, enabled, now],
    )?;
    repo::must_get::<AccountSource>(conn, source_id)
}

/// Pause (06 §3.3): pollers and the write-back queue stop; rows stay.
pub fn pause(conn: &Connection, account_id: &str, now: OffsetDateTime) -> DbResult<bool> {
    Ok(
        super::health::transition(conn, account_id, &super::health::Transition::UserPause, now)?
            .is_some(),
    )
}

/// Resume: `active`; the caller schedules an immediate catch-up poll.
pub fn resume(conn: &Connection, account_id: &str, now: OffsetDateTime) -> DbResult<bool> {
    Ok(super::health::transition(
        conn,
        account_id,
        &super::health::Transition::UserResume,
        now,
    )?
    .is_some())
}

/// What a removal did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub account_id: String,
    pub messages: usize,
    pub events: usize,
    /// Upstream revoke failed: "the upstream grant may remain" (06 §2.1).
    pub upstream_may_remain: bool,
    pub snoozes_tombstoned: usize,
    pub links_tombstoned: usize,
}

/// Delete the row (cascading to sources, threads, messages, events and
/// queued write-backs), then tombstone the snoozes and links that pointed
/// at removed rows. Contacts stay (they are the user's data).
pub fn delete_rows(conn: &Connection, device_id: &str, account_id: &str) -> DbResult<Removed> {
    let mut r = Removed {
        account_id: account_id.into(),
        ..Default::default()
    };
    r.messages = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE account_id = ?1",
        [account_id],
        |x| x.get::<_, i64>(0),
    )? as usize;
    r.events = conn.query_row(
        "SELECT COUNT(*) FROM calendar_events WHERE account_id = ?1",
        [account_id],
        |x| x.get::<_, i64>(0),
    )? as usize;
    conn.execute_batch(
        "DROP TABLE IF EXISTS temp.removed_items;
         CREATE TEMP TABLE removed_items (kind TEXT, id TEXT);",
    )?;
    conn.execute(
        "INSERT INTO temp.removed_items SELECT 'message', id FROM messages WHERE account_id = ?1
         UNION ALL SELECT 'thread', id FROM threads WHERE account_id = ?1
         UNION ALL SELECT 'calendar_event', id FROM calendar_events WHERE account_id = ?1",
        [account_id],
    )?;
    conn.execute("DELETE FROM accounts WHERE id = ?1", [account_id])?;
    let now = crate::db::now_rfc3339();
    r.snoozes_tombstoned = conn.execute(
        "UPDATE snoozes SET deleted_at = ?1, updated_at = ?1, rev = rev + 1, device_id = ?2
          WHERE deleted_at IS NULL AND EXISTS (SELECT 1 FROM temp.removed_items x
                 WHERE x.kind = snoozes.target_kind AND x.id = snoozes.target_id)",
        params![now, device_id],
    )?;
    r.links_tombstoned = conn.execute(
        "UPDATE links SET deleted_at = ?1, updated_at = ?1, rev = rev + 1, device_id = ?2
          WHERE deleted_at IS NULL AND (
                EXISTS (SELECT 1 FROM temp.removed_items x WHERE x.kind = links.from_kind AND x.id = links.from_id)
             OR EXISTS (SELECT 1 FROM temp.removed_items x WHERE x.kind = links.to_kind AND x.id = links.to_id))",
        params![now, device_id],
    )?;
    conn.execute_batch("DROP TABLE IF EXISTS temp.removed_items;")?;
    Ok(r)
}

/// Remove (06 §3.3): revoke upstream (best effort), delete the keyring
/// entry, delete the rows.
pub fn remove(
    db: &Db,
    http: &dyn HttpClient,
    keyring: &Keyring,
    account_id: &str,
) -> std::result::Result<Removed, ConnectorError> {
    let a = db
        .read(|c| repo::get_by_id::<Account>(c, account_id))?
        .ok_or(ConnectorError::NotFound)?;
    let provider = Provider::parse(&a.provider).unwrap_or(Provider::Google);
    let tokens: Option<TokenSet> = keyring.get_json(&a.keyring_ref).ok().flatten();
    let mut upstream_failed = false;
    if let Some(t) = &tokens
        && oauth::revoke(http, provider, t).is_err()
    {
        upstream_failed = true;
    }
    let _ = keyring.delete(&a.keyring_ref);
    let device = db.device_id().to_string();
    let id = account_id.to_string();
    let mut r = db.write(move |c| repo::tx(c, |tx| delete_rows(tx, &device, &id)))?;
    r.upstream_may_remain = upstream_failed || tokens.is_none();
    Ok(r)
}

// ----------------------------------------------------------------- tokens

/// The token source of a stored account.
pub struct AccountTokens {
    pub provider: Provider,
    pub entry: String,
    pub keyring: Arc<Keyring>,
    pub http: Arc<dyn HttpClient>,
    pub client: Option<OAuthClient>,
    /// The clock (tests pin it).
    pub now: Box<dyn Fn() -> OffsetDateTime + Send + Sync>,
    /// Called with the granted scopes after a refresh (scope downgrade, 06 §6.10).
    pub on_scopes: Option<Box<dyn Fn(Vec<String>) + Send + Sync>>,
}

impl AccountTokens {
    pub fn for_account(
        account: &Account,
        keyring: Arc<Keyring>,
        http: Arc<dyn HttpClient>,
    ) -> AccountTokens {
        let provider = Provider::parse(&account.provider).unwrap_or(Provider::Google);
        let client = oauth::load_client(&keyring, provider, account.team_id.as_deref()).ok();
        AccountTokens {
            provider,
            entry: account.keyring_ref.clone(),
            keyring,
            http,
            client,
            now: Box::new(OffsetDateTime::now_utc),
            on_scopes: None,
        }
    }

    fn stored(&self) -> Result<TokenSet, ConnectorError> {
        self.keyring
            .get_json::<TokenSet>(&self.entry)?
            .ok_or_else(|| ConnectorError::InvalidGrant("no stored token".into()))
    }

    /// Refresh under the entry's mutex; `force` refreshes even when fresh.
    pub fn refresh_now(&self, force: bool) -> Result<TokenSet, ConnectorError> {
        let lock = self.keyring.entry_lock(&self.entry);
        let _g = lock.lock().unwrap_or_else(|p| p.into_inner());
        let cur = self.stored()?;
        let now = (self.now)();
        if !force && !cur.needs_refresh(self.provider, now) {
            return Ok(cur);
        }
        let Some(rt) = cur.refresh_token.clone() else {
            return if cur.expires_at.is_none() {
                Ok(cur)
            } else {
                Err(ConnectorError::InvalidGrant("no refresh token".into()))
            };
        };
        let client = self
            .client
            .clone()
            .ok_or_else(|| ConnectorError::NotConfigured("the OAuth client is missing".into()))?;
        let ex = oauth::refresh(self.http.as_ref(), self.provider, &client, &rt, now)?;
        let mut next = ex.tokens;
        if next.refresh_token.is_none() {
            next.refresh_token = Some(rt);
        }
        if next.scope.is_empty() {
            next.scope = cur.scope.clone();
        }
        // Google keeps its refresh token: the rotation time stays.
        if self.provider == Provider::Google && next.refresh_token == cur.refresh_token {
            next.rotated_at = cur.rotated_at.clone();
        }
        self.keyring.set_json(&self.entry, &next)?;
        if let Some(f) = &self.on_scopes
            && !ex.scopes.is_empty()
        {
            f(ex.scopes);
        }
        Ok(next)
    }
}

impl TokenSource for AccountTokens {
    fn access_token(&self) -> Result<String, ConnectorError> {
        let cur = self.stored()?;
        if cur.needs_refresh(self.provider, (self.now)()) {
            return Ok(self.refresh_now(false)?.access_token);
        }
        Ok(cur.access_token)
    }

    fn refresh(&self) -> Result<String, ConnectorError> {
        Ok(self.refresh_now(true)?.access_token)
    }
}

/// Store the scopes a refresh reported (06 §6.10 scope downgrade).
pub fn update_scopes(
    conn: &Connection,
    account_id: &str,
    scopes: &[String],
    now: &str,
) -> DbResult<()> {
    conn.execute(
        "UPDATE accounts SET scopes_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![
            account_id,
            serde_json::to_string(scopes).unwrap_or_else(|_| "[]".into()),
            now
        ],
    )?;
    Ok(())
}

/// The account id owning a keyring entry, if any (Diagnostics).
pub fn by_keyring_ref(conn: &Connection, entry: &str) -> DbResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM accounts WHERE keyring_ref = ?1",
            [entry],
            |r| r.get(0),
        )
        .optional()?)
}
