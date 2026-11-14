//! The Gmail adapter (06 §6): identity from `userinfo`, the initial sync
//! (§6.4: `getProfile` → `historyId` captured first, `labels.list`,
//! `messages.list` over `INBOX` within `gmail.initial_sync_days` capped at
//! `gmail.initial_sync_max`, `messages.get` per the body policy),
//! incremental sync with `history.list` (§6.5: no label filter, folded per
//! message, the cursor is the response `historyId`, 404 → a bounded
//! resync), the storage mapping (§6.6), the body fetch policy (§6.7),
//! triage write-back (§6.8: `messages.modify` / `threads.modify` with
//! `UNREAD` / `INBOX`), Google's error bodies (§6.9) and open-in-source
//! (§6.11). The initial sync is resumable: its progress lives in
//! `accounts.cursor_json.gmail_backfill` so a budget that runs out mid-way
//! resumes next minute.

pub mod mime;

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::http::{HttpRequest, HttpResponse};
use super::store::{self, NewMessage, NewThread, Sender};
use super::{
    Api, Capabilities, ConnectorAdapter, ConnectorError, FetchedBody, Identity, Provider, Result,
    SourceCandidate, SourceKind, SyncCx, SyncOutcome, oauth,
};
use crate::repo::{Account, AccountSource, ConnectorWriteback, Message};
use crate::urgency;

pub const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
pub const USERINFO: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// `metadataHeaders` (06 §6.6).
pub const METADATA_HEADERS: &[&str] = &[
    "From",
    "To",
    "Cc",
    "Subject",
    "Date",
    "Message-ID",
    "In-Reply-To",
    "List-Unsubscribe",
    "List-Id",
    "Precedence",
    "Auto-Submitted",
];

/// Never stored (06 §6.5).
pub const NEVER_STORED: &[&str] = &["DRAFT", "SPAM", "TRASH"];

/// Messages ingested per write transaction.
const CHUNK: usize = 25;

/// Gmail's capability flags from the granted scopes (06 §2.2, §6.3).
pub fn capabilities(scopes: &[String]) -> Capabilities {
    let modify = scopes.iter().any(|s| s == oauth::SCOPE_GMAIL_MODIFY);
    Capabilities {
        read_messages: true,
        read_events: scopes.iter().any(|s| s == oauth::SCOPE_CALENDAR_READONLY),
        fetch_body_on_demand: true,
        mark_read_writeback: modify,
        mark_unread_writeback: modify,
        archive_writeback: modify,
        thread_actions: if modify { "full".into() } else { "none".into() },
        deep_link: "web".into(),
        incremental_cursor: "history_id".into(),
    }
}

/// Open in source (06 §6.11): `#all/` so archived threads open too.
pub fn source_url(account_email: &str, thread_id: &str) -> String {
    format!("https://mail.google.com/mail/?authuser={account_email}#all/{thread_id}")
}

/// Google's error body → [`ConnectorError`] (06 §6.9).
pub fn google_error(resp: &HttpResponse) -> ConnectorError {
    let v = resp.json_value();
    let reason = v
        .pointer("/error/errors/0/reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    match (resp.status, reason) {
        (429, _) | (403, "rateLimitExceeded" | "userRateLimitExceeded") => {
            ConnectorError::RateLimited {
                retry_after_secs: resp.retry_after_secs().unwrap_or(1),
            }
        }
        (403, "accessNotConfigured") => ConnectorError::Config(
            "The Gmail API is not enabled in your Google Cloud project".into(),
        ),
        (403, "insufficientPermissions") | (403, "ACCESS_TOKEN_SCOPE_INSUFFICIENT") => {
            ConnectorError::MissingScope("a Gmail scope was not granted".into())
        }
        (404 | 410, _) => ConnectorError::NotFound,
        (s, _) if s >= 500 => ConnectorError::Server(s, super::short_detail(&v)),
        _ => super::classify_status(resp),
    }
}

fn ok_json(api: &Api<'_>, req: HttpRequest) -> Result<Value> {
    let resp = api.send(req)?;
    if !resp.ok() {
        return Err(google_error(&resp));
    }
    Ok(resp.json_value())
}

/// `complete_connect` identity for a Google account (06 §3.1, §4.2):
/// `userinfo` gives `sub`, `email`, `picture`; services from the scopes.
pub fn google_identify(api: &Api<'_>, granted: &[String]) -> Result<Identity> {
    let v = ok_json(api, HttpRequest::get(USERINFO))?;
    let sub = v
        .get("sub")
        .and_then(Value::as_str)
        .ok_or_else(|| ConnectorError::Protocol("userinfo without sub".into()))?;
    let email = v
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    Ok(Identity {
        external_id: sub.to_string(),
        display_name: email.clone().unwrap_or_else(|| sub.to_string()),
        email,
        handle: None,
        team_id: None,
        team_name: None,
        team_url: None,
        avatar_url: v.get("picture").and_then(Value::as_str).map(str::to_string),
        services: oauth::google_services(granted),
        scopes: granted.to_vec(),
    })
}

/// The resumable initial sync (06 §6.4) in `cursor_json.gmail_backfill`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backfill {
    pub history_id: String,
    #[serde(default)]
    pub listed: bool,
    #[serde(default)]
    pub page_token: Option<String>,
    #[serde(default)]
    pub pending: Vec<String>,
    #[serde(default)]
    pub total: usize,
    /// A resync after a lost cursor keeps local read / archive state.
    #[serde(default)]
    pub resync: bool,
}

fn read_cursor(cx: &SyncCx<'_>) -> Result<Value> {
    let id = cx.account.id.clone();
    let raw: String = cx.db.read(move |c| {
        Ok(c.query_row(
            "SELECT cursor_json FROM accounts WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )?)
    })?;
    Ok(serde_json::from_str(&raw).unwrap_or_else(|_| json!({})))
}

fn write_cursor(cx: &SyncCx<'_>, f: impl FnOnce(&mut Value) + Send + 'static) -> Result<()> {
    let id = cx.account.id.clone();
    let now = cx.now_str();
    cx.db.write(move |c| {
        let raw: String = c.query_row(
            "SELECT cursor_json FROM accounts WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )?;
        let mut v: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
        if !v.is_object() {
            v = json!({});
        }
        f(&mut v);
        c.execute(
            "UPDATE accounts SET cursor_json = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, v.to_string(), now],
        )?;
        Ok(())
    })?;
    Ok(())
}

/// A message's folded history: final labels, added, deleted, first-seen order.
type Fold = (Option<Vec<String>>, bool, bool, usize);

/// The Gmail adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct GmailAdapter;

impl GmailAdapter {
    fn watched_labels(&self, cx: &SyncCx<'_>) -> Result<Vec<(String, String)>> {
        let id = cx.account.id.clone();
        Ok(cx.db.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, external_id FROM account_sources
                  WHERE account_id = ?1 AND kind = 'gmail_label' AND enabled = 1",
            )?;
            let r = st.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(r.collect::<rusqlite::Result<Vec<_>>>()?)
        })?)
    }

    fn inbox_source_id(&self, cx: &SyncCx<'_>) -> Result<Option<String>> {
        let id = cx.account.id.clone();
        Ok(cx.db.read(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row(
                "SELECT id FROM account_sources WHERE account_id = ?1 AND kind = 'gmail_inbox'",
                [&id],
                |r| r.get(0),
            )
            .optional()?)
        })?)
    }

    /// `messages.get` per the body policy; `None` when it vanished (404).
    fn fetch(&self, cx: &SyncCx<'_>, id: &str) -> Result<Option<Value>> {
        let policy = cx.settings.gmail_body_fetch.as_str();
        let url = format!("{API}/messages/{id}");
        let get = |format: &str| -> Result<Option<Value>> {
            let mut req = HttpRequest::get(&url).query(&[("format", format)]);
            if format == "metadata" {
                let hs: Vec<(&str, &str)> = METADATA_HEADERS
                    .iter()
                    .map(|h| ("metadataHeaders", *h))
                    .collect();
                req = req.query(&hs);
            }
            match ok_json(&cx.api, req) {
                Ok(v) => Ok(Some(v)),
                Err(ConnectorError::NotFound) => Ok(None),
                Err(e) => Err(e),
            }
        };
        if policy == "always" {
            return get("full");
        }
        let Some(meta) = get("metadata")? else {
            return Ok(None);
        };
        if policy == "never" {
            return Ok(Some(meta));
        }
        let labels = labels_of(&meta);
        let wants_full = labels.iter().any(|l| l == "INBOX")
            && !labels
                .iter()
                .any(|l| l == "CATEGORY_PROMOTIONS" || l == "CATEGORY_SOCIAL");
        if !wants_full {
            return Ok(Some(meta));
        }
        match get("full") {
            Ok(Some(full)) => Ok(Some(full)),
            Ok(None) => Ok(None),
            // The budget ran out between the two calls: keep the metadata.
            Err(ConnectorError::BudgetExhausted) => Ok(Some(meta)),
            Err(e) => Err(e),
        }
    }

    /// Map a `messages.get` reply to a stored message, or `None` for
    /// drafts / spam / trash and mail outside `INBOX` and watched labels.
    pub fn to_new(
        account: &Account,
        msg: &Value,
        inbox_source: Option<&str>,
        watched: &[(String, String)],
        state_changed: bool,
    ) -> Option<NewMessage> {
        let id = msg.get("id")?.as_str()?;
        let thread_id = msg.get("threadId")?.as_str()?;
        let labels = labels_of(msg);
        if labels.iter().any(|l| NEVER_STORED.contains(&l.as_str())) {
            return None;
        }
        let in_inbox = labels.iter().any(|l| l == "INBOX");
        let label_source = watched
            .iter()
            .find(|(_, ext)| labels.iter().any(|l| l == ext))
            .map(|(sid, _)| sid.clone());
        if !in_inbox && label_source.is_none() {
            return None;
        }
        let payload = msg.get("payload").cloned().unwrap_or(Value::Null);
        let h = |n: &str| mime::header(&payload, n).map(str::to_string);
        let (sender_name, sender_addr) = mime::parse_address(&h("From").unwrap_or_default());
        let me = account.email.clone().unwrap_or_default().to_lowercase();
        let to: Vec<(String, String)> = h("To")
            .map(|t| mime::split_addresses(&t))
            .unwrap_or_default()
            .iter()
            .map(|a| mime::parse_address(a))
            .collect();
        let cc: Vec<(String, String)> = h("Cc")
            .map(|t| mime::split_addresses(&t))
            .unwrap_or_default()
            .iter()
            .map(|a| mime::parse_address(a))
            .collect();
        let is_direct = to.len() == 1 && !me.is_empty() && to[0].1 == me;
        let subject = h("Subject").unwrap_or_default();
        let (body, has_att) = mime::body_text(&payload);
        let gmail_snippet =
            mime::decode_entities(msg.get("snippet").and_then(Value::as_str).unwrap_or(""));
        let (snippet, body_text, truncated) = match &body {
            Some(b) => {
                let (t, cut) = super::cap_body(b, super::BODY_CAP_BYTES);
                (super::snippet_of(b), Some(t), cut)
            }
            None => (super::snippet_of(&gmail_snippet), None, false),
        };
        let mut all_labels = labels.clone();
        all_labels.extend(mime::hint_labels(&payload));
        if has_att {
            all_labels.push("attachments".into());
        }
        let received_at = msg
            .get("internalDate")
            .and_then(|v| {
                v.as_str()
                    .map(str::to_string)
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
            })
            .and_then(|ms| ms.parse::<f64>().ok())
            .and_then(|ms| super::ts_from_epoch_f64(ms / 1000.0))
            .unwrap_or_else(crate::db::now_rfc3339);
        let rfc = h("Message-ID").map(|m| {
            m.trim()
                .trim_matches(|c| c == '<' || c == '>')
                .to_lowercase()
        });
        let participants: Vec<Value> = std::iter::once((sender_name.clone(), sender_addr.clone()))
            .chain(to.iter().cloned())
            .chain(cc.iter().cloned())
            .map(|(n, a)| json!({"name": n, "address": a}))
            .collect();
        let url = source_url(&me, thread_id);
        Some(NewMessage {
            account_id: account.id.clone(),
            source_id: if in_inbox {
                inbox_source.map(str::to_string).or(label_source)
            } else {
                label_source
            },
            provider: Provider::Google,
            external_id: id.to_string(),
            dedup_key: urgency::dedup::email_key(rfc.as_deref(), &account.id, id),
            rfc_message_id: rfc.filter(|r| !r.is_empty()),
            kind: "email".into(),
            thread: NewThread {
                external_id: thread_id.to_string(),
                kind: "email".into(),
                title: mime::thread_title(&subject),
                channel_id: None,
                channel_name: None,
                participants: Value::Array(participants),
                source_url: Some(url.clone()),
            },
            sender: Sender {
                name: sender_name,
                address: sender_addr,
                avatar_url: None,
                identity_kind: "email".into(),
                identity_scope: String::new(),
            },
            channel_id: None,
            channel_name: None,
            subject: Some(subject).filter(|s| !s.is_empty()),
            snippet,
            body_text,
            body_truncated: truncated,
            has_attachments: has_att,
            is_direct,
            is_read: Some(!labels.iter().any(|l| l == "UNREAD")),
            is_archived: Some(!in_inbox),
            state_changed,
            labels: all_labels,
            received_at,
            source_url: url,
        })
    }

    fn ingest(&self, cx: &SyncCx<'_>, msgs: Vec<NewMessage>, out: &mut SyncOutcome) -> Result<()> {
        if msgs.is_empty() {
            return Ok(());
        }
        let device = cx.db.device_id().to_string();
        let (now, tz, days) = (cx.now, cx.tz, cx.settings.message_body_days);
        let rep = cx.db.write(move |c| {
            crate::repo::tx(c, |tx| {
                let ctx = urgency::Ctx::load(tx, now, tz)?;
                store::ingest(tx, &device, &msgs, &ctx, days)
            })
        })?;
        out.inserted.extend(rep.inserted);
        out.updated += rep.updated;
        out.alerts.extend(rep.urgency.alerts);
        Ok(())
    }

    /// One step of the initial sync / resync.
    fn backfill(&self, cx: &SyncCx<'_>, mut bf: Backfill, out: &mut SyncOutcome) -> Result<()> {
        out.full_sync = true;
        if bf.history_id.is_empty() {
            let p = ok_json(&cx.api, HttpRequest::get(format!("{API}/profile")))?;
            bf.history_id = p
                .get("historyId")
                .and_then(|h| {
                    h.as_str()
                        .map(str::to_string)
                        .or_else(|| h.as_u64().map(|n| n.to_string()))
                })
                .ok_or_else(|| ConnectorError::Protocol("profile without historyId".into()))?;
            if let Ok(l) = ok_json(&cx.api, HttpRequest::get(format!("{API}/labels"))) {
                let names: BTreeMap<String, String> = l
                    .get("labels")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| {
                                Some((
                                    x.get("id")?.as_str()?.to_string(),
                                    x.get("name")?.as_str()?.to_string(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                write_cursor(cx, move |v| v["gmail_labels"] = json!(names))?;
            }
            let snap = bf.clone();
            write_cursor(cx, move |v| v["gmail_backfill"] = json!(snap))?;
        }
        let max = cx.settings.gmail_initial_sync_max.max(1) as usize;
        while !bf.listed {
            let mut req = HttpRequest::get(format!("{API}/messages")).query(&[
                ("labelIds", "INBOX"),
                (
                    "q",
                    &format!("newer_than:{}d", cx.settings.gmail_initial_sync_days),
                ),
                ("maxResults", "100"),
            ]);
            if let Some(t) = &bf.page_token {
                req = req.query(&[("pageToken", t.as_str())]);
            }
            let page = match ok_json(&cx.api, req) {
                Ok(p) => p,
                Err(ConnectorError::BudgetExhausted) => {
                    out.more = true;
                    let snap = bf.clone();
                    write_cursor(cx, move |v| v["gmail_backfill"] = json!(snap))?;
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            for m in page
                .get("messages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if bf.pending.len() + bf.total >= max {
                    break;
                }
                if let Some(id) = m.get("id").and_then(Value::as_str)
                    && !bf.pending.iter().any(|p| p == id)
                {
                    bf.pending.push(id.to_string());
                }
            }
            bf.page_token = page
                .get("nextPageToken")
                .and_then(Value::as_str)
                .map(str::to_string);
            if bf.page_token.is_none() || bf.pending.len() + bf.total >= max {
                bf.listed = true;
                bf.page_token = None;
            }
        }
        let inbox = self.inbox_source_id(cx)?;
        let watched = self.watched_labels(cx)?;
        let mut chunk = Vec::new();
        let mut done = 0usize;
        let pending = bf.pending.clone();
        let mut budget_hit = false;
        for id in &pending {
            let known = {
                let (acc, ext) = (cx.account.id.clone(), id.clone());
                cx.db.read(move |c| store::message_id(c, &acc, &ext))?
            };
            if known.is_none() {
                match self.fetch(cx, id) {
                    Ok(Some(m)) => {
                        if let Some(n) =
                            Self::to_new(cx.account, &m, inbox.as_deref(), &watched, false)
                        {
                            chunk.push(n);
                        }
                    }
                    Ok(None) => {}
                    Err(ConnectorError::BudgetExhausted) => {
                        budget_hit = true;
                        break;
                    }
                    Err(e) => {
                        self.ingest(cx, std::mem::take(&mut chunk), out)?;
                        bf.pending.drain(..done);
                        bf.total += done;
                        let snap = bf.clone();
                        write_cursor(cx, move |v| v["gmail_backfill"] = json!(snap))?;
                        return Err(e);
                    }
                }
            }
            done += 1;
            if chunk.len() >= CHUNK {
                self.ingest(cx, std::mem::take(&mut chunk), out)?;
            }
        }
        self.ingest(cx, chunk, out)?;
        bf.pending.drain(..done);
        bf.total += done;
        if budget_hit || !bf.pending.is_empty() {
            out.more = true;
            let snap = bf.clone();
            write_cursor(cx, move |v| v["gmail_backfill"] = json!(snap))?;
            return Ok(());
        }
        let hid = bf.history_id.clone();
        write_cursor(cx, move |v| {
            v["gmail_history_id"] = json!(hid);
            if let Some(o) = v.as_object_mut() {
                o.remove("gmail_backfill");
            }
        })?;
        Ok(())
    }

    /// Incremental sync (06 §6.5).
    fn history(&self, cx: &SyncCx<'_>, start: &str, out: &mut SyncOutcome) -> Result<()> {
        let mut page_token: Option<String> = None;
        let mut latest = start.to_string();
        // message id → (final labels, added, deleted, order)
        let mut fold: HashMap<String, Fold> = HashMap::new();
        let mut order = 0usize;
        loop {
            let mut req = HttpRequest::get(format!("{API}/history")).query(&[
                ("startHistoryId", start),
                ("historyTypes", "messageAdded"),
                ("historyTypes", "messageDeleted"),
                ("historyTypes", "labelAdded"),
                ("historyTypes", "labelRemoved"),
                ("maxResults", "500"),
            ]);
            if let Some(t) = &page_token {
                req = req.query(&[("pageToken", t.as_str())]);
            }
            let resp = cx.api.send(req)?;
            if resp.status == 404 {
                return Err(ConnectorError::CursorGone);
            }
            if !resp.ok() {
                return Err(google_error(&resp));
            }
            let v = resp.json_value();
            for rec in v
                .get("history")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let mut touch = |m: &Value, added: bool, deleted: bool| {
                    let Some(id) = m.get("id").and_then(Value::as_str) else {
                        return;
                    };
                    order += 1;
                    let e = fold
                        .entry(id.to_string())
                        .or_insert((None, false, false, order));
                    if let Some(l) = m.get("labelIds").and_then(Value::as_array) {
                        e.0 = Some(
                            l.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect(),
                        );
                    }
                    e.1 |= added;
                    e.2 |= deleted;
                    e.3 = order;
                };
                for (key, added, deleted) in [
                    ("messagesAdded", true, false),
                    ("messagesDeleted", false, true),
                    ("labelsAdded", false, false),
                    ("labelsRemoved", false, false),
                ] {
                    for item in rec.get(key).and_then(Value::as_array).into_iter().flatten() {
                        if let Some(m) = item.get("message") {
                            touch(m, added, deleted);
                        }
                    }
                }
            }
            if let Some(h) = v.get("historyId") {
                latest = h
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| h.as_u64().map(|n| n.to_string()))
                    .unwrap_or(latest);
            }
            page_token = v
                .get("nextPageToken")
                .and_then(Value::as_str)
                .map(str::to_string);
            if page_token.is_none() {
                break;
            }
        }
        let mut items: Vec<(String, Fold)> = fold.into_iter().collect();
        items.sort_by_key(|(_, e)| e.3);
        let inbox = self.inbox_source_id(cx)?;
        let watched = self.watched_labels(cx)?;
        let mut new_msgs = Vec::new();
        for (id, (labels, _added, deleted, _)) in items {
            let known = {
                let (acc, ext) = (cx.account.id.clone(), id.clone());
                cx.db.read(move |c| store::message_id(c, &acc, &ext))?
            };
            if deleted {
                if known.is_some() {
                    let (acc, ext) = (cx.account.id.clone(), id.clone());
                    cx.db.write(move |c| store::delete_message(c, &acc, &ext))?;
                    out.deleted += 1;
                }
                continue;
            }
            match (known, labels) {
                (Some(mid), Some(l)) => {
                    // Mirror reading / archiving done in Gmail itself.
                    if l.iter().any(|x| NEVER_STORED.contains(&x.as_str())) {
                        let (acc, ext) = (cx.account.id.clone(), id.clone());
                        cx.db.write(move |c| store::delete_message(c, &acc, &ext))?;
                        out.deleted += 1;
                        continue;
                    }
                    let now = cx.now_str();
                    let read = !l.iter().any(|x| x == "UNREAD");
                    let archived = !l.iter().any(|x| x == "INBOX");
                    let labels_json = serde_json::to_string(&l).unwrap_or_default();
                    let changed = cx.db.write(move |c| {
                        crate::repo::tx(c, |tx| {
                            // keep the hint labels Mode derived from headers
                            tx.execute(
                                "UPDATE messages SET labels_json = (
                                    SELECT json_group_array(value) FROM (
                                      SELECT value FROM json_each(?2)
                                      UNION SELECT value FROM json_each(messages.labels_json)
                                       WHERE value IN ('list','bulk','auto','attachments','mode:not_synced')))
                                  WHERE id = ?1",
                                rusqlite::params![mid, labels_json],
                            )?;
                            let t = store::apply_provider_state(tx, &mid, Some(read), Some(archived), true, &now)?;
                            let thread: Option<String> = tx.query_row(
                                "SELECT thread_id FROM messages WHERE id = ?1",
                                [&mid],
                                |r| r.get(0),
                            )?;
                            urgency::rank::recompute_threads(tx, &thread.into_iter().collect::<Vec<_>>(), &now)?;
                            Ok(t)
                        })
                    })?;
                    if changed {
                        out.updated += 1;
                    }
                }
                (Some(_), None) => {}
                (None, l) => {
                    // New to Mode (added, or moved into INBOX / a watched label).
                    let relevant = l.as_ref().is_none_or(|l| {
                        l.iter()
                            .any(|x| x == "INBOX" || watched.iter().any(|(_, e)| e == x))
                    });
                    if !relevant {
                        continue;
                    }
                    match self.fetch(cx, &id) {
                        Ok(Some(m)) => {
                            if let Some(n) =
                                Self::to_new(cx.account, &m, inbox.as_deref(), &watched, true)
                            {
                                new_msgs.push(n);
                            }
                        }
                        Ok(None) => {}
                        Err(ConnectorError::BudgetExhausted) => {
                            // Keep the old cursor: the next poll replays (idempotent).
                            out.more = true;
                            self.ingest(cx, new_msgs, out)?;
                            return Ok(());
                        }
                        Err(e) => return Err(e),
                    }
                    if new_msgs.len() >= CHUNK {
                        self.ingest(cx, std::mem::take(&mut new_msgs), out)?;
                    }
                }
            }
        }
        self.ingest(cx, new_msgs, out)?;
        write_cursor(cx, move |v| v["gmail_history_id"] = json!(latest))?;
        Ok(())
    }
}

/// The `labelIds` of a message object.
pub fn labels_of(msg: &Value) -> Vec<String> {
    msg.get("labelIds")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

impl ConnectorAdapter for GmailAdapter {
    fn provider(&self) -> Provider {
        Provider::Google
    }

    fn capabilities(&self, account: &Account) -> Capabilities {
        capabilities(&super::json_strings(&account.scopes_json))
    }

    fn identify(&self, api: &Api<'_>, granted: &[String]) -> Result<Identity> {
        google_identify(api, granted)
    }

    fn list_sources(&self, api: &Api<'_>, account: &Account) -> Result<Vec<SourceCandidate>> {
        let mut out = Vec::new();
        let services = super::json_strings(&account.services_json);
        if services.iter().any(|s| s == "gmail") {
            out.push(SourceCandidate {
                kind: SourceKind::GmailInbox,
                external_id: "INBOX".into(),
                name: "Gmail inbox".into(),
                enabled: true,
                private: false,
                member_count: None,
            });
            if let Ok(l) = ok_json(api, HttpRequest::get(format!("{API}/labels"))) {
                for x in l
                    .get("labels")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if x.get("type").and_then(Value::as_str) != Some("user") {
                        continue;
                    }
                    if let (Some(id), Some(name)) = (
                        x.get("id").and_then(Value::as_str),
                        x.get("name").and_then(Value::as_str),
                    ) {
                        out.push(SourceCandidate {
                            kind: SourceKind::GmailLabel,
                            external_id: id.into(),
                            name: name.into(),
                            enabled: false,
                            private: false,
                            member_count: None,
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    fn sync(&self, cx: &SyncCx<'_>, source: &AccountSource) -> Result<SyncOutcome> {
        let mut out = SyncOutcome::default();
        let kind = SourceKind::parse(&source.kind);
        if kind == Some(SourceKind::GmailLabel) {
            // The account-level history covers labels; a newly watched label
            // backfills once (its `cursor` remembers it).
            if source.cursor.is_none() {
                let req = HttpRequest::get(format!("{API}/messages")).query(&[
                    ("labelIds", source.external_id.as_str()),
                    (
                        "q",
                        &format!("newer_than:{}d", cx.settings.gmail_initial_sync_days),
                    ),
                    ("maxResults", "100"),
                ]);
                let page = ok_json(&cx.api, req)?;
                let watched = self.watched_labels(cx)?;
                let inbox = self.inbox_source_id(cx)?;
                let mut msgs = Vec::new();
                for m in page
                    .get("messages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let Some(id) = m.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    match self.fetch(cx, id) {
                        Ok(Some(full)) => msgs.extend(Self::to_new(
                            cx.account,
                            &full,
                            inbox.as_deref(),
                            &watched,
                            false,
                        )),
                        Ok(None) => {}
                        Err(ConnectorError::BudgetExhausted) => {
                            out.more = true;
                            break;
                        }
                        Err(e) => return Err(e),
                    }
                }
                self.ingest(cx, msgs, &mut out)?;
                if !out.more {
                    let (sid, now) = (source.id.clone(), cx.now_str());
                    cx.db.write(move |c| {
                        c.execute(
                            "UPDATE account_sources SET cursor = 'backfilled', updated_at = ?2 WHERE id = ?1",
                            rusqlite::params![sid, now],
                        )?;
                        Ok(())
                    })?;
                }
            }
            out.requests = cx.api.used();
            return Ok(out);
        }
        let cursor = read_cursor(cx)?;
        let backfill: Option<Backfill> = cursor
            .get("gmail_backfill")
            .and_then(|b| serde_json::from_value(b.clone()).ok());
        let history = cursor
            .get("gmail_history_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let r = match (backfill, history) {
            (Some(bf), _) => self.backfill(cx, bf, &mut out),
            (None, None) => self.backfill(cx, Backfill::default(), &mut out),
            (None, Some(h)) => match self.history(cx, &h, &mut out) {
                Err(ConnectorError::CursorGone) => {
                    // Google keeps roughly a week: resync the 14-day window,
                    // deduplicated by external id, local state kept.
                    write_cursor(cx, |v| {
                        if let Some(o) = v.as_object_mut() {
                            o.remove("gmail_history_id");
                        }
                    })?;
                    self.backfill(
                        cx,
                        Backfill {
                            resync: true,
                            ..Default::default()
                        },
                        &mut out,
                    )
                }
                other => other,
            },
        };
        out.requests = cx.api.used();
        match r {
            Ok(()) => Ok(out),
            Err(ConnectorError::BudgetExhausted) => {
                out.more = true;
                Ok(out)
            }
            Err(e) => Err(e),
        }
    }

    fn fetch_body(&self, cx: &SyncCx<'_>, message: &Message) -> Result<FetchedBody> {
        let v = ok_json(
            &cx.api,
            HttpRequest::get(format!("{API}/messages/{}", message.external_id))
                .query(&[("format", "full")]),
        )?;
        let payload = v.get("payload").cloned().unwrap_or(Value::Null);
        let (body, _) = mime::body_text(&payload);
        let text = body.unwrap_or_else(|| {
            mime::decode_entities(v.get("snippet").and_then(Value::as_str).unwrap_or(""))
        });
        let (t, cut) = super::cap_body(&text, super::BODY_CAP_BYTES);
        Ok(FetchedBody {
            text: t,
            truncated: cut,
        })
    }

    fn write_back(&self, cx: &SyncCx<'_>, item: &ConnectorWriteback) -> Result<()> {
        if !self.capabilities(cx.account).mark_read_writeback {
            return Err(ConnectorError::Unsupported);
        }
        let (add, remove): (Vec<&str>, Vec<&str>) = match item.action.as_str() {
            "mark_read" => (vec![], vec!["UNREAD"]),
            "mark_unread" => (vec!["UNREAD"], vec![]),
            "archive" => (vec![], vec!["INBOX"]),
            "unarchive" => (vec!["INBOX"], vec![]),
            _ => return Err(ConnectorError::Unsupported),
        };
        let path = if item.target_kind == "thread" {
            "threads"
        } else {
            "messages"
        };
        ok_json(
            &cx.api,
            HttpRequest::post(format!("{API}/{path}/{}/modify", item.external_id))
                .json(&json!({ "addLabelIds": add, "removeLabelIds": remove })),
        )?;
        Ok(())
    }

    fn source_url(&self, account: &Account, message: &Message) -> String {
        if !message.source_url.is_empty() {
            return message.source_url.clone();
        }
        let thread = message.thread_id.clone().unwrap_or_default();
        source_url(account.email.as_deref().unwrap_or(""), &thread)
    }
}

/// The label id → name map cached at initial sync.
pub fn label_names(account: &Account) -> BTreeMap<String, String> {
    serde_json::from_str::<Value>(&account.cursor_json)
        .ok()
        .and_then(|v| v.get("gmail_labels").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}
