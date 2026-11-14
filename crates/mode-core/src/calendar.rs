//! Calendar (08 §3.2 `calendar`; 06 §7; 02 §7.8; 03 §6): join-link
//! extraction (06 §7.4, first match wins), the overlay's `agenda.changed`
//! (contract `AgendaChanged`: today from now in sections Now / Next / Later
//! today, plus tomorrow's first event when today is empty; canceled events
//! of the day struck through; declined never), the app's agenda range, the
//! next event, Join, and Prepare notes (an existing `prepare_for` note is
//! reused; otherwise `notes::api::note_prepare` creates one).

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{Duration, OffsetDateTime};

use crate::db::{DbError, Result, format_timestamp, parse_timestamp};
use crate::notes::fields::LocalTz;

fn re(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("static regex")
}

/// The join link and its `conference` (06 §7.4), first match wins:
/// `conferenceData` video entry points, `hangoutLink`, then Meet / Zoom /
/// Teams / other patterns in `location`, then `description`.
pub fn join_link(e: &Value) -> (Option<String>, Option<String>) {
    if let Some(eps) = e
        .pointer("/conferenceData/entryPoints")
        .and_then(Value::as_array)
        && let Some(uri) = eps
            .iter()
            .find(|p| p.get("entryPointType").and_then(Value::as_str) == Some("video"))
            .and_then(|p| p.get("uri").and_then(Value::as_str))
    {
        let key = e
            .pointer("/conferenceData/conferenceSolution/key/type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let conf = if key == "hangoutsMeet" {
            "meet".to_string()
        } else {
            conference_by_host(uri)
        };
        return (Some(uri.to_string()), Some(conf));
    }
    if let Some(h) = e.get("hangoutLink").and_then(Value::as_str) {
        return (Some(h.to_string()), Some("meet".into()));
    }
    static PATTERNS: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> =
        std::sync::OnceLock::new();
    let pats = PATTERNS.get_or_init(|| {
        vec![
            (re(r"https://meet\.google\.com/[a-z]{3}-[a-z]{4}-[a-z]{3}"), "meet"),
            (re(r"https://[\w.-]*zoom\.us/(j|my|w)/[^\s<>]+"), "zoom"),
            (
                re(r"https://teams\.microsoft\.com/l/meetup-join/[^\s<>]+|https://teams\.live\.com/meet/[^\s<>]+"),
                "teams",
            ),
            (
                re(r"https://[\w.-]*(webex\.com|whereby\.com|around\.co|meet\.jit\.si)/[^\s<>]+"),
                "other",
            ),
        ]
    });
    for field in ["location", "description"] {
        let Some(text) = e.get(field).and_then(Value::as_str) else {
            continue;
        };
        for (r, conf) in pats {
            if let Some(m) = r.find(text) {
                let url = m
                    .as_str()
                    .trim_end_matches(['.', ',', ')', '"', '\''])
                    .to_string();
                return (Some(url), Some((*conf).to_string()));
            }
        }
    }
    (None, None)
}

fn conference_by_host(uri: &str) -> String {
    let host = url::Url::parse(uri)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default();
    if host.ends_with("meet.google.com") {
        "meet".into()
    } else if host.ends_with("zoom.us") {
        "zoom".into()
    } else if host.contains("teams.microsoft.com") || host.contains("teams.live.com") {
        "teams".into()
    } else {
        "other".into()
    }
}

/// An overlay Calendar row (contract `AgendaRow`, 02 §7.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgendaRow {
    pub id: String,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub status: String,
    pub my_response: Option<String>,
    pub account_color: Option<String>,
    pub calendar_name: Option<String>,
    pub conference: Option<String>,
    pub join_url: Option<String>,
    pub location: Option<String>,
    /// `now` | `next` | `later` | `tomorrow`.
    pub section: String,
    /// Additive fields for the tooltip and actions.
    pub html_link: Option<String>,
    pub organizer: Option<String>,
    pub attendee_count: i64,
    pub prepare_note_id: Option<String>,
}

/// `agenda.changed` (contract `AgendaChanged`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgendaChanged {
    pub rows: Vec<AgendaRow>,
    pub next_2h_count: i64,
}

const ROW_SQL: &str = "
SELECT e.id, e.title, e.starts_at, e.ends_at, e.all_day, e.status, e.my_response, a.color,
       e.calendar_name, e.conference, e.join_url, e.location, e.html_link, e.organizer, e.attendees_json,
       (SELECT l.to_id FROM links l
         WHERE l.from_kind = 'calendar_event' AND l.from_id = e.id AND l.to_kind = 'note'
           AND l.link_type = 'prepare_for' AND l.deleted_at IS NULL LIMIT 1)
  FROM calendar_events e JOIN accounts a ON a.id = e.account_id
 WHERE e.my_response IS NOT 'declined'
   AND e.ends_at > :from AND e.starts_at < :to
   AND e.calendar_id IN (SELECT s.external_id FROM account_sources s
                          WHERE s.account_id = e.account_id AND s.kind = 'calendar' AND s.enabled = 1)
 ORDER BY e.all_day DESC, e.starts_at";

fn rows_between(conn: &Connection, from: &str, to: &str) -> Result<Vec<AgendaRow>> {
    let mut st = conn.prepare(ROW_SQL)?;
    let rows = st.query_map(rusqlite::named_params! { ":from": from, ":to": to }, |r| {
        let att: String = r.get(14)?;
        Ok(AgendaRow {
            id: r.get(0)?,
            title: r.get(1)?,
            starts_at: r.get(2)?,
            ends_at: r.get(3)?,
            all_day: r.get(4)?,
            status: r.get(5)?,
            my_response: r.get(6)?,
            account_color: r.get(7)?,
            calendar_name: r.get(8)?,
            conference: r.get(9)?,
            join_url: r.get(10)?,
            location: r.get(11)?,
            html_link: r.get(12)?,
            organizer: r.get(13)?,
            attendee_count: serde_json::from_str::<Vec<Value>>(&att)
                .map(|v| v.len() as i64)
                .unwrap_or(0),
            prepare_note_id: r.get(15)?,
            section: String::new(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The overlay's Calendar tab (02 §7.3 table: "Q9 for today from now,
/// plus tomorrow's first event when today is empty").
pub fn agenda_changed(
    conn: &Connection,
    now: OffsetDateTime,
    tz: &dyn LocalTz,
) -> Result<AgendaChanged> {
    let today = tz.local_date(now);
    let tomorrow = today + Duration::days(1);
    let day_after = today + Duration::days(2);
    let end_today = format_timestamp(tz.local_midnight(tomorrow));
    let now_s = format_timestamp(now);
    let in_2h = format_timestamp(now + Duration::hours(2));
    let mut rows = Vec::new();
    for mut r in rows_between(conn, &now_s, &end_today)? {
        r.section = if r.starts_at <= now_s && r.ends_at > now_s {
            "now".into()
        } else if r.starts_at <= in_2h {
            "next".into()
        } else {
            "later".into()
        };
        rows.push(r);
    }
    let live_today = rows.iter().any(|r| r.status != "cancelled");
    if !live_today {
        let tomorrow_rows = rows_between(
            conn,
            &format_timestamp(tz.local_midnight(tomorrow)),
            &format_timestamp(tz.local_midnight(day_after)),
        )?;
        if let Some(mut first) = tomorrow_rows
            .into_iter()
            .find(|r| r.status != "cancelled" && !r.all_day)
        {
            first.section = "tomorrow".into();
            rows.push(first);
        }
    }
    let next_2h_count = rows
        .iter()
        .filter(|r| r.status != "cancelled" && r.starts_at > now_s && r.starts_at <= in_2h)
        .count() as i64;
    Ok(AgendaChanged {
        rows,
        next_2h_count,
    })
}

/// One event of the app's Calendar view (03 §6), with its detail fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRow {
    pub id: String,
    pub account_id: String,
    pub account_label: String,
    pub account_color: Option<String>,
    pub calendar_id: String,
    pub calendar_name: Option<String>,
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
    pub prepare_note_id: Option<String>,
    pub synced_at: String,
}

const EVENT_SQL: &str = "
SELECT e.id, e.account_id, a.display_name, a.color, e.calendar_id, e.calendar_name, e.title, e.description,
       e.location, e.starts_at, e.ends_at, e.all_day, e.start_date, e.end_date, e.timezone, e.status,
       e.my_response, e.organizer, e.attendees_json, e.join_url, e.conference, e.html_link,
       (SELECT l.to_id FROM links l
         WHERE l.from_kind = 'calendar_event' AND l.from_id = e.id AND l.to_kind = 'note'
           AND l.link_type = 'prepare_for' AND l.deleted_at IS NULL LIMIT 1),
       e.synced_at
  FROM calendar_events e JOIN accounts a ON a.id = e.account_id";

fn event_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    let att: String = r.get(18)?;
    Ok(EventRow {
        id: r.get(0)?,
        account_id: r.get(1)?,
        account_label: r.get(2)?,
        account_color: r.get(3)?,
        calendar_id: r.get(4)?,
        calendar_name: r.get(5)?,
        title: r.get(6)?,
        description: r.get(7)?,
        location: r.get(8)?,
        starts_at: r.get(9)?,
        ends_at: r.get(10)?,
        all_day: r.get(11)?,
        start_date: r.get(12)?,
        end_date: r.get(13)?,
        timezone: r.get(14)?,
        status: r.get(15)?,
        my_response: r.get(16)?,
        organizer: r.get(17)?,
        attendees: serde_json::from_str(&att).unwrap_or(Value::Array(vec![])),
        join_url: r.get(19)?,
        conference: r.get(20)?,
        html_link: r.get(21)?,
        prepare_note_id: r.get(22)?,
        synced_at: r.get(23)?,
    })
}

/// Events overlapping `[from, to)` on enabled calendars (declined ones
/// excluded, canceled ones kept with their status), all-day first.
pub fn agenda_list(conn: &Connection, from: &str, to: &str) -> Result<Vec<EventRow>> {
    if parse_timestamp(from).is_none() || parse_timestamp(to).is_none() {
        return Err(DbError::Invalid {
            what: "range",
            detail: "from and to are timestamps".into(),
        });
    }
    let sql = format!(
        "{EVENT_SQL}
         WHERE e.my_response IS NOT 'declined' AND e.ends_at > ?1 AND e.starts_at < ?2
           AND e.calendar_id IN (SELECT s.external_id FROM account_sources s
                                  WHERE s.account_id = e.account_id AND s.kind = 'calendar' AND s.enabled = 1)
         ORDER BY e.all_day DESC, e.starts_at"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params![from, to], event_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn event(conn: &Connection, id: &str) -> Result<Option<EventRow>> {
    let sql = format!("{EVENT_SQL} WHERE e.id = ?1");
    Ok(conn.query_row(&sql, [id], event_row).optional()?)
}

/// The next event starting after `now` (not canceled, not declined).
pub fn next_event(
    conn: &Connection,
    now: OffsetDateTime,
    within_hours: i64,
) -> Result<Option<EventRow>> {
    let from = format_timestamp(now);
    let to = format_timestamp(now + Duration::hours(within_hours));
    Ok(agenda_list(conn, &from, &to)?
        .into_iter()
        .find(|e| e.status != "cancelled" && e.starts_at > from))
}

/// Join (03 §6.7): the event's `join_url`.
pub fn join_url(conn: &Connection, id: &str) -> Result<String> {
    let e = event(conn, id)?.ok_or_else(|| DbError::NotFound {
        table: "calendar_events",
        key: id.into(),
    })?;
    e.join_url.ok_or_else(|| DbError::Invalid {
        what: "event",
        detail: "this event has no join link".into(),
    })
}

/// The live prepare note of an event, if one exists.
pub fn prepare_note(conn: &Connection, event_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT l.to_id FROM links l JOIN notes n ON n.id = l.to_id AND n.deleted_at IS NULL
              WHERE l.from_kind = 'calendar_event' AND l.from_id = ?1 AND l.to_kind = 'note'
                AND l.link_type = 'prepare_for' AND l.deleted_at IS NULL
              ORDER BY l.created_at DESC LIMIT 1",
            [event_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Prepare notes (06 §7, 03 §6.7): reuse the linked note, else create it
/// through the notes core. Returns the note id.
pub fn prepare_notes(
    notes: &crate::notes::Notes,
    event_id: &str,
) -> std::result::Result<String, crate::notes::api::ApiError> {
    let exists = notes
        .db()
        .read(|c| event(c, event_id))
        .map_err(crate::notes::api::ApiError::from)?;
    if exists.is_none() {
        return Err(crate::notes::api::ApiError {
            code: "not_found".into(),
            message: "the event is no longer in the calendar".into(),
        });
    }
    if let Some(id) = notes
        .db()
        .read(|c| prepare_note(c, event_id))
        .map_err(crate::notes::api::ApiError::from)?
    {
        return Ok(id);
    }
    let c = crate::notes::api::note_prepare(
        notes,
        crate::notes::api::FromItemArgs {
            id: event_id.to_string(),
        },
    )?;
    Ok(c.note_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn join_links_in_spec_order() {
        let conf = json!({"conferenceData":{"entryPoints":[{"entryPointType":"phone","uri":"tel:+1"},
            {"entryPointType":"video","uri":"https://meet.google.com/abc-defg-hij"}],
            "conferenceSolution":{"key":{"type":"hangoutsMeet"}}},
            "hangoutLink":"https://meet.google.com/zzz-zzzz-zzz"});
        assert_eq!(
            join_link(&conf),
            (
                Some("https://meet.google.com/abc-defg-hij".into()),
                Some("meet".into())
            )
        );
        let zoom_ep = json!({"conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://acme.zoom.us/j/123"}],
            "conferenceSolution":{"key":{"type":"addOn"}}}});
        assert_eq!(join_link(&zoom_ep).1.as_deref(), Some("zoom"));
        let hang = json!({"hangoutLink":"https://meet.google.com/aaa-bbbb-ccc"});
        assert_eq!(join_link(&hang).1.as_deref(), Some("meet"));
        let loc = json!({"location":"Room 4 / https://acme.zoom.us/j/987654?pwd=x", "description":"https://meet.google.com/aaa-bbbb-ccc"});
        assert_eq!(
            join_link(&loc),
            (
                Some("https://acme.zoom.us/j/987654?pwd=x".into()),
                Some("zoom".into())
            )
        );
        let teams = json!({"description":"Join: https://teams.microsoft.com/l/meetup-join/19%3ameeting_x/0?context=y."});
        assert_eq!(join_link(&teams).1.as_deref(), Some("teams"));
        let other = json!({"description":"https://meet.jit.si/ModeStandup"});
        assert_eq!(join_link(&other).1.as_deref(), Some("other"));
        assert_eq!(join_link(&json!({"location":"Room 4"})), (None, None));
    }
}
