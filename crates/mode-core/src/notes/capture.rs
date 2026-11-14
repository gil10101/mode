//! Capture (04 §7): the token grammar of §7.2 read from the first line
//! only, and every capture path of §7.1 that the core owns (hotkey and
//! capture box, drag-to-overlay, note from session / message, prepare
//! notes, ⊕ Add task). Each path writes one `.md` file atomically, indexes
//! it in the same writer job with a `note_revisions` row (origin `mode`)
//! and returns the new id; the caller pushes `notes.changed` (§7.4).

use serde::{Deserialize, Serialize};
use time::Date;

use super::ctx::{Cx, invalid};
use super::fields::{self, WeekStart};
use super::reconcile;
use super::vault;
use super::write;
use crate::db::Result;
use crate::repo::{self, Note};

/// The longest title a capture keeps (04 §2.4).
pub const MAX_TITLE_CHARS: usize = 120;

/// What the first line's tokens said (04 §7.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    /// The first line with its tokens removed (`Untitled` when nothing is left).
    pub title: String,
    pub tags: Vec<String>,
    pub urgent: bool,
    /// The resolved local date.
    pub when: Option<String>,
    pub status: Option<String>,
    /// `(folder id, path)` from `in:Path`.
    pub folder: Option<(String, String)>,
    /// Why a token was refused (`@2026-02-30`), shown under the box.
    pub hints: Vec<String>,
}

/// What the grammar needs to know.
#[derive(Debug, Clone, Default)]
pub struct TokenCtx {
    pub today: Option<Date>,
    pub week_start: WeekStart,
    /// Status slugs from `settings['notes.statuses']`.
    pub statuses: Vec<String>,
    /// Live folders `(id, path)`.
    pub folders: Vec<(String, String)>,
}

impl TokenCtx {
    pub fn from_cx(cx: &Cx) -> Result<TokenCtx> {
        let mut stmt = cx
            .conn
            .prepare("SELECT id, path FROM folders WHERE deleted_at IS NULL ORDER BY path")?;
        let folders = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(TokenCtx {
            today: Some(cx.today()),
            week_start: cx.week_start,
            statuses: cx.statuses().iter().map(|s| s.slug.clone()).collect(),
            folders,
        })
    }
}

fn is_tag_word(w: &str) -> Option<String> {
    let rest = w.strip_prefix('#')?;
    let n = rest.chars().count();
    if n == 0
        || n > 64
        || rest
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '#' | ',' | '[' | ']' | '"'))
    {
        return None;
    }
    fields::tag_display(rest)
}

fn is_iso_shape(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Parse the capture token grammar on one line (04 §7.2). Order is free;
/// unknown `/slug`, `in:` paths and `@words` stay text; `\#`, `\@`, `\/`,
/// `\!` are literal.
pub fn parse_tokens(line: &str, ctx: &TokenCtx) -> Tokens {
    let mut t = Tokens::default();
    // A Markdown heading marker on a dropped line is not a tag.
    let mut line = line.trim();
    if let Some(rest) = line.strip_prefix('#') {
        let rest2 = rest.trim_start_matches('#');
        if rest2.starts_with(' ') {
            line = rest2.trim_start();
        }
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let last = words.len().saturating_sub(1);
    let mut kept: Vec<String> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let w = *w;
        if let Some(rest) = w.strip_prefix('\\')
            && rest.starts_with(['#', '@', '/', '!'])
        {
            kept.push(rest.to_string());
            continue;
        }
        if w == "!" {
            t.urgent = true;
            continue;
        }
        if let Some(tag) = is_tag_word(w) {
            let norm = tag.to_lowercase();
            if !t.tags.iter().any(|x| x.to_lowercase() == norm) {
                t.tags.push(tag);
            }
            continue;
        }
        if let Some(rest) = w.strip_prefix('@')
            && let Some(today) = ctx.today
        {
            let lower = rest.to_ascii_lowercase();
            let date = match lower.as_str() {
                "today" => Some(today),
                "tomorrow" => Some(today + time::Duration::days(1)),
                _ if is_iso_shape(rest) => match fields::parse_date(rest) {
                    Some(d) => Some(d),
                    None => {
                        t.hints.push(format!("{rest} is not a date"));
                        None
                    }
                },
                _ => fields::weekday_from_name(&lower).map(|wd| fields::next_weekday(today, wd)),
            };
            if let Some(d) = date {
                t.when = Some(fields::fmt_date(d));
                continue;
            }
        }
        if let Some(slug) = w.strip_prefix('/')
            && fields::is_slug(slug)
            && ctx.statuses.iter().any(|s| s == slug)
        {
            t.status = Some(slug.to_string());
            continue;
        }
        if let Some(path) = w.strip_prefix("in:")
            && !path.is_empty()
        {
            let want = path.trim_matches('/').to_lowercase();
            if let Some((id, p)) = ctx.folders.iter().find(|(_, p)| p.to_lowercase() == want) {
                t.folder = Some((id.clone(), p.clone()));
                continue;
            }
        }
        if i == last && w.len() > 1 && w.ends_with('!') && !w.ends_with("\\!") {
            t.urgent = true;
            kept.push(w[..w.len() - 1].to_string());
            continue;
        }
        kept.push(w.to_string());
    }
    let title: String = kept.join(" ").chars().take(MAX_TITLE_CHARS).collect();
    t.title = if title.trim().is_empty() {
        "Untitled".to_string()
    } else {
        title.trim().to_string()
    };
    t
}

/// The item a capture was made from, kept as a `created_from` link (§7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedFrom {
    /// `session` | `message` (| `thread` | `calendar_event`).
    pub kind: String,
    pub id: String,
}

/// What a capture produced.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Captured {
    pub note_id: String,
    pub path: String,
    pub title: String,
    pub hints: Vec<String>,
}

fn captured(note: &Note, hints: Vec<String>) -> Captured {
    Captured {
        note_id: note.id.clone(),
        path: note.path.clone(),
        title: note.title.clone(),
        hints,
    }
}

/// `# Title`, then the remaining lines after one blank line.
fn body_with(title: &str, rest: &str) -> String {
    let rest = rest.trim_matches(['\n', '\r']);
    if rest.trim().is_empty() {
        format!("# {title}\n")
    } else {
        format!("# {title}\n\n{rest}\n")
    }
}

/// Paths 1, 2, 8 (and the text payload of 3): the capture box, the hotkey,
/// ⊕ Add task. Tokens on the first line set the fields; the remaining
/// lines are the body, never interpreted.
pub fn capture_text(cx: &Cx, text: &str, created_from: Option<&CreatedFrom>) -> Result<Captured> {
    // Tokens are read line by line from `\n` text; the file is a new one
    // (`super::eol::NEW_FILE`).
    let text = super::eol::normalize(text, super::eol::NEW_FILE);
    let (first, rest) = text.split_once('\n').unwrap_or((text.as_ref(), ""));
    if first.trim().is_empty() && rest.trim().is_empty() {
        return Err(invalid("capture", "nothing to capture"));
    }
    let ctx = TokenCtx::from_cx(cx)?;
    let t = parse_tokens(first, &ctx);
    let dir = match &t.folder {
        Some((_, p)) => p.clone(),
        None => reconcile::capture_folder(cx)?.1,
    };
    let mut c = write::new_canonical(cx);
    c.status = t.status.clone().unwrap_or_else(|| "todo".into());
    c.urgent = t.urgent;
    c.when = t.when.clone();
    c.tags = t.tags.clone();
    let note = write::create_file(
        cx,
        &dir,
        &t.title,
        &c,
        &body_with(&t.title, rest),
        "capture",
    )?;
    if let Some(from) = created_from {
        let kind = match from.kind.as_str() {
            "session" | "message" | "thread" | "calendar_event" => from.kind.as_str(),
            other => {
                return Err(invalid(
                    "created_from",
                    format!("{other:?} is not a capture source"),
                ));
            }
        };
        write::link(
            cx,
            ("note", &note.id),
            (kind, &from.id),
            "created_from",
            None,
            "system",
        )?;
    }
    Ok(captured(&note, t.hints))
}

/// Path 3 for URLs and file paths (§7.1): the title is the URL host + path
/// or the first file name; the body lists the links; one `attachment`
/// link per item.
pub fn capture_links(cx: &Cx, items: &[String]) -> Result<Captured> {
    let items: Vec<&str> = items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let first = items
        .first()
        .ok_or_else(|| invalid("capture", "nothing to capture"))?;
    let url_of = |s: &str| -> String {
        if s.contains("://") {
            s.to_string()
        } else {
            format!("file://{}", s.replace('\\', "/"))
        }
    };
    let title = if let Some(rest) = first.split_once("://").map(|(_, r)| r) {
        if first.starts_with("file://") {
            rest.rsplit('/').next().unwrap_or(rest).to_string()
        } else {
            rest.split(['?', '#'])
                .next()
                .unwrap_or(rest)
                .trim_end_matches('/')
                .to_string()
        }
    } else {
        std::path::Path::new(first)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| first.to_string())
    };
    let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
    let list: Vec<String> = items.iter().map(|i| format!("- <{}>", url_of(i))).collect();
    let (_, dir) = reconcile::capture_folder(cx)?;
    let c = write::new_canonical(cx);
    let note = write::create_file(
        cx,
        &dir,
        &title,
        &c,
        &body_with(&title, &list.join("\n")),
        "capture",
    )?;
    for i in &items {
        let url = url_of(i);
        write::link(
            cx,
            ("note", &note.id),
            ("url", &url),
            "attachment",
            Some(i),
            "user",
        )?;
    }
    Ok(captured(&note, Vec::new()))
}

/// Path 4, note from session (§7.1): titled with the session's display
/// name; a short header block with the repo, branch and session id —
/// never prompt or assistant text — then an empty line for typing.
pub fn note_from_session(cx: &Cx, session_id: &str) -> Result<Captured> {
    let s = repo::get_by_id::<repo::Session>(cx.conn, session_id)?.ok_or_else(|| {
        crate::db::DbError::NotFound {
            table: "sessions",
            key: session_id.to_string(),
        }
    })?;
    let mut lines = vec![format!(
        "**Repo:** {}",
        s.repo_root.as_deref().unwrap_or(&s.cwd)
    )];
    if let Some(b) = &s.git_branch {
        lines.push(format!("**Branch:** {b}"));
    }
    lines.push(format!("**Session:** {}", s.claude_session_id));
    let title = if s.display_name.trim().is_empty() {
        "Session".to_string()
    } else {
        s.display_name.clone()
    };
    let (_, dir) = reconcile::capture_folder(cx)?;
    let c = write::new_canonical(cx);
    let note = write::create_file(
        cx,
        &dir,
        &title,
        &c,
        &format!("# {title}\n\n{}\n\n", lines.join("  \n")),
        "session",
    )?;
    write::link(
        cx,
        ("note", &note.id),
        ("session", session_id),
        "created_from",
        None,
        "system",
    )?;
    Ok(captured(&note, Vec::new()))
}

/// Path 5, note from message (§7.1, v0.2): the subject (else the first 80
/// characters of the snippet); a blockquote with the sender, the time, the
/// snippet (≤ 500 characters) and the source link, never the full body.
pub fn note_from_message(cx: &Cx, message_id: &str) -> Result<Captured> {
    let m = repo::get_by_id::<repo::Message>(cx.conn, message_id)?.ok_or_else(|| {
        crate::db::DbError::NotFound {
            table: "messages",
            key: message_id.to_string(),
        }
    })?;
    let title = match m
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => m.snippet.chars().take(80).collect(),
    };
    let snippet: String = m.snippet.chars().take(500).collect();
    let quote = format!(
        "> **{}** · {}\n>\n> {}\n>\n> [Open]({})",
        m.sender_name,
        m.received_at,
        snippet.replace('\n', "\n> "),
        m.source_url
    );
    let (_, dir) = reconcile::capture_folder(cx)?;
    let c = write::new_canonical(cx);
    let note = write::create_file(
        cx,
        &dir,
        &title,
        &c,
        &format!("# {title}\n\n{quote}\n"),
        "message",
    )?;
    write::link(
        cx,
        ("note", &note.id),
        ("message", message_id),
        "created_from",
        None,
        "system",
    )?;
    if let Some(t) = &m.thread_id {
        write::link(
            cx,
            ("note", &note.id),
            ("thread", t),
            "created_from",
            None,
            "system",
        )?;
    }
    Ok(captured(&note, Vec::new()))
}

/// Path 6, prepare notes (§7.1, v0.2): date, time, location, join link and
/// attendees as a list, then `## Notes`; a `prepare_for` link from the event.
pub fn prepare_notes(cx: &Cx, event_id: &str) -> Result<Captured> {
    let e = repo::get_by_id::<repo::CalendarEvent>(cx.conn, event_id)?.ok_or_else(|| {
        crate::db::DbError::NotFound {
            table: "calendar_events",
            key: event_id.to_string(),
        }
    })?;
    let tz = cx.tz();
    let local = |s: &str| crate::db::parse_timestamp(s).map(|t| tz.to_local(t));
    let mut lines = Vec::new();
    if let (Some(a), Some(b)) = (local(&e.starts_at), local(&e.ends_at)) {
        lines.push(format!("- **Date:** {}", fields::fmt_date(a.date())));
        if !e.all_day {
            lines.push(format!(
                "- **Time:** {:02}:{:02} – {:02}:{:02}",
                a.hour(),
                a.minute(),
                b.hour(),
                b.minute()
            ));
        }
    }
    if let Some(l) = e.location.as_deref().filter(|l| !l.trim().is_empty()) {
        lines.push(format!("- **Location:** {l}"));
    }
    if let Some(j) = &e.join_url {
        lines.push(format!("- **Join:** <{j}>"));
    }
    let attendees: Vec<String> = serde_json::from_str::<serde_json::Value>(&e.attendees_json)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|a| {
            a.get("name")
                .or_else(|| a.get("email"))
                .and_then(|v| v.as_str())
                .or_else(|| a.as_str())
                .map(str::to_string)
        })
        .collect();
    if !attendees.is_empty() {
        lines.push("- **Attendees:**".into());
        for a in attendees {
            lines.push(format!("  - {a}"));
        }
    }
    let title = e.title.clone();
    let (_, dir) = reconcile::capture_folder(cx)?;
    let c = write::new_canonical(cx);
    let note = write::create_file(
        cx,
        &dir,
        &title,
        &c,
        &format!("# {title}\n\n{}\n\n## Notes\n", lines.join("\n")),
        "calendar",
    )?;
    write::link(
        cx,
        ("calendar_event", event_id),
        ("note", &note.id),
        "prepare_for",
        None,
        "system",
    )?;
    Ok(captured(&note, Vec::new()))
}

/// The capture folder must be writable and inside the vault.
pub fn capture_dir(cx: &Cx) -> Result<String> {
    let (_, p) = reconcile::capture_folder(cx)?;
    if !vault::abs_path(&cx.env.root, &p).is_dir() {
        return Err(invalid("capture", "the capture folder is missing"));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> TokenCtx {
        TokenCtx {
            today: fields::parse_date("2026-10-18"),
            week_start: WeekStart::Monday,
            statuses: vec![
                "todo".into(),
                "doing".into(),
                "done".into(),
                "waiting".into(),
            ],
            folders: vec![("f1".into(), "Projects/mode".into())],
        }
    }

    #[test]
    fn the_spec_example() {
        let t = parse_tokens("Call the plumber @tomorrow #home !", &ctx());
        assert_eq!(t.title, "Call the plumber");
        assert_eq!(t.when.as_deref(), Some("2026-10-19"));
        assert_eq!(t.tags, vec!["home"]);
        assert!(t.urgent);
        assert!(t.hints.is_empty());
    }

    #[test]
    fn every_token() {
        let t = parse_tokens(
            "#gym #Gym #monday Leg day @fri /doing in:projects/MODE",
            &ctx(),
        );
        assert_eq!(t.tags, vec!["gym", "monday"]);
        assert_eq!(t.title, "Leg day");
        assert_eq!(t.when.as_deref(), Some("2026-10-23"));
        assert_eq!(t.status.as_deref(), Some("doing"));
        assert_eq!(t.folder, Some(("f1".into(), "Projects/mode".into())));
        let t = parse_tokens("Pay the deposit!", &ctx());
        assert!(t.urgent);
        assert_eq!(t.title, "Pay the deposit");
        let t = parse_tokens(r"\#notatag \@home \/doing a\! b", &ctx());
        assert_eq!(t.title, r"#notatag @home /doing a\! b");
        assert!(!t.urgent && t.tags.is_empty());
        let t = parse_tokens("x @2026-02-30 /blocked in:Nowhere @sun", &ctx());
        assert_eq!(t.title, "x @2026-02-30 /blocked in:Nowhere");
        assert_eq!(t.hints, vec!["2026-02-30 is not a date"]);
        assert_eq!(t.when.as_deref(), Some("2026-10-18"), "today included");
        let t = parse_tokens("#gym @today", &ctx());
        assert_eq!(t.title, "Untitled");
        let t = parse_tokens("## Heading #a", &ctx());
        assert_eq!(t.title, "Heading");
        assert_eq!(t.tags, vec!["a"]);
        let t = parse_tokens("tags #a,b", &ctx());
        assert_eq!(t.title, "tags #a,b");
        assert_eq!(
            parse_tokens(&"w ".repeat(100), &ctx())
                .title
                .chars()
                .count(),
            120 - 1
        );
    }
}
