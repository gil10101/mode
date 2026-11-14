//! The Slack adapter (06 §8): one user-created app per workspace from the
//! manifest (§8.1, [`manifest`]); user scopes only (§8.2); connect through
//! `auth.test` + `users.info` + `users.conversations` (§8.3); the channel
//! picker (§8.4); the polling plan (§8.5: `conversations.history` per DM,
//! group DM and watched channel from the latest `ts`, followed threads via
//! `conversations.replies`, the `search.messages` mention net); mention
//! detection (§8.6); the storage mapping (§8.7: `<channel>:<ts>` ids, text
//! with mentions / channels / links resolved, `last_read` mirroring);
//! `conversations.mark` write-back (§8.8); token errors (§8.9) and 429
//! handling (§8.10).

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::http::{HttpRequest, HttpResponse};
use super::store::{self, NewMessage, NewThread, Sender};
use super::{
    Api, Capabilities, ConnectorAdapter, ConnectorError, FetchedBody, Identity, Provider, Result,
    SourceCandidate, SourceKind, SyncCx, SyncOutcome, oauth,
};
use crate::repo::{Account, AccountSource, ConnectorWriteback, Message};
use crate::urgency;

pub const API: &str = "https://slack.com/api";
/// At most this many followed threads per poll (06 §8.5).
pub const THREADS_PER_POLL: usize = 5;
/// The backfill cap per source (06 §8.4).
pub const BACKFILL_MAX: usize = 200;
/// Housekeeping subtypes never stored (06 §8.7).
pub const SKIPPED_SUBTYPES: &[&str] = &[
    "channel_join",
    "channel_leave",
    "channel_topic",
    "channel_purpose",
    "channel_name",
    "channel_archive",
    "channel_unarchive",
    "group_join",
    "group_leave",
    "group_topic",
    "group_purpose",
    "pinned_item",
    "unpinned_item",
    "reminder_add",
    "message_changed",
    "message_deleted",
];

/// The app manifest of 06 §8.1 with the loopback ports filled in; without
/// write-back the `*:write` scopes are left out.
pub fn manifest(ports: &[u16], writeback: bool) -> String {
    let mut s = String::from(
        "display_information:\n  name: Mode\n  description: Mode reads your DMs, mentions and chosen channels on this device. It never posts.\noauth_config:\n  redirect_urls:\n",
    );
    for p in ports {
        s.push_str(&format!("    - http://127.0.0.1:{p}/callback\n"));
    }
    s.push_str("  scopes:\n    user:\n");
    for sc in oauth::SLACK_READ_SCOPES {
        s.push_str(&format!("      - {sc}\n"));
    }
    if writeback {
        for sc in oauth::SLACK_WRITE_SCOPES {
            s.push_str(&format!("      - {sc}\n"));
        }
    }
    s.push_str("settings:\n  org_deploy_enabled: false\n  socket_mode_enabled: false\n  token_rotation_enabled: true\n");
    s
}

/// Slack's capability flags from the granted user scopes (06 §2.2).
pub fn capabilities(scopes: &[String]) -> Capabilities {
    let write = scopes.iter().any(|s| s.ends_with(":write"));
    Capabilities {
        read_messages: true,
        read_events: false,
        fetch_body_on_demand: true,
        mark_read_writeback: write,
        mark_unread_writeback: false,
        archive_writeback: false,
        thread_actions: if write {
            "mark_read".into()
        } else {
            "none".into()
        },
        deep_link: "slack".into(),
        incremental_cursor: "latest_ts".into(),
    }
}

/// `1729500000.000100` → `p1729500000000100` (the web archive link form).
pub fn ts_path(ts: &str) -> String {
    format!("p{}", ts.replace('.', ""))
}

/// The deep link (`slack://…`) and the web fallback (06 §8.8).
pub fn links(
    team_id: &str,
    team_url: Option<&str>,
    channel: &str,
    ts: &str,
    thread_ts: Option<&str>,
) -> (String, String) {
    let mut deep = format!("slack://channel?team={team_id}&id={channel}&message={ts}");
    if let Some(t) = thread_ts.filter(|t| *t != ts) {
        deep.push_str(&format!("&thread_ts={t}"));
    }
    let base = team_url
        .map(|u| u.trim_end_matches('/').to_string())
        .unwrap_or_else(|| "https://app.slack.com".into());
    let mut web = format!("{base}/archives/{channel}/{}", ts_path(ts));
    if let Some(t) = thread_ts.filter(|t| *t != ts) {
        web.push_str(&format!("?thread_ts={t}&cid={channel}"));
    }
    (deep, web)
}

/// A Slack response → its JSON or the mapped error (06 §8.9, §8.10).
pub fn slack_json(resp: &HttpResponse) -> Result<Value> {
    if resp.status == 429 {
        return Err(ConnectorError::RateLimited {
            retry_after_secs: resp.retry_after_secs().unwrap_or(60),
        });
    }
    if !resp.ok() {
        return Err(super::classify_status(resp));
    }
    let v = resp.json_value();
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(v);
    }
    let code = v.get("error").and_then(Value::as_str).unwrap_or("unknown");
    Err(match code {
        "missing_scope" => ConnectorError::MissingScope(
            v.get("needed")
                .and_then(Value::as_str)
                .unwrap_or("scope")
                .to_string(),
        ),
        "channel_not_found" | "not_in_channel" | "message_not_found" | "thread_not_found" => {
            ConnectorError::NotFound
        }
        "ratelimited" => ConnectorError::RateLimited {
            retry_after_secs: resp.retry_after_secs().unwrap_or(60),
        },
        other => oauth::slack_auth_error(other),
    })
}

fn call(api: &Api<'_>, method: &str, params: &[(&str, &str)]) -> Result<Value> {
    let resp = api.send(HttpRequest::get(format!("{API}/{method}")).query(params))?;
    slack_json(&resp)
}

/// Resolve Slack's wire markup for display (06 §8.7): `<@U…>` → `@name`,
/// `<#C…|name>` → `#name`, `<url|label>` → `label`, `<!here>` → `@here`,
/// `&amp;` / `&lt;` / `&gt;`.
pub fn resolve_text(text: &str, name_of: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let Some(end) = tail.find('>') else {
            out.push('<');
            rest = tail;
            continue;
        };
        let inner = &tail[..end];
        let (target, label) = inner.split_once('|').unwrap_or((inner, ""));
        let rendered = if let Some(u) = target.strip_prefix('@') {
            if label.is_empty() {
                format!("@{}", name_of(u).unwrap_or_else(|| u.to_string()))
            } else {
                format!("@{label}")
            }
        } else if let Some(c) = target.strip_prefix('#') {
            if label.is_empty() {
                format!("#{c}")
            } else {
                format!("#{label}")
            }
        } else if let Some(b) = target.strip_prefix('!') {
            let word = b.split('^').next().unwrap_or(b);
            match word {
                "here" | "channel" | "everyone" => format!("@{word}"),
                "subteam" => {
                    if label.is_empty() {
                        "@group".into()
                    } else {
                        label.to_string()
                    }
                }
                _ => {
                    if label.is_empty() {
                        word.to_string()
                    } else {
                        label.to_string()
                    }
                }
            }
        } else if label.is_empty() {
            target.to_string()
        } else {
            label.to_string()
        };
        out.push_str(&rendered);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Whether the wire text carries a broadcast (`<!here>` …, 06 §8.6).
pub fn is_broadcast(text: &str) -> bool {
    text.contains("<!here")
        || text.contains("<!channel")
        || text.contains("<!everyone")
        || text.contains("<!subteam^")
}

/// The Slack adapter. Holds the `users.info` cache (refreshed every 24 h
/// by the service clearing it).
#[derive(Debug, Default)]
pub struct SlackAdapter {
    users: Mutex<HashMap<String, (String, Option<String>)>>,
}

impl SlackAdapter {
    pub fn new() -> SlackAdapter {
        SlackAdapter::default()
    }

    pub fn clear_cache(&self) {
        self.users.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// `(display name, image_72)` for a user id (cached; `users.info`).
    fn user(&self, api: &Api<'_>, user: &str) -> (String, Option<String>) {
        if let Some(u) = self
            .users
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(user)
            .cloned()
        {
            return u;
        }
        let got = call(api, "users.info", &[("user", user)]).ok().map(|v| {
            let p = v.pointer("/user/profile");
            let name = p
                .and_then(|p| p.get("display_name"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| p.and_then(|p| p.get("real_name")).and_then(Value::as_str))
                .or_else(|| v.pointer("/user/real_name").and_then(Value::as_str))
                .or_else(|| v.pointer("/user/name").and_then(Value::as_str))
                .unwrap_or(user)
                .to_string();
            let avatar = p
                .and_then(|p| p.get("image_72"))
                .and_then(Value::as_str)
                .map(str::to_string);
            (name, avatar)
        });
        match got {
            Some(u) => {
                self.users
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(user.to_string(), u.clone());
                u
            }
            None => (user.to_string(), None),
        }
    }

    fn cached_name(&self, user: &str) -> Option<String> {
        self.users
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(user)
            .map(|u| u.0.clone())
    }

    fn team_url(account: &Account) -> Option<String> {
        serde_json::from_str::<Value>(&account.cursor_json)
            .ok()?
            .get("team_url")?
            .as_str()
            .map(str::to_string)
    }

    /// One wire message → a stored message (or `None` for housekeeping).
    #[allow(clippy::too_many_arguments)]
    pub fn to_new(
        &self,
        api: &Api<'_>,
        account: &Account,
        source: Option<&AccountSource>,
        channel_id: &str,
        channel_name: &str,
        conv_kind: &str,
        m: &Value,
        last_read: Option<&str>,
        via_search: bool,
    ) -> Option<NewMessage> {
        let s = |k: &str| m.get(k).and_then(Value::as_str);
        let ts = s("ts")?;
        if s("subtype").is_some_and(|st| SKIPPED_SUBTYPES.contains(&st)) {
            return None;
        }
        let me = account.handle.clone().unwrap_or_default();
        let team = account.team_id.clone().unwrap_or_default();
        let bot = s("bot_id").is_some() || s("subtype") == Some("bot_message");
        let user = s("user").or(s("bot_id")).unwrap_or("unknown").to_string();
        let (sender_name, avatar) = if bot {
            (
                s("username")
                    .map(str::to_string)
                    .or_else(|| {
                        m.pointer("/bot_profile/name")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "bot".into()),
                m.pointer("/bot_profile/icons/image_72")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )
        } else {
            self.user(api, &user)
        };
        let raw = s("text").unwrap_or("");
        // warm the cache for mentions so `<@U…>` resolves to names
        for part in raw.split("<@").skip(1) {
            if let Some(id) = part.split(['>', '|']).next()
                && self.cached_name(id).is_none()
            {
                let _ = self.user(api, id);
            }
        }
        let text = resolve_text(raw, &|u| self.cached_name(u));
        let files: Vec<String> = m
            .get("files")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let has_att = !files.is_empty()
            || m.get("attachments")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());
        let shown = if text.trim().is_empty() && !files.is_empty() {
            files.join(", ")
        } else {
            text.clone()
        };
        let mut labels: Vec<String> = Vec::new();
        if bot {
            labels.push("bot".into());
        }
        if is_broadcast(raw) {
            labels.push("broadcast".into());
        }
        if has_att {
            labels.push("attachments".into());
        }
        if let Some(st) = s("subtype") {
            labels.push(format!("subtype:{st}"));
        }
        for r in m
            .get("reactions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(n) = r.get("name").and_then(Value::as_str) {
                labels.push(format!("reaction:{n}"));
            }
        }
        let from_self = !me.is_empty() && user == me;
        let mentioned = !me.is_empty() && raw.contains(&format!("<@{me}"));
        let is_dm = matches!(conv_kind, "dm" | "group_dm");
        let is_direct = !from_self && (is_dm || mentioned || via_search);
        let thread_ts = s("thread_ts").map(str::to_string).or_else(|| {
            // search results carry it only in the permalink
            s("permalink")
                .and_then(|p| url::Url::parse(p).ok())
                .and_then(|u| {
                    u.query_pairs()
                        .find(|(k, _)| k == "thread_ts")
                        .map(|(_, v)| v.into_owned())
                })
        });
        let is_reply = thread_ts.as_deref().is_some_and(|t| t != ts);
        let (kind, thread_ext) = if is_reply {
            let k = if is_dm { conv_kind } else { "channel_thread" };
            (
                k.to_string(),
                format!("{channel_id}:{}", thread_ts.clone().unwrap_or_default()),
            )
        } else {
            (conv_kind.to_string(), channel_id.to_string())
        };
        let (_, web) = links(
            &team,
            Self::team_url(account).as_deref(),
            channel_id,
            ts,
            thread_ts.as_deref(),
        );
        let is_read = from_self || last_read.is_some_and(|lr| ts_le(ts, lr));
        let received_at = ts
            .parse::<f64>()
            .ok()
            .and_then(super::ts_from_epoch_f64)
            .unwrap_or_else(crate::db::now_rfc3339);
        let (body, truncated) = super::cap_body(&shown, super::BODY_CAP_BYTES);
        Some(NewMessage {
            account_id: account.id.clone(),
            source_id: source.map(|s| s.id.clone()),
            provider: Provider::Slack,
            external_id: format!("{channel_id}:{ts}"),
            rfc_message_id: None,
            dedup_key: urgency::dedup::slack_key(&team, channel_id, ts),
            kind,
            thread: NewThread {
                external_id: thread_ext,
                kind: if is_reply && !is_dm {
                    "channel_thread".into()
                } else {
                    conv_kind.to_string()
                },
                title: channel_name.to_string(),
                channel_id: Some(channel_id.to_string()),
                channel_name: Some(channel_name.to_string()),
                participants: json!([]),
                source_url: Some(web.clone()),
            },
            sender: Sender {
                name: sender_name,
                address: user,
                avatar_url: avatar,
                identity_kind: "slack_user".into(),
                identity_scope: team.clone(),
            },
            channel_id: Some(channel_id.to_string()),
            channel_name: Some(channel_name.to_string()),
            subject: None,
            snippet: super::snippet_of(&shown),
            body_text: Some(body),
            body_truncated: truncated,
            has_attachments: has_att,
            labels,
            is_direct,
            is_read: Some(is_read),
            is_archived: None,
            state_changed: last_read.is_some(),
            received_at,
            source_url: web,
        })
    }

    fn ingest(&self, cx: &SyncCx<'_>, msgs: Vec<NewMessage>, out: &mut SyncOutcome) -> Result<()> {
        if msgs.is_empty() {
            return Ok(());
        }
        if let Some(latest) = msgs.iter().map(|m| m.received_at.clone()).max()
            && out.latest_message_at.as_ref().is_none_or(|l| latest > *l)
        {
            out.latest_message_at = Some(latest);
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

    fn set_source_cursor(&self, cx: &SyncCx<'_>, source_id: &str, ts: &str) -> Result<()> {
        let (sid, ts, now) = (source_id.to_string(), ts.to_string(), cx.now_str());
        cx.db.write(move |c| {
            c.execute(
                "UPDATE account_sources SET cursor = ?2, updated_at = ?3 WHERE id = ?1
                   AND (cursor IS NULL OR CAST(cursor AS REAL) < CAST(?2 AS REAL))",
                rusqlite::params![sid, ts, now],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    fn conv_kind(source: &AccountSource) -> &'static str {
        match SourceKind::parse(&source.kind) {
            Some(SourceKind::SlackDm) => "dm",
            Some(SourceKind::SlackMpim) => "group_dm",
            _ => "channel",
        }
    }

    /// One conversation: history from the cursor, `last_read`, followed threads.
    fn sync_conversation(
        &self,
        cx: &SyncCx<'_>,
        source: &AccountSource,
        out: &mut SyncOutcome,
    ) -> Result<()> {
        let oldest = match &source.cursor {
            Some(c) if !c.is_empty() => c.clone(),
            _ => {
                out.full_sync = true;
                let back =
                    cx.now - time::Duration::days(cx.settings.slack_initial_sync_days.max(1));
                format!("{}.000000", back.unix_timestamp())
            }
        };
        let v = call(
            &cx.api,
            "conversations.history",
            &[
                ("channel", source.external_id.as_str()),
                ("oldest", oldest.as_str()),
                ("limit", "200"),
                ("inclusive", "false"),
            ],
        )?;
        let msgs: Vec<Value> = v
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if msgs.len() == 15 && v.get("has_more").and_then(Value::as_bool) == Some(true) {
            tracing::warn!(
                target: "mode::connectors",
                "slack: history clamped to 15 messages; the app was not created from the manifest"
            );
        }
        let last_read = if msgs.is_empty() {
            None
        } else {
            call(
                &cx.api,
                "conversations.info",
                &[("channel", source.external_id.as_str())],
            )
            .ok()
            .and_then(|i| {
                i.pointer("/channel/last_read")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        };
        let kind = Self::conv_kind(source);
        let mut newest = oldest.clone();
        let mut batch = Vec::new();
        let mut threads: Vec<(String, String)> = Vec::new();
        for m in msgs.iter().take(BACKFILL_MAX) {
            if let Some(ts) = m.get("ts").and_then(Value::as_str)
                && !ts_le(ts, &newest)
            {
                newest = ts.to_string();
            }
            if m.get("subscribed").and_then(Value::as_bool) == Some(true)
                && let (Some(ts), Some(latest)) = (
                    m.get("ts").and_then(Value::as_str),
                    m.get("latest_reply").and_then(Value::as_str),
                )
            {
                threads.push((ts.to_string(), latest.to_string()));
            }
            if let Some(n) = self.to_new(
                &cx.api,
                cx.account,
                Some(source),
                &source.external_id,
                &source.name,
                kind,
                m,
                last_read.as_deref(),
                false,
            ) {
                batch.push(n);
            }
        }
        // Followed threads with new replies (≤ 5 per poll).
        let thread_cursors = self.thread_cursors(cx)?;
        let mut followed = 0;
        for (parent, latest) in threads {
            if followed >= THREADS_PER_POLL {
                break;
            }
            let key = format!("{}:{parent}", source.external_id);
            let since = thread_cursors
                .get(&key)
                .cloned()
                .unwrap_or_else(|| parent.clone());
            if ts_le(&latest, &since) {
                continue;
            }
            followed += 1;
            match call(
                &cx.api,
                "conversations.replies",
                &[
                    ("channel", source.external_id.as_str()),
                    ("ts", parent.as_str()),
                    ("oldest", since.as_str()),
                ],
            ) {
                Ok(r) => {
                    for m in r
                        .get("messages")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if m.get("ts").and_then(Value::as_str) == Some(parent.as_str()) {
                            continue;
                        }
                        if let Some(n) = self.to_new(
                            &cx.api,
                            cx.account,
                            Some(source),
                            &source.external_id,
                            &source.name,
                            kind,
                            m,
                            last_read.as_deref(),
                            false,
                        ) {
                            batch.push(n);
                        }
                    }
                    self.set_thread_cursor(cx, &key, &latest)?;
                }
                Err(ConnectorError::NotFound) => {}
                Err(e) => {
                    self.ingest(cx, batch, out)?;
                    return Err(e);
                }
            }
        }
        // Mirror reading done in the Slack app for rows already stored.
        if let Some(lr) = &last_read {
            let (acc, ch, lr, now) = (
                cx.account.id.clone(),
                source.external_id.clone(),
                lr.clone(),
                cx.now_str(),
            );
            out.updated += cx.db.write(move |c| {
                crate::repo::tx(c, |tx| {
                    let ids: Vec<String> = {
                        let mut st = tx.prepare(
                            "SELECT id, external_id FROM messages WHERE account_id = ?1 AND channel_id = ?2 AND is_read = 0",
                        )?;
                        st.query_map(rusqlite::params![acc, ch], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                        .into_iter()
                        .filter(|(_, ext)| ext.rsplit(':').next().is_some_and(|ts| ts_le(ts, &lr)))
                        .map(|(id, _)| id)
                        .collect()
                    };
                    let mut n = 0;
                    for id in &ids {
                        if store::apply_provider_state(tx, id, Some(true), None, true, &now)? {
                            n += 1;
                        }
                    }
                    let threads: Vec<String> = {
                        let mut st = tx.prepare(
                            "SELECT DISTINCT thread_id FROM messages WHERE account_id = ?1 AND channel_id = ?2 AND thread_id IS NOT NULL",
                        )?;
                        st.query_map(rusqlite::params![acc, ch], |r| r.get(0))?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    urgency::rank::recompute_threads(tx, &threads, &now)?;
                    Ok(n)
                })
            })?;
        }
        self.ingest(cx, batch, out)?;
        if newest != oldest {
            self.set_source_cursor(cx, &source.id, &newest)?;
        }
        Ok(())
    }

    fn thread_cursors(&self, cx: &SyncCx<'_>) -> Result<HashMap<String, String>> {
        let id = cx.account.id.clone();
        let raw: String = cx.db.read(move |c| {
            Ok(c.query_row(
                "SELECT cursor_json FROM accounts WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )?)
        })?;
        Ok(serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v.get("slack_threads").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    fn set_thread_cursor(&self, cx: &SyncCx<'_>, key: &str, ts: &str) -> Result<()> {
        let (id, key, ts, now) = (
            cx.account.id.clone(),
            key.to_string(),
            ts.to_string(),
            cx.now_str(),
        );
        cx.db.write(move |c| {
            let raw: String = c.query_row(
                "SELECT cursor_json FROM accounts WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )?;
            let mut v: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
            if !v.get("slack_threads").is_some_and(Value::is_object) {
                v["slack_threads"] = json!({});
            }
            v["slack_threads"][&key] = json!(ts);
            c.execute(
                "UPDATE accounts SET cursor_json = ?2, updated_at = ?3 WHERE id = ?1",
                rusqlite::params![id, v.to_string(), now],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// The `search.messages` mention net (06 §8.5, §8.6).
    fn sync_mentions(
        &self,
        cx: &SyncCx<'_>,
        source: &AccountSource,
        out: &mut SyncOutcome,
    ) -> Result<()> {
        let me = cx.account.handle.clone().unwrap_or_default();
        if me.is_empty() {
            return Ok(());
        }
        let query = format!("<@{me}>");
        let v = call(
            &cx.api,
            "search.messages",
            &[
                ("query", query.as_str()),
                ("sort", "timestamp"),
                ("sort_dir", "desc"),
                ("count", "20"),
            ],
        )?;
        let cursor = source.cursor.clone().unwrap_or_default();
        let mut newest = cursor.clone();
        let mut batch = Vec::new();
        for m in v
            .pointer("/messages/matches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(ts) = m.get("ts").and_then(Value::as_str) else {
                continue;
            };
            if !cursor.is_empty() && ts_le(ts, &cursor) {
                continue;
            }
            if newest.is_empty() || !ts_le(ts, &newest) {
                newest = ts.to_string();
            }
            let ch = m
                .pointer("/channel/id")
                .and_then(Value::as_str)
                .unwrap_or("");
            if ch.is_empty() {
                continue;
            }
            let is_im = m.pointer("/channel/is_im").and_then(Value::as_bool) == Some(true);
            let is_mpim = m.pointer("/channel/is_mpim").and_then(Value::as_bool) == Some(true);
            let kind = if is_im {
                "dm"
            } else if is_mpim {
                "group_dm"
            } else {
                "channel"
            };
            let name = m
                .pointer("/channel/name")
                .and_then(Value::as_str)
                .map(|n| {
                    if kind == "channel" {
                        format!("#{n}")
                    } else {
                        n.to_string()
                    }
                })
                .unwrap_or_else(|| ch.to_string());
            if let Some(n) = self.to_new(
                &cx.api,
                cx.account,
                Some(source),
                ch,
                &name,
                kind,
                m,
                None,
                true,
            ) {
                batch.push(n);
            }
        }
        self.ingest(cx, batch, out)?;
        if !newest.is_empty() && newest != cursor {
            self.set_source_cursor(cx, &source.id, &newest)?;
        }
        Ok(())
    }

    /// `users.conversations` for `types` (paginated).
    pub fn conversations(&self, api: &Api<'_>, types: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut params = vec![
                ("types", types),
                ("exclude_archived", "true"),
                ("limit", "200"),
            ];
            if !cursor.is_empty() {
                params.push(("cursor", cursor.as_str()));
            }
            let v = call(api, "users.conversations", &params)?;
            out.extend(
                v.get("channels")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            cursor = v
                .pointer("/response_metadata/next_cursor")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if cursor.is_empty() {
                break;
            }
        }
        Ok(out)
    }

    /// The channel picker list (06 §8.4): public and private channels the
    /// user is a member of.
    pub fn channels(&self, api: &Api<'_>) -> Result<Vec<SourceCandidate>> {
        Ok(self
            .conversations(api, "public_channel,private_channel")?
            .iter()
            .filter_map(|c| {
                Some(SourceCandidate {
                    kind: SourceKind::SlackChannel,
                    external_id: c.get("id")?.as_str()?.to_string(),
                    name: format!("#{}", c.get("name").and_then(Value::as_str).unwrap_or("")),
                    enabled: false,
                    private: c
                        .get("is_private")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    member_count: c.get("num_members").and_then(Value::as_i64),
                })
            })
            .collect())
    }

    /// DMs, group DMs and the mention net (06 §3.4), seeded at connect and
    /// re-listed every 10 minutes so new DMs appear.
    pub fn direct_sources(&self, api: &Api<'_>) -> Result<Vec<SourceCandidate>> {
        let mut out = Vec::new();
        for c in self.conversations(api, "im,mpim")? {
            let Some(id) = c.get("id").and_then(Value::as_str) else {
                continue;
            };
            let is_im = c.get("is_im").and_then(Value::as_bool).unwrap_or(false);
            let name = if is_im {
                let u = c.get("user").and_then(Value::as_str).unwrap_or("");
                format!("@{}", self.user(api, u).0)
            } else {
                mpim_name(c.get("name").and_then(Value::as_str).unwrap_or(id))
            };
            out.push(SourceCandidate {
                kind: if is_im {
                    SourceKind::SlackDm
                } else {
                    SourceKind::SlackMpim
                },
                external_id: id.into(),
                name,
                enabled: true,
                private: true,
                member_count: None,
            });
        }
        out.push(SourceCandidate {
            kind: SourceKind::SlackMentions,
            external_id: "*".into(),
            name: "Mentions".into(),
            enabled: true,
            private: false,
            member_count: None,
        });
        Ok(out)
    }
}

/// `mpdm-ana--tom--me-1` → `ana, tom, me`.
pub fn mpim_name(raw: &str) -> String {
    let s = raw.strip_prefix("mpdm-").unwrap_or(raw);
    let s = s.rsplit_once('-').map(|(a, _)| a).unwrap_or(s);
    s.split("--").collect::<Vec<_>>().join(", ")
}

/// `a <= b` for Slack timestamps (`seconds.micros`).
pub fn ts_le(a: &str, b: &str) -> bool {
    let parse = |t: &str| -> (i64, i64) {
        let (s, f) = t.split_once('.').unwrap_or((t, "0"));
        let f = format!("{f:0<6}");
        (s.parse().unwrap_or(0), f[..6].parse().unwrap_or(0))
    };
    parse(a) <= parse(b)
}

impl ConnectorAdapter for SlackAdapter {
    fn provider(&self) -> Provider {
        Provider::Slack
    }

    fn capabilities(&self, account: &Account) -> Capabilities {
        capabilities(&super::json_strings(&account.scopes_json))
    }

    fn identify(&self, api: &Api<'_>, granted: &[String]) -> Result<Identity> {
        let t = slack_json(
            &api.send(HttpRequest::post(format!("{API}/auth.test")).form::<&str, &str>(&[]))?,
        )?;
        let user = t
            .get("user_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ConnectorError::Protocol("auth.test without user_id".into()))?;
        let team = t
            .get("team_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ConnectorError::Protocol("auth.test without team_id".into()))?;
        let team_name = t.get("team").and_then(Value::as_str).unwrap_or(team);
        let (name, avatar) = self.user(api, user);
        Ok(Identity {
            external_id: format!("{team}:{user}"),
            display_name: format!("{team_name} · @{name}"),
            email: None,
            handle: Some(user.to_string()),
            team_id: Some(team.to_string()),
            team_name: Some(team_name.to_string()),
            team_url: t.get("url").and_then(Value::as_str).map(str::to_string),
            avatar_url: avatar,
            services: vec!["slack".into()],
            scopes: granted.to_vec(),
        })
    }

    fn list_sources(&self, api: &Api<'_>, _account: &Account) -> Result<Vec<SourceCandidate>> {
        self.direct_sources(api)
    }

    fn sync(&self, cx: &SyncCx<'_>, source: &AccountSource) -> Result<SyncOutcome> {
        let mut out = SyncOutcome::default();
        let r = match SourceKind::parse(&source.kind) {
            Some(SourceKind::SlackMentions) => self.sync_mentions(cx, source, &mut out),
            Some(SourceKind::SlackDm | SourceKind::SlackMpim | SourceKind::SlackChannel) => {
                self.sync_conversation(cx, source, &mut out)
            }
            _ => Ok(()),
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
        let (channel, ts) = message
            .external_id
            .split_once(':')
            .ok_or(ConnectorError::NotFound)?;
        let v = call(
            &cx.api,
            "conversations.history",
            &[
                ("channel", channel),
                ("latest", ts),
                ("oldest", ts),
                ("inclusive", "true"),
                ("limit", "1"),
            ],
        )?;
        let m = v.pointer("/messages/0").ok_or(ConnectorError::NotFound)?;
        let text = resolve_text(m.get("text").and_then(Value::as_str).unwrap_or(""), &|u| {
            self.cached_name(u)
        });
        let (t, cut) = super::cap_body(&text, super::BODY_CAP_BYTES);
        Ok(FetchedBody {
            text: t,
            truncated: cut,
        })
    }

    fn write_back(&self, cx: &SyncCx<'_>, item: &ConnectorWriteback) -> Result<()> {
        if item.action != "mark_read" || !self.capabilities(cx.account).mark_read_writeback {
            return Err(ConnectorError::Unsupported);
        }
        let (channel, ts) = item
            .external_id
            .split_once(':')
            .ok_or(ConnectorError::NotFound)?;
        slack_json(
            &cx.api.send(
                HttpRequest::post(format!("{API}/conversations.mark"))
                    .form(&[("channel", channel), ("ts", ts)]),
            )?,
        )?;
        Ok(())
    }

    fn source_url(&self, account: &Account, message: &Message) -> String {
        let (channel, ts) = message.external_id.split_once(':').unwrap_or(("", ""));
        let (deep, _) = links(
            account.team_id.as_deref().unwrap_or(""),
            Self::team_url(account).as_deref(),
            channel,
            ts,
            None,
        );
        deep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_resolves() {
        let names = |u: &str| (u == "U1").then(|| "ana".to_string());
        assert_eq!(
            resolve_text(
                "hi <@U1> see <#C9|release> and <https://x.y|the doc> <!here> &amp; &lt;3",
                &names
            ),
            "hi @ana see #release and the doc @here & <3"
        );
        assert_eq!(
            resolve_text("<@U2> <https://a.b>", &names),
            "@U2 https://a.b"
        );
        assert!(is_broadcast("<!channel> deploy"));
        assert!(!is_broadcast("<@U1> deploy"));
    }

    #[test]
    fn timestamps_and_links() {
        assert!(ts_le("1729500000.000100", "1729500000.000200"));
        assert!(!ts_le("1729500001.0", "1729500000.999999"));
        assert!(ts_le("1729500000.1", "1729500000.100000"));
        let (deep, web) = links(
            "T1",
            Some("https://acme.slack.com/"),
            "C1",
            "1729500000.000100",
            Some("1729400000.000200"),
        );
        assert_eq!(
            deep,
            "slack://channel?team=T1&id=C1&message=1729500000.000100&thread_ts=1729400000.000200"
        );
        assert_eq!(
            web,
            "https://acme.slack.com/archives/C1/p1729500000000100?thread_ts=1729400000.000200&cid=C1"
        );
        assert_eq!(mpim_name("mpdm-ana--tom--me-1"), "ana, tom, me");
    }

    #[test]
    fn manifest_lists_ports_and_scopes() {
        let m = manifest(&[48311, 48312], true);
        assert!(m.contains("http://127.0.0.1:48311/callback"));
        assert!(m.contains("- search:read"));
        assert!(m.contains("- im:write"));
        assert!(m.contains("token_rotation_enabled: true"));
        assert!(!manifest(&[48311], false).contains(":write"));
    }
}
