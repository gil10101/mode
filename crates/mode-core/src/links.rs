//! Thread and event links (03 §4.1 "links", §6.6 "linked"; 09 §5.5
//! `links`, `link_dismissals`, §13 S1): the sessions, notes and messages a
//! message thread or a calendar event is about, found on open without an AI
//! call or the network.
//!
//! Three sources, in this order of trust:
//!
//! 1. **Explicit**: `links` rows touching the thread (or one of its
//!    messages) or the event, in either direction: links the user made by
//!    hand ("linked by you"), notes captured from the thread ("made from
//!    this thread"), the event's prepare note ("prepare note").
//! 2. **Mentions**: the thread's subjects and bodies, or the event's title
//!    and description, name a session (display name or git branch), a
//!    note's title, or a PR / issue number that the session's or note's own
//!    text also carries. Matching is on whole words, case-insensitive, with
//!    `-`, `_`, `/` and spaces equivalent (`rate limiter` names
//!    `rate-limiter`); names shorter than four characters, and names made
//!    only of common words (`main`, `notes`, `Release planning`), never match.
//! 3. **Attendees** (events only): messages from the event's attendees
//!    (by email, or a Slack user on the same contact) received in the three
//!    days before the start whose subject shares a meaningful word with the
//!    event title.
//!
//! Suggestions (2, 3) stop at five per kind; explicit links are never
//! dropped. Removing a link tombstones the explicit rows and records a
//! dismissal so the suggestion stays away; adding one by hand writes a
//! `references` row (`created_by = 'user'`) and clears the dismissal.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::Duration;

use crate::db::{DbError, Result, format_timestamp, parse_timestamp};
use crate::repo::{self, Link};

/// Suggested links per kind (explicit links are not counted against it).
pub const CAP_PER_KIND: usize = 5;
/// Shorter names never match (`api`, `ui`).
pub const MIN_NAME_CHARS: usize = 4;
/// How far before an event an attendee's message counts.
pub const ATTENDEE_WINDOW_DAYS: i64 = 3;
/// How many recent sessions and notes are matched against (newest first).
const SESSION_POOL: i64 = 200;
const NOTE_POOL: i64 = 500;
/// How much of a note body or message body is read for PR numbers and mentions.
const TEXT_CHARS: i64 = 8000;

/// One row of the thread panel's "links" group or the event panel's "linked".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemLink {
    /// `session` | `note` | `message`.
    pub kind: String,
    pub id: String,
    pub title: String,
    /// Why it is here: `linked by you`, `mentions rate-limiter`, `from an attendee`.
    pub why: String,
    /// The user made this link by hand.
    pub by_user: bool,
    /// The session's state (the row glyph); `None` for notes and messages.
    pub state: Option<String>,
}

/// What the links hang off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor<'a> {
    Thread(&'a str),
    Event(&'a str),
}

impl Anchor<'_> {
    pub fn kind(&self) -> &'static str {
        match self {
            Anchor::Thread(_) => "thread",
            Anchor::Event(_) => "calendar_event",
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Anchor::Thread(id) | Anchor::Event(id) => id,
        }
    }
}

/// The kinds a link may point at.
pub const TARGET_KINDS: &[&str] = &["session", "note", "message"];

// ------------------------------------------------------------------ matcher

/// Lowercase words: runs of letters and digits; everything else separates.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn common() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| COMMON.iter().copied().collect())
}

/// A word too common to identify a session or note on its own.
pub fn is_common(word: &str) -> bool {
    common().contains(word) || stop().contains(word)
}

/// The words of a name worth matching, or `None` when the name is shorter
/// than [`MIN_NAME_CHARS`], has no letter, or is made only of common words.
pub fn name_words(name: &str) -> Option<Vec<String>> {
    let w = words(name);
    let chars: usize = w.iter().map(|x| x.chars().count()).sum();
    if w.is_empty() || chars < MIN_NAME_CHARS {
        return None;
    }
    if !w.iter().any(|x| x.chars().any(char::is_alphabetic)) {
        return None;
    }
    if w.iter().all(|x| is_common(x)) {
        return None;
    }
    Some(w)
}

/// `needle` appears in `hay` as consecutive whole words.
pub fn contains_words(hay: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|win| win == needle)
}

fn refs_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(pull request|pr|pull|mr|issue)[\s/]*(?:#|no\.?\s*)?(\d{1,6})\b|(?:^|[^\w&/#])#(\d{2,6})\b",
        )
        .expect("static regex")
    })
}

/// PR and issue numbers in a text, each with how the text says it
/// (`PR 412`, `issue 88`, `#412`); the first spelling of a number wins.
pub fn refs(text: &str) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    for c in refs_re().captures_iter(text) {
        let (n, label) = if let (Some(k), Some(n)) = (c.get(1), c.get(2)) {
            let Ok(n) = n.as_str().parse::<u32>() else {
                continue;
            };
            let word = if k.as_str().eq_ignore_ascii_case("issue") {
                "issue"
            } else {
                "PR"
            };
            (n, format!("{word} {n}"))
        } else if let Some(n) = c.get(3) {
            let Ok(n) = n.as_str().parse::<u32>() else {
                continue;
            };
            (n, format!("#{n}"))
        } else {
            continue;
        };
        out.entry(n).or_insert(label);
    }
    out
}

fn stop() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| STOP.iter().copied().collect())
}

fn singular(w: &str) -> String {
    if w.len() > 4 && w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_string()
    } else {
        w.to_string()
    }
}

/// The words of a title or subject that carry meaning: three letters or
/// more, not a function word or a meeting word (`sync`, `call`, `re`),
/// plurals folded (`releases` = `release`).
pub fn meaningful(text: &str) -> HashSet<String> {
    words(text)
        .into_iter()
        .filter(|w| w.chars().count() >= 3 && w.chars().any(char::is_alphabetic))
        .filter(|w| !stop().contains(w.as_str()))
        .map(|w| singular(&w))
        .collect()
}

/// A candidate session or note for the mention rules.
#[derive(Debug, Clone, Default)]
pub struct Candidate {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    /// Names that identify it as written (display name, branch, title).
    pub names: Vec<String>,
    /// PR / issue numbers its own text carries.
    pub refs: HashSet<u32>,
    pub state: Option<String>,
}

/// The text a thread or event is matched on: its words and its references.
#[derive(Debug, Clone, Default)]
pub struct Haystack {
    words: Vec<String>,
    refs: BTreeMap<u32, String>,
}

impl Haystack {
    pub fn new(texts: &[&str]) -> Self {
        let mut words_out = Vec::new();
        let mut refs_out = BTreeMap::new();
        for t in texts {
            // A sentinel between texts so a name never spans two of them.
            words_out.extend(words(t));
            words_out.push("\u{0}".into());
            for (n, l) in refs(t) {
                refs_out.entry(n).or_insert(l);
            }
        }
        Haystack {
            words: words_out,
            refs: refs_out,
        }
    }
}

/// Why `c` is mentioned by `hay`, if it is: the first matching name
/// (`mentions rate-limiter`), else a shared PR / issue number
/// (`mentions PR 412`).
pub fn mention(hay: &Haystack, c: &Candidate) -> Option<String> {
    for name in &c.names {
        if let Some(w) = name_words(name)
            && contains_words(&hay.words, &w)
        {
            return Some(format!("mentions {name}"));
        }
    }
    hay.refs
        .iter()
        .find(|(n, _)| c.refs.contains(n))
        .map(|(_, label)| format!("mentions {label}"))
}

/// A picked link before ordering and capping.
#[derive(Debug, Clone)]
pub struct Pick {
    pub link: ItemLink,
    pub explicit: bool,
}

/// Order (sessions, notes, messages; explicit first, then suggestions in the
/// order found), drop duplicates (explicit wins), drop dismissed suggestions
/// and cap suggestions at [`CAP_PER_KIND`] per kind.
pub fn assemble(picks: Vec<Pick>, dismissed: &HashSet<(String, String)>) -> Vec<ItemLink> {
    let mut out = Vec::new();
    for kind in TARGET_KINDS {
        let mut seen: HashSet<String> = HashSet::new();
        let mut n = 0usize;
        for p in picks.iter().filter(|p| p.link.kind == *kind && p.explicit) {
            if seen.insert(p.link.id.clone()) {
                out.push(p.link.clone());
                n += 1;
            }
        }
        for p in picks.iter().filter(|p| p.link.kind == *kind && !p.explicit) {
            if n >= CAP_PER_KIND {
                break;
            }
            if dismissed.contains(&(p.link.kind.clone(), p.link.id.clone())) {
                continue;
            }
            if seen.insert(p.link.id.clone()) {
                out.push(p.link.clone());
                n += 1;
            }
        }
    }
    out
}

// --------------------------------------------------------------- the store

fn session_candidates(conn: &Connection) -> Result<Vec<Candidate>> {
    let mut st = conn.prepare(
        "SELECT id, display_name, git_branch, state, title, last_prompt_preview, last_assistant_preview
           FROM sessions WHERE parent_session_id IS NULL
          ORDER BY last_event_at DESC LIMIT ?1",
    )?;
    let rows = st.query_map([SESSION_POOL], |r| {
        let name: String = r.get(1)?;
        let branch: Option<String> = r.get(2)?;
        let mut text = vec![name.clone()];
        for i in [2, 4, 5, 6] {
            if let Some(t) = r.get::<_, Option<String>>(i)? {
                text.push(t);
            }
        }
        let mut names = vec![name.clone()];
        if let Some(b) = branch
            && words(&b) != words(&name)
        {
            names.push(b);
        }
        Ok(Candidate {
            kind: "session",
            id: r.get(0)?,
            title: name,
            names,
            refs: text.iter().flat_map(|t| refs(t).into_keys()).collect(),
            state: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn note_candidates(conn: &Connection) -> Result<Vec<Candidate>> {
    let mut st = conn.prepare(
        "SELECT id, title, substr(body, 1, ?2) FROM notes
          WHERE deleted_at IS NULL AND archived_at IS NULL
          ORDER BY updated_at DESC LIMIT ?1",
    )?;
    let rows = st.query_map(params![NOTE_POOL, TEXT_CHARS], |r| {
        let title: String = r.get(1)?;
        let body: String = r.get(2)?;
        let mut rs: HashSet<u32> = refs(&title).into_keys().collect();
        rs.extend(refs(&body).into_keys());
        Ok(Candidate {
            kind: "note",
            id: r.get(0)?,
            names: vec![title.clone()],
            title,
            refs: rs,
            state: None,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// `Sam · #release`, `Priya · Re: API v2 rollout`.
fn message_title(
    sender: &str,
    subject: Option<&str>,
    channel: Option<&str>,
    snippet: &str,
) -> String {
    let what = channel
        .filter(|c| !c.is_empty())
        .or(subject.filter(|s| !s.is_empty()))
        .unwrap_or_else(|| snippet.lines().next().unwrap_or(""));
    if what.is_empty() {
        sender.to_string()
    } else {
        format!("{sender} · {what}")
    }
}

/// `(title, state)` of a live target, `None` when it is gone.
fn target(conn: &Connection, kind: &str, id: &str) -> Result<Option<(String, Option<String>)>> {
    Ok(match kind {
        "note" => conn
            .query_row(
                "SELECT title FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |r| Ok((r.get(0)?, None)),
            )
            .optional()?,
        "session" => conn
            .query_row(
                "SELECT display_name, state FROM sessions WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?,
        "message" => conn
            .query_row(
                "SELECT m.sender_name, m.subject, t.channel_name, m.snippet
                   FROM messages m LEFT JOIN threads t ON t.id = m.thread_id WHERE m.id = ?1",
                [id],
                |r| {
                    let s: String = r.get(0)?;
                    let sub: Option<String> = r.get(1)?;
                    let ch: Option<String> = r.get(2)?;
                    let sn: String = r.get(3)?;
                    Ok((message_title(&s, sub.as_deref(), ch.as_deref(), &sn), None))
                },
            )
            .optional()?,
        _ => None,
    })
}

fn latest_message(conn: &Connection, thread_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM messages WHERE thread_id = ?1 ORDER BY received_at DESC LIMIT 1",
            [thread_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The rows (both directions) of the anchor's ends: the thread and its
/// messages, or the event.
fn ends(conn: &Connection, anchor: Anchor<'_>) -> Result<Vec<(&'static str, String)>> {
    let mut out = vec![(anchor.kind(), anchor.id().to_string())];
    if let Anchor::Thread(t) = anchor {
        let mut st = conn.prepare("SELECT id FROM messages WHERE thread_id = ?1")?;
        for id in st.query_map([t], |r| r.get::<_, String>(0))? {
            out.push(("message", id?));
        }
    }
    Ok(out)
}

fn explicit(conn: &Connection, anchor: Anchor<'_>) -> Result<Vec<Pick>> {
    let own: Vec<(&str, String)> = ends(conn, anchor)?;
    let mut out = Vec::new();
    for (k, id) in &own {
        for l in crate::queries::links_for(conn, k, id)? {
            let (kind, tid) = match l.other_kind.as_str() {
                "note" | "session" => (l.other_kind.clone(), l.other_id.clone()),
                "message" => {
                    if own.iter().any(|(k, i)| *k == "message" && *i == l.other_id) {
                        continue;
                    }
                    ("message".to_string(), l.other_id.clone())
                }
                "thread" => {
                    if matches!(anchor, Anchor::Thread(t) if t == l.other_id) {
                        continue;
                    }
                    match latest_message(conn, &l.other_id)? {
                        Some(m) => ("message".to_string(), m),
                        None => continue,
                    }
                }
                _ => continue,
            };
            let Some((title, state)) = target(conn, &kind, &tid)? else {
                continue;
            };
            let why = match (l.link_type.as_str(), l.created_by.as_str()) {
                ("prepare_for", _) => "prepare note".to_string(),
                ("created_from", _) => match anchor {
                    Anchor::Thread(_) => "made from this thread".to_string(),
                    Anchor::Event(_) => "made from this event".to_string(),
                },
                (_, "user") => "linked by you".to_string(),
                (_, "ai") => "linked in chat".to_string(),
                _ => "linked".to_string(),
            };
            out.push(Pick {
                link: ItemLink {
                    kind,
                    id: tid,
                    title,
                    why,
                    by_user: l.created_by == "user",
                    state,
                },
                explicit: true,
            });
        }
    }
    Ok(out)
}

fn dismissed(conn: &Connection, anchor: Anchor<'_>) -> Result<HashSet<(String, String)>> {
    let mut st = conn.prepare(
        "SELECT target_kind, target_id FROM link_dismissals WHERE anchor_kind = ?1 AND anchor_id = ?2",
    )?;
    let rows = st.query_map(params![anchor.kind(), anchor.id()], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
}

fn mentions(conn: &Connection, hay: &Haystack) -> Result<Vec<Pick>> {
    let mut out = Vec::new();
    for c in session_candidates(conn)?
        .into_iter()
        .chain(note_candidates(conn)?)
    {
        if let Some(why) = mention(hay, &c) {
            out.push(Pick {
                link: ItemLink {
                    kind: c.kind.into(),
                    id: c.id,
                    title: c.title,
                    why,
                    by_user: false,
                    state: c.state,
                },
                explicit: false,
            });
        }
    }
    Ok(out)
}

/// The thread panel's links (03 §4.1). Empty when the thread is gone.
pub fn thread_links(conn: &Connection, thread_id: &str) -> Result<Vec<ItemLink>> {
    let anchor = Anchor::Thread(thread_id);
    let mut st = conn.prepare(
        "SELECT COALESCE(subject, ''), COALESCE(substr(body_text, 1, ?2), snippet)
           FROM messages WHERE thread_id = ?1 ORDER BY received_at DESC LIMIT 20",
    )?;
    let texts = st
        .query_map(params![thread_id, TEXT_CHARS], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let flat: Vec<&str> = texts
        .iter()
        .flat_map(|(a, b)| [a.as_str(), b.as_str()])
        .collect();
    let hay = Haystack::new(&flat);
    let mut picks = explicit(conn, anchor)?;
    picks.extend(mentions(conn, &hay)?);
    Ok(assemble(picks, &dismissed(conn, anchor)?))
}

/// Attendee emails (lowercase) of an event, the user excluded.
pub fn attendee_emails(attendees_json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<Value>>(attendees_json)
        .unwrap_or_default()
        .iter()
        .filter(|a| a.get("self").and_then(Value::as_bool) != Some(true))
        .filter_map(|a| a.get("email").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect()
}

/// Messages from the event's attendees in the window before it whose
/// subject (Slack: channel and first line) shares a meaningful word with
/// the title; one per thread, newest first.
fn attendee_messages(
    conn: &Connection,
    title: &str,
    starts_at: &str,
    attendees_json: &str,
) -> Result<Vec<Pick>> {
    let emails = attendee_emails(attendees_json);
    let title_words = meaningful(title);
    if emails.is_empty() || title_words.is_empty() {
        return Ok(Vec::new());
    }
    let Some(start) = parse_timestamp(starts_at) else {
        return Ok(Vec::new());
    };
    let from = format_timestamp(start - Duration::days(ATTENDEE_WINDOW_DAYS));
    let emails_json = serde_json::to_string(&emails)?;
    let mut st = conn.prepare(
        "WITH em(v) AS (SELECT value FROM json_each(?1)),
              ct(id) AS (SELECT contact_id FROM contact_identities
                          WHERE kind = 'email' AND value IN (SELECT v FROM em) AND deleted_at IS NULL),
              sl(v) AS (SELECT value FROM contact_identities
                         WHERE kind = 'slack_user' AND contact_id IN (SELECT id FROM ct) AND deleted_at IS NULL)
         SELECT m.id, m.thread_id, m.sender_name, m.subject, t.channel_name, m.snippet
           FROM messages m LEFT JOIN threads t ON t.id = m.thread_id
          WHERE m.received_at >= ?2 AND m.received_at < ?3
            AND (lower(m.sender_address) IN (SELECT v FROM em)
                 OR m.sender_contact_id IN (SELECT id FROM ct)
                 OR m.sender_address IN (SELECT v FROM sl))
          ORDER BY m.received_at DESC LIMIT 200",
    )?;
    let rows = st
        .query_map(params![emails_json, from, starts_at], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut threads = HashSet::new();
    let mut out = Vec::new();
    for (id, thread, sender, subject, channel, snippet) in rows {
        if let Some(t) = &thread
            && !threads.insert(t.clone())
        {
            continue;
        }
        let first = snippet.lines().next().unwrap_or("");
        let said = match subject.as_deref().filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => format!("{} {first}", channel.as_deref().unwrap_or("")),
        };
        if meaningful(&said).is_disjoint(&title_words) {
            continue;
        }
        out.push(Pick {
            link: ItemLink {
                kind: "message".into(),
                title: message_title(&sender, subject.as_deref(), channel.as_deref(), &snippet),
                id,
                why: "from an attendee".into(),
                by_user: false,
                state: None,
            },
            explicit: false,
        });
    }
    Ok(out)
}

/// The event panel's linked items (03 §6.6); `None` when the event is gone.
pub fn event_links(conn: &Connection, event_id: &str) -> Result<Option<Vec<ItemLink>>> {
    let anchor = Anchor::Event(event_id);
    let Some((title, description, starts_at, attendees)): Option<(String, Option<String>, String, String)> =
        conn.query_row(
            "SELECT title, description, starts_at, attendees_json FROM calendar_events WHERE id = ?1",
            [event_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let hay = Haystack::new(&[&title, description.as_deref().unwrap_or("")]);
    let mut picks = explicit(conn, anchor)?;
    picks.extend(mentions(conn, &hay)?);
    picks.extend(attendee_messages(conn, &title, &starts_at, &attendees)?);
    Ok(Some(assemble(picks, &dismissed(conn, anchor)?)))
}

/// The anchor exists (a thread, or an event).
pub fn anchor_exists(conn: &Connection, anchor: Anchor<'_>) -> Result<bool> {
    let sql = match anchor {
        Anchor::Thread(_) => "SELECT 1 FROM threads WHERE id = ?1",
        Anchor::Event(_) => "SELECT 1 FROM calendar_events WHERE id = ?1",
    };
    Ok(conn
        .query_row(sql, [anchor.id()], |r| r.get::<_, i64>(0))
        .optional()?
        .is_some())
}

fn invalid(detail: impl Into<String>) -> DbError {
    DbError::Invalid {
        what: "link",
        detail: detail.into(),
    }
}

/// The links of an anchor as the panels show them.
pub fn links_of(conn: &Connection, anchor: Anchor<'_>) -> Result<Vec<ItemLink>> {
    Ok(match anchor {
        Anchor::Thread(t) => thread_links(conn, t)?,
        Anchor::Event(e) => event_links(conn, e)?.unwrap_or_default(),
    })
}

/// Link by hand: a `references` row from the anchor to the target
/// (`created_by = 'user'`), and any dismissal of that target cleared.
pub fn add(
    conn: &Connection,
    device_id: &str,
    anchor: Anchor<'_>,
    to_kind: &str,
    to_id: &str,
) -> Result<()> {
    if !TARGET_KINDS.contains(&to_kind) {
        return Err(invalid(format!("{to_kind:?} is not a link target")));
    }
    if !anchor_exists(conn, anchor)? {
        return Err(DbError::NotFound {
            table: if matches!(anchor, Anchor::Thread(_)) {
                "threads"
            } else {
                "calendar_events"
            },
            key: anchor.id().into(),
        });
    }
    if target(conn, to_kind, to_id)?.is_none() {
        return Err(invalid(format!("the {to_kind} is gone")));
    }
    conn.execute(
        "DELETE FROM link_dismissals
          WHERE anchor_kind = ?1 AND anchor_id = ?2 AND target_kind = ?3 AND target_id = ?4",
        params![anchor.kind(), anchor.id(), to_kind, to_id],
    )?;
    let live = repo::count::<Link>(
        conn,
        "from_kind = ? AND from_id = ? AND to_kind = ? AND to_id = ? AND link_type = 'references' AND deleted_at IS NULL",
        &[&anchor.kind(), &anchor.id(), &to_kind, &to_id],
    )?;
    if live == 0 {
        let mut l = Link {
            id: repo::new_id(),
            from_kind: anchor.kind().into(),
            from_id: anchor.id().into(),
            to_kind: to_kind.into(),
            to_id: to_id.into(),
            link_type: "references".into(),
            created_by: "user".into(),
            ..Default::default()
        };
        repo::create(conn, device_id, &mut l)?;
    }
    Ok(())
}

/// Remove a link: tombstone every live `links` row between the anchor's
/// ends and the target (both directions; a message target also covers its
/// thread), and remember the dismissal so no rule suggests it again.
pub fn remove(
    conn: &Connection,
    device_id: &str,
    anchor: Anchor<'_>,
    to_kind: &str,
    to_id: &str,
) -> Result<usize> {
    if !TARGET_KINDS.contains(&to_kind) {
        return Err(invalid(format!("{to_kind:?} is not a link target")));
    }
    let mut targets = vec![(to_kind.to_string(), to_id.to_string())];
    if to_kind == "message"
        && let Some(t) = conn
            .query_row(
                "SELECT thread_id FROM messages WHERE id = ?1",
                [to_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
    {
        targets.push(("thread".into(), t));
    }
    let now = format_timestamp(time::OffsetDateTime::now_utc());
    let mut n = 0;
    for (ek, eid) in ends(conn, anchor)? {
        for (tk, tid) in &targets {
            n += conn.execute(
                "UPDATE links SET deleted_at = ?5, updated_at = ?5, rev = rev + 1, device_id = ?6
                  WHERE deleted_at IS NULL
                    AND ((from_kind = ?1 AND from_id = ?2 AND to_kind = ?3 AND to_id = ?4)
                      OR (from_kind = ?3 AND from_id = ?4 AND to_kind = ?1 AND to_id = ?2))",
                params![ek, eid, tk, tid, now, device_id],
            )?;
        }
    }
    conn.execute(
        "INSERT INTO link_dismissals (anchor_kind, anchor_id, target_kind, target_id, dismissed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT DO UPDATE SET dismissed_at = excluded.dismissed_at",
        params![anchor.kind(), anchor.id(), to_kind, to_id, now],
    )?;
    Ok(n)
}

/// Words that never identify a session or note on their own: everyday
/// English and the words every repo, branch and note title shares.
const COMMON: &[&str] = &[
    "about",
    "after",
    "again",
    "agenda",
    "also",
    "back",
    "before",
    "being",
    "best",
    "better",
    "between",
    "both",
    "branch",
    "bug",
    "bugfix",
    "bugs",
    "build",
    "call",
    "change",
    "changes",
    "check",
    "chore",
    "clean",
    "cleanup",
    "code",
    "come",
    "config",
    "could",
    "daily",
    "data",
    "default",
    "demo",
    "deploy",
    "dev",
    "develop",
    "development",
    "docs",
    "doing",
    "done",
    "draft",
    "each",
    "even",
    "every",
    "feat",
    "feature",
    "features",
    "file",
    "files",
    "final",
    "first",
    "five",
    "fix",
    "fixes",
    "four",
    "from",
    "game",
    "general",
    "good",
    "great",
    "have",
    "here",
    "home",
    "hotfix",
    "idea",
    "ideas",
    "inbox",
    "info",
    "into",
    "issue",
    "issues",
    "item",
    "items",
    "just",
    "last",
    "later",
    "like",
    "list",
    "local",
    "look",
    "made",
    "main",
    "make",
    "many",
    "master",
    "meeting",
    "meetings",
    "minor",
    "misc",
    "mode",
    "more",
    "most",
    "much",
    "must",
    "need",
    "never",
    "next",
    "note",
    "notes",
    "nothing",
    "once",
    "only",
    "other",
    "over",
    "plan",
    "planning",
    "plans",
    "please",
    "post",
    "prod",
    "production",
    "project",
    "projects",
    "quick",
    "read",
    "readme",
    "really",
    "refactor",
    "release",
    "releases",
    "repo",
    "review",
    "reviews",
    "same",
    "scratch",
    "session",
    "sessions",
    "setup",
    "should",
    "show",
    "since",
    "some",
    "soon",
    "staging",
    "start",
    "still",
    "stuff",
    "sync",
    "take",
    "task",
    "tasks",
    "team",
    "temp",
    "test",
    "testing",
    "tests",
    "than",
    "thanks",
    "that",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "things",
    "this",
    "those",
    "three",
    "through",
    "time",
    "today",
    "todo",
    "tomorrow",
    "trunk",
    "update",
    "updates",
    "used",
    "user",
    "users",
    "very",
    "want",
    "week",
    "weekly",
    "well",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "will",
    "with",
    "work",
    "working",
    "would",
    "year",
    "your",
];

/// Function words and meeting words: never "meaningful" in a title.
const STOP: &[&str] = &[
    "a",
    "about",
    "after",
    "all",
    "an",
    "and",
    "any",
    "are",
    "as",
    "at",
    "be",
    "but",
    "by",
    "call",
    "can",
    "catch",
    "chat",
    "daily",
    "did",
    "do",
    "for",
    "from",
    "fw",
    "fwd",
    "get",
    "had",
    "has",
    "have",
    "how",
    "i",
    "if",
    "in",
    "into",
    "invitation",
    "invite",
    "is",
    "it",
    "its",
    "just",
    "let",
    "me",
    "meeting",
    "my",
    "new",
    "no",
    "not",
    "now",
    "of",
    "on",
    "one",
    "or",
    "our",
    "out",
    "per",
    "quick",
    "re",
    "so",
    "sync",
    "that",
    "the",
    "their",
    "then",
    "this",
    "to",
    "up",
    "updated",
    "us",
    "via",
    "was",
    "we",
    "weekly",
    "what",
    "when",
    "who",
    "why",
    "will",
    "with",
    "you",
    "your",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(kind: &'static str, id: &str, names: &[&str], text: &str) -> Candidate {
        Candidate {
            kind,
            id: id.into(),
            title: names[0].into(),
            names: names.iter().map(|s| s.to_string()).collect(),
            refs: refs(text).into_keys().collect(),
            state: None,
        }
    }

    #[test]
    fn names_match_on_whole_words_case_and_separator_blind() {
        let rate = cand("session", "s1", &["rate-limiter", "feat/api"], "");
        let hay = Haystack::new(&["the Rate Limiter is green on main"]);
        assert_eq!(
            mention(&hay, &rate).as_deref(),
            Some("mentions rate-limiter")
        );
        let hay = Haystack::new(&["merged feat/api last night"]);
        assert_eq!(mention(&hay, &rate).as_deref(), Some("mentions feat/api"));
        // Inside a longer word: no match.
        let hay = Haystack::new(&["the ratelimiter crate", "first-rate limiters"]);
        assert_eq!(mention(&hay, &rate), None);
        // A name never spans two texts.
        let hay = Haystack::new(&["…at the rate", "limiter later"]);
        assert_eq!(mention(&hay, &rate), None);
    }

    #[test]
    fn short_and_common_names_are_skipped() {
        assert_eq!(name_words("api"), None);
        assert_eq!(name_words("ui"), None);
        assert_eq!(name_words("main"), None);
        assert_eq!(name_words("Meeting notes"), None);
        assert_eq!(name_words("Release planning"), None);
        assert_eq!(name_words("2026"), None);
        assert_eq!(
            name_words("Release checklist"),
            Some(vec!["release".to_string(), "checklist".to_string()])
        );
        assert_eq!(name_words("auth").map(|w| w.len()), Some(1));
        let main = cand("session", "s", &["main"], "");
        assert_eq!(mention(&Haystack::new(&["pushed to main"]), &main), None);
        let notes = cand("note", "n", &["Notes"], "");
        assert_eq!(
            mention(&Haystack::new(&["my notes are here"]), &notes),
            None
        );
    }

    #[test]
    fn pr_and_issue_numbers_are_shared_references() {
        let r = refs("Lena's migration guide is in PR 412; see issue #88, pull/77 and #1234.");
        assert_eq!(r.get(&412).map(String::as_str), Some("PR 412"));
        assert_eq!(r.get(&88).map(String::as_str), Some("issue 88"));
        assert_eq!(r.get(&77).map(String::as_str), Some("PR 77"));
        assert_eq!(r.get(&1234).map(String::as_str), Some("#1234"));
        // A Slack channel, a heading and a color are not references.
        assert!(refs("#release ## Notes &#39; color #fff").is_empty());
        let note = cand(
            "note",
            "n1",
            &["Migration guide"],
            "Tracks pr #412 until it merges.",
        );
        let hay = Haystack::new(&["PR 412 is up"]);
        assert_eq!(mention(&hay, &note).as_deref(), Some("mentions PR 412"));
        let hay = Haystack::new(&["PR 413 is up"]);
        assert_eq!(mention(&hay, &note), None);
    }

    fn pick(kind: &str, id: &str, explicit: bool) -> Pick {
        Pick {
            link: ItemLink {
                kind: kind.into(),
                id: id.into(),
                title: id.into(),
                why: if explicit {
                    "linked by you"
                } else {
                    "mentions x"
                }
                .into(),
                by_user: explicit,
                state: None,
            },
            explicit,
        }
    }

    #[test]
    fn suggestions_cap_at_five_per_kind_explicit_first_and_kept() {
        let mut picks: Vec<Pick> = (0..8)
            .map(|i| pick("note", &format!("n{i}"), false))
            .collect();
        picks.push(pick("note", "n-hand", true));
        picks.push(pick("note", "n3", true));
        picks.push(pick("message", "m1", false));
        picks.insert(0, pick("session", "s1", false));
        let out = assemble(picks, &HashSet::new());
        let ids: Vec<&str> = out.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["s1", "n-hand", "n3", "n0", "n1", "n2", "m1"]);
        assert_eq!(
            out[2].why, "linked by you",
            "explicit wins over the same suggestion"
        );
        // Seven explicit notes: none dropped, no suggestion added.
        let mut many: Vec<Pick> = (0..7)
            .map(|i| pick("note", &format!("e{i}"), true))
            .collect();
        many.push(pick("note", "x", false));
        assert_eq!(assemble(many, &HashSet::new()).len(), 7);
    }

    #[test]
    fn dismissed_suggestions_stay_away_explicit_links_do_not() {
        let picks = vec![
            pick("note", "n1", false),
            pick("note", "n2", false),
            pick("session", "s1", true),
        ];
        let gone: HashSet<(String, String)> = [
            ("note".to_string(), "n1".to_string()),
            ("session".to_string(), "s1".to_string()),
        ]
        .into();
        let ids: Vec<String> = assemble(picks, &gone).into_iter().map(|l| l.id).collect();
        assert_eq!(ids, ["s1", "n2"]);
    }

    #[test]
    fn meaningful_words_skip_function_and_meeting_words() {
        let t = meaningful("Re: Weekly sync about the releases");
        assert_eq!(t, ["release".to_string()].into_iter().collect());
        assert!(meaningful("#release can we ship v2").contains("release"));
        assert!(meaningful("1:1").is_empty());
    }

    #[test]
    fn message_titles() {
        assert_eq!(
            message_title("Sam", None, Some("#release"), "can we ship"),
            "Sam · #release"
        );
        assert_eq!(
            message_title("Priya", Some("Re: API v2 rollout"), None, "x"),
            "Priya · Re: API v2 rollout"
        );
        assert_eq!(
            message_title("Tom", None, Some(""), "quick one\nmore"),
            "Tom · quick one"
        );
        assert_eq!(
            attendee_emails(
                r#"[{"email":"Sam@Acme.com"},{"email":"me@acme.com","self":true},{"name":"x"}]"#
            ),
            ["sam@acme.com"]
        );
    }
}
