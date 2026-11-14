//! The Google Calendar adapter (06 §7): same account and consent as Gmail
//! (ADR-0015). Sources are `calendarList` entries (enabled for `primary`
//! and `selected`); events within −30 / +90 days; a full sync
//! (`timeMin` / `timeMax` / `singleEvents` / `orderBy=startTime`, paginated,
//! the final `nextSyncToken` stored in `cursor_json.calendar[<id>]`) at
//! connect, every 24 h and on 410; incremental sync with the `syncToken`
//! only; the field mapping of §7.3 and the join-link extraction of §7.4
//! ([`crate::calendar::join_link`]). Read-only: no write-back exists.

use serde_json::{Value, json};
use time::{Duration, OffsetDateTime, PrimitiveDateTime};

use super::http::{HttpRequest, HttpResponse};
use super::store::{self, NewEvent};
use super::{
    Api, Capabilities, ConnectorAdapter, ConnectorError, FetchedBody, Identity, Provider, Result,
    SourceCandidate, SourceKind, SyncCx, SyncOutcome,
};
use crate::repo::{Account, AccountSource, ConnectorWriteback, Message};

pub const API: &str = "https://www.googleapis.com/calendar/v3";
/// A full sync rolls the window forward every 24 h (06 §7.2).
pub const FULL_EVERY_HOURS: i64 = 24;
pub const WINDOW_BACK_DAYS: i64 = 30;
pub const WINDOW_AHEAD_DAYS: i64 = 90;
pub const MAX_ATTENDEES: usize = 100;
pub const DESCRIPTION_MAX: usize = 2 * 1024;

/// Percent-encode a path segment (calendar ids carry `@` and `#`).
pub fn path_segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn calendar_error(resp: &HttpResponse) -> ConnectorError {
    match resp.status {
        410 => ConnectorError::CursorGone,
        _ => super::gmail::google_error(resp),
    }
}

fn ok_json(api: &Api<'_>, req: HttpRequest) -> Result<Value> {
    let resp = api.send(req)?;
    if !resp.ok() {
        return Err(calendar_error(&resp));
    }
    Ok(resp.json_value())
}

/// Google's `responseStatus` → `my_response`.
pub fn response_word(s: &str) -> Option<&'static str> {
    match s {
        "accepted" => Some("accepted"),
        "declined" => Some("declined"),
        "tentative" => Some("tentative"),
        "needsAction" => Some("needs_action"),
        _ => None,
    }
}

/// Parse an RFC 3339 `dateTime` with an offset into a 09 §4.1 UTC timestamp.
pub fn parse_rfc3339(s: &str) -> Option<OffsetDateTime> {
    let s = s.trim();
    if let Some(t) = crate::db::parse_timestamp(s) {
        if s.ends_with('Z') || s.len() <= 23 {
            return Some(t);
        }
    }
    // 2026-10-22T10:00:00+02:00 / -05:00
    let (body, sign, off) = match s.rfind(['+', '-']) {
        Some(i) if i > 10 => (&s[..i], &s[i..i + 1], &s[i + 1..]),
        _ => return crate::db::parse_timestamp(s),
    };
    let base = crate::db::parse_timestamp(body)?;
    let (h, m) = off.split_once(':').unwrap_or((off, "0"));
    let mins = h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?;
    let delta = Duration::minutes(if sign == "+" { mins } else { -mins });
    Some(base - delta)
}

/// Map one `events.list` item (06 §7.3). `local_midnight` converts an
/// all-day date to the UTC instant of local midnight at sync.
pub fn to_event(
    account_id: &str,
    calendar_id: &str,
    calendar_name: Option<&str>,
    e: &Value,
    local_midnight: &dyn Fn(time::Date) -> OffsetDateTime,
) -> Option<NewEvent> {
    let s = |k: &str| e.get(k).and_then(Value::as_str);
    let id = s("id")?;
    let status = match s("status").unwrap_or("confirmed") {
        "cancelled" => "cancelled",
        "tentative" => "tentative",
        _ => "confirmed",
    };
    let date = |k: &str, f: &str| e.pointer(&format!("/{k}/{f}")).and_then(Value::as_str);
    let (starts_at, ends_at, all_day, start_date, end_date) =
        if let (Some(a), Some(b)) = (date("start", "dateTime"), date("end", "dateTime")) {
            (parse_rfc3339(a)?, parse_rfc3339(b)?, false, None, None)
        } else if let (Some(a), Some(b)) = (date("start", "date"), date("end", "date")) {
            let da = crate::notes::fields::parse_date(a)?;
            let db = crate::notes::fields::parse_date(b)?;
            (
                local_midnight(da),
                local_midnight(db),
                true,
                Some(a.to_string()),
                Some(b.to_string()),
            )
        } else {
            return None;
        };
    let attendees: Vec<Value> = e
        .get("attendees")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(MAX_ATTENDEES)
                .map(|x| {
                    json!({
                        "email": x.get("email").and_then(Value::as_str),
                        "name": x.get("displayName").and_then(Value::as_str),
                        "response": x.get("responseStatus").and_then(Value::as_str).and_then(response_word),
                        "organizer": x.get("organizer").and_then(Value::as_bool).unwrap_or(false),
                        "self": x.get("self").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let my_response = e
        .get("attendees")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .find(|x| x.get("self").and_then(Value::as_bool) == Some(true))
        })
        .and_then(|x| x.get("responseStatus").and_then(Value::as_str))
        .and_then(response_word)
        .map(str::to_string);
    let description = s("description").map(|d| {
        let text = super::gmail::mime::html_to_text(d);
        super::cap_body(&text, DESCRIPTION_MAX).0
    });
    let (join_url, conference) = crate::calendar::join_link(e);
    Some(NewEvent {
        account_id: account_id.into(),
        calendar_id: calendar_id.into(),
        calendar_name: calendar_name.map(str::to_string),
        external_id: id.into(),
        recurring_event_id: s("recurringEventId").map(str::to_string),
        ical_uid: s("iCalUID").map(str::to_string),
        title: s("summary")
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(NO_TITLE)
            .to_string(),
        description,
        location: s("location").map(str::to_string),
        starts_at: crate::db::format_timestamp(starts_at),
        ends_at: crate::db::format_timestamp(ends_at),
        all_day,
        start_date,
        end_date,
        timezone: date("start", "timeZone").map(str::to_string),
        status: status.into(),
        my_response,
        organizer: e
            .pointer("/organizer/email")
            .and_then(Value::as_str)
            .map(str::to_string),
        attendees: Value::Array(attendees),
        join_url,
        conference,
        html_link: s("htmlLink").map(str::to_string),
        etag: s("etag").map(str::to_string),
    })
}

/// An event without a summary (06 §7.3).
pub const NO_TITLE: &str = "(no title)";

/// The Calendar adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct GcalAdapter;

fn cursor_of(cx: &SyncCx<'_>) -> Result<Value> {
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

fn set_cursor(
    cx: &SyncCx<'_>,
    calendar_id: &str,
    token: Option<String>,
    full_at: Option<String>,
) -> Result<()> {
    let (id, cal, now) = (cx.account.id.clone(), calendar_id.to_string(), cx.now_str());
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
        if !v.get("calendar").is_some_and(Value::is_object) {
            v["calendar"] = json!({});
        }
        if !v.get("calendar_full_at").is_some_and(Value::is_object) {
            v["calendar_full_at"] = json!({});
        }
        match token {
            Some(t) => v["calendar"][&cal] = json!(t),
            None => {
                if let Some(o) = v["calendar"].as_object_mut() {
                    o.remove(&cal);
                }
            }
        }
        if let Some(f) = full_at {
            v["calendar_full_at"][&cal] = json!(f);
        }
        c.execute(
            "UPDATE accounts SET cursor_json = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, v.to_string(), now],
        )?;
        Ok(())
    })?;
    Ok(())
}

impl GcalAdapter {
    fn write_events(
        &self,
        cx: &SyncCx<'_>,
        events: Vec<NewEvent>,
        out: &mut SyncOutcome,
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let now = cx.now;
        let n = cx.db.write(move |c| {
            crate::repo::tx(c, |tx| {
                let lo = crate::db::format_timestamp(now - Duration::days(WINDOW_BACK_DAYS));
                let hi = crate::db::format_timestamp(now + Duration::days(WINDOW_AHEAD_DAYS));
                let now_s = crate::db::format_timestamp(now);
                let mut n = 0;
                for e in &events {
                    if e.ends_at < lo || e.starts_at > hi {
                        continue;
                    }
                    if store::upsert_event(tx, e, &now_s)? {
                        n += 1;
                    }
                }
                Ok(n)
            })
        })?;
        out.events_changed += n;
        Ok(())
    }

    fn page_events(
        &self,
        cx: &SyncCx<'_>,
        source: &AccountSource,
        v: &Value,
        midnight: &dyn Fn(time::Date) -> OffsetDateTime,
    ) -> Vec<NewEvent> {
        v.get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                to_event(
                    &cx.account.id,
                    &source.external_id,
                    Some(&source.name),
                    e,
                    midnight,
                )
            })
            .collect()
    }

    fn sync_calendar(
        &self,
        cx: &SyncCx<'_>,
        source: &AccountSource,
        out: &mut SyncOutcome,
    ) -> Result<()> {
        let cursor = cursor_of(cx)?;
        let token = cursor
            .get("calendar")
            .and_then(|m| m.get(&source.external_id))
            .and_then(Value::as_str)
            .map(str::to_string);
        let full_at = cursor
            .get("calendar_full_at")
            .and_then(|m| m.get(&source.external_id))
            .and_then(Value::as_str)
            .and_then(crate::db::parse_timestamp);
        let due_full = token.is_none()
            || full_at.is_none_or(|t| cx.now - t >= Duration::hours(FULL_EVERY_HOURS));
        let tz = cx.tz;
        let db = cx.db;
        let midnight = move |d: time::Date| -> OffsetDateTime {
            db.read(|c| {
                Ok(crate::urgency::with_tz(c, tz, |z| {
                    z.to_utc(PrimitiveDateTime::new(d, time::Time::MIDNIGHT))
                }))
            })
            .unwrap_or_else(|_| PrimitiveDateTime::new(d, time::Time::MIDNIGHT).assume_utc())
        };
        let base = format!(
            "{API}/calendars/{}/events",
            path_segment(&source.external_id)
        );
        if !due_full {
            let mut page: Option<String> = None;
            let mut next_token: Option<String> = None;
            loop {
                let mut req = HttpRequest::get(&base).query(&[
                    ("syncToken", token.as_deref().unwrap_or("")),
                    ("singleEvents", "true"),
                ]);
                if let Some(p) = &page {
                    req = req.query(&[("pageToken", p.as_str())]);
                }
                let v = match ok_json(&cx.api, req) {
                    Ok(v) => v,
                    Err(ConnectorError::CursorGone) => {
                        set_cursor(cx, &source.external_id, None, None)?;
                        return self.full(cx, source, &base, &midnight, out);
                    }
                    Err(e) => return Err(e),
                };
                let events = self.page_events(cx, source, &v, &midnight);
                // canceled instances without times: mark existing rows canceled
                for e in v
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if e.get("status").and_then(Value::as_str) == Some("cancelled")
                        && e.pointer("/start").is_none()
                        && let Some(id) = e.get("id").and_then(Value::as_str)
                    {
                        let (acc, cal, ext, now) = (
                            cx.account.id.clone(),
                            source.external_id.clone(),
                            id.to_string(),
                            cx.now_str(),
                        );
                        out.events_changed += cx.db.write(move |c| {
                            Ok(c.execute(
                                "UPDATE calendar_events SET status = 'cancelled', synced_at = ?4, updated_at = ?4
                                  WHERE account_id = ?1 AND calendar_id = ?2 AND external_id = ?3",
                                rusqlite::params![acc, cal, ext, now],
                            )?)
                        })?;
                    }
                }
                self.write_events(cx, events, out)?;
                page = v
                    .get("nextPageToken")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(t) = v.get("nextSyncToken").and_then(Value::as_str) {
                    next_token = Some(t.to_string());
                }
                if page.is_none() {
                    break;
                }
            }
            if next_token.is_some() {
                set_cursor(cx, &source.external_id, next_token, None)?;
            }
            return Ok(());
        }
        self.full(cx, source, &base, &midnight, out)
    }

    fn full(
        &self,
        cx: &SyncCx<'_>,
        source: &AccountSource,
        base: &str,
        midnight: &dyn Fn(time::Date) -> OffsetDateTime,
        out: &mut SyncOutcome,
    ) -> Result<()> {
        out.full_sync = true;
        let lo = crate::db::format_timestamp(cx.now - Duration::days(WINDOW_BACK_DAYS));
        let hi = crate::db::format_timestamp(cx.now + Duration::days(WINDOW_AHEAD_DAYS));
        let mut page: Option<String> = None;
        let mut sync_token = None;
        let mut seen: Vec<String> = Vec::new();
        loop {
            let mut req = HttpRequest::get(base).query(&[
                ("timeMin", lo.as_str()),
                ("timeMax", hi.as_str()),
                ("singleEvents", "true"),
                ("orderBy", "startTime"),
                ("maxResults", "250"),
            ]);
            if let Some(p) = &page {
                req = req.query(&[("pageToken", p.as_str())]);
            }
            let v = ok_json(&cx.api, req)?;
            let events = self.page_events(cx, source, &v, midnight);
            seen.extend(events.iter().map(|e| e.external_id.clone()));
            self.write_events(cx, events, out)?;
            page = v
                .get("nextPageToken")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(t) = v.get("nextSyncToken").and_then(Value::as_str) {
                sync_token = Some(t.to_string());
            }
            if page.is_none() {
                break;
            }
        }
        // A full listing omits events deleted upstream (and, after a 410,
        // the deletions the lost token would have reported): mark the rows
        // it no longer returns canceled, as incremental sync would have.
        let (acc, cal, now_s) = (
            cx.account.id.clone(),
            source.external_id.clone(),
            cx.now_str(),
        );
        out.events_changed += cx.db.write(move |c| {
            let seen = serde_json::to_string(&seen).unwrap_or_else(|_| "[]".into());
            Ok(c.execute(
                "UPDATE calendar_events SET status = 'cancelled', synced_at = ?4, updated_at = ?4
                  WHERE account_id = ?1 AND calendar_id = ?2 AND status <> 'cancelled'
                    AND external_id NOT IN (SELECT value FROM json_each(?3))",
                rusqlite::params![acc, cal, seen, now_s],
            )?)
        })?;
        let acc = cx.account.id.clone();
        let now = cx.now;
        out.deleted += cx.db.write(move |c| store::prune_events(c, &acc, now))?;
        set_cursor(cx, &source.external_id, sync_token, Some(cx.now_str()))?;
        Ok(())
    }
}

impl ConnectorAdapter for GcalAdapter {
    fn provider(&self) -> Provider {
        Provider::Google
    }

    fn capabilities(&self, account: &Account) -> Capabilities {
        let mut c = super::gmail::capabilities(&super::json_strings(&account.scopes_json));
        c.read_events = true;
        c.deep_link = "html_link".into();
        c.incremental_cursor = "sync_token".into();
        c
    }

    fn identify(&self, api: &Api<'_>, granted: &[String]) -> Result<Identity> {
        super::gmail::google_identify(api, granted)
    }

    fn list_sources(&self, api: &Api<'_>, account: &Account) -> Result<Vec<SourceCandidate>> {
        if !super::json_strings(&account.services_json)
            .iter()
            .any(|s| s == "calendar")
        {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut req = HttpRequest::get(format!("{API}/users/me/calendarList"));
            if let Some(p) = &page {
                req = req.query(&[("pageToken", p.as_str())]);
            }
            let v = ok_json(api, req)?;
            for c in v
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = c.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let name = c
                    .get("summaryOverride")
                    .or_else(|| c.get("summary"))
                    .and_then(Value::as_str)
                    .unwrap_or(id);
                let primary = c.get("primary").and_then(Value::as_bool).unwrap_or(false);
                let selected = c.get("selected").and_then(Value::as_bool).unwrap_or(false);
                out.push(SourceCandidate {
                    kind: SourceKind::Calendar,
                    external_id: id.into(),
                    name: name.into(),
                    enabled: primary || selected,
                    private: false,
                    member_count: None,
                });
            }
            page = v
                .get("nextPageToken")
                .and_then(Value::as_str)
                .map(str::to_string);
            if page.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn sync(&self, cx: &SyncCx<'_>, source: &AccountSource) -> Result<SyncOutcome> {
        let mut out = SyncOutcome::default();
        let r = self.sync_calendar(cx, source, &mut out);
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

    fn fetch_body(&self, _cx: &SyncCx<'_>, _message: &Message) -> Result<FetchedBody> {
        Err(ConnectorError::Unsupported)
    }

    fn write_back(&self, _cx: &SyncCx<'_>, _item: &ConnectorWriteback) -> Result<()> {
        Err(ConnectorError::Unsupported)
    }

    fn source_url(&self, _account: &Account, message: &Message) -> String {
        message.source_url.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_parse_to_utc() {
        assert_eq!(
            parse_rfc3339("2026-10-22T10:00:00+02:00").map(crate::db::format_timestamp),
            Some("2026-10-22T08:00:00.000Z".into())
        );
        assert_eq!(
            parse_rfc3339("2026-10-22T10:00:00-05:30").map(crate::db::format_timestamp),
            Some("2026-10-22T15:30:00.000Z".into())
        );
        assert_eq!(
            parse_rfc3339("2026-10-22T10:00:00Z").map(crate::db::format_timestamp),
            Some("2026-10-22T10:00:00.000Z".into())
        );
        assert_eq!(
            path_segment("en.usa#holiday@group"),
            "en.usa%23holiday%40group"
        );
    }
}
