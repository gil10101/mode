//! The in-memory palette index and its static parts (views, commands,
//! Settings pages and keys), and its dynamic rows: sessions, notes,
//! messages, calendar events, Chat conversations, automations and the files
//! sessions touched. An invalidation rebuilds it on a background thread
//! while queries keep answering from the previous index.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::db::{Db, Result, parse_timestamp};

/// The index is rebuilt when older than this even without an invalidation.
pub const MAX_AGE: Duration = Duration::from_secs(120);

/// Files indexed (the most recently touched).
pub const MAX_FILES: i64 = 2_000;
/// Calendar events indexed: this far back and ahead.
pub const EVENTS_BACK_DAYS: i64 = 30;
pub const EVENTS_AHEAD_DAYS: i64 = 180;

/// What a palette row is (`PaletteKind` in `palette-store.ts`, plus
/// `command` and `file`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Command,
    Session,
    Note,
    Message,
    Event,
    Chat,
    Automation,
    Setting,
    File,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Command,
        Kind::Session,
        Kind::Note,
        Kind::Message,
        Kind::Event,
        Kind::Chat,
        Kind::Automation,
        Kind::Setting,
        Kind::File,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Command => "command",
            Kind::Session => "session",
            Kind::Note => "note",
            Kind::Message => "message",
            Kind::Event => "event",
            Kind::Chat => "chat",
            Kind::Automation => "automation",
            Kind::Setting => "setting",
            Kind::File => "file",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "command" => Kind::Command,
            "session" => Kind::Session,
            "note" => Kind::Note,
            "message" => Kind::Message,
            "event" | "calendar_event" => Kind::Event,
            "chat" => Kind::Chat,
            "automation" => Kind::Automation,
            "setting" | "settings" => Kind::Setting,
            "file" => Kind::File,
            _ => return None,
        })
    }
}

/// One indexed row.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    pub id: String,
    pub title: String,
    pub detail: String,
    pub route: String,
    /// Lower-case title.
    pub title_lc: String,
    /// Byte offsets where a word starts in `title_lc`.
    pub starts: Vec<u16>,
    /// Lower-case detail line plus aliases.
    pub keywords_lc: String,
    /// Unix seconds of the last activity (0 = static rows).
    pub ts: i64,
    /// [`char_mask`] of the title and of the keywords: a query whose
    /// letters are not all in one of them cannot match (the fast reject).
    pub title_mask: u64,
    pub keywords_mask: u64,
}

/// The letters and digits a text contains as a bitset (a–z, 0–9, one bit
/// for anything beyond ASCII); spaces and punctuation are ignored.
pub fn char_mask(text: &str) -> u64 {
    let mut m = 0u64;
    for b in text.bytes() {
        m |= match b {
            b'a'..=b'z' => 1 << (b - b'a'),
            b'A'..=b'Z' => 1 << (b - b'A'),
            b'0'..=b'9' => 1 << (26 + b - b'0'),
            0x80.. => 1 << 36,
            _ => 0,
        };
    }
    m
}

fn word_starts(s: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let mut prev_alnum = false;
    for (i, c) in s.char_indices() {
        let alnum = c.is_alphanumeric();
        if alnum && !prev_alnum && i < u16::MAX as usize {
            out.push(i as u16);
        }
        prev_alnum = alnum;
    }
    out
}

impl Entry {
    pub fn new(
        kind: Kind,
        id: &str,
        title: &str,
        detail: &str,
        route: &str,
        keywords: &str,
        ts: i64,
    ) -> Entry {
        let title_lc = title.to_lowercase();
        let keywords_lc = format!("{} {}", detail, keywords).to_lowercase();
        Entry {
            kind,
            id: id.into(),
            title: title.into(),
            detail: detail.into(),
            route: route.into(),
            starts: word_starts(&title_lc),
            title_mask: char_mask(&title_lc),
            keywords_mask: char_mask(&keywords_lc),
            title_lc,
            keywords_lc,
            ts,
        }
    }
}

/// The views of the sidebar (03 §2.3) as "Go to" commands.
const VIEWS: [(&str, &str); 10] = [
    ("Sessions", "sessions"),
    ("Inbox", "inbox"),
    ("Notes", "notes"),
    ("Calendar", "calendar"),
    ("Chat", "chat"),
    ("Automations", "automations"),
    ("Integrations", "integrations"),
    ("Shortcuts", "shortcuts"),
    ("Stats", "stats"),
    ("Settings", "settings/general"),
];

/// `(id, title, route, keywords)` of the commands.
const COMMANDS: [(&str, &str, &str, &str); 9] = [
    ("note.new", "New note", "notes", "create add write"),
    (
        "session.launch",
        "Launch session",
        "sessions",
        "start claude code new terminal",
    ),
    (
        "chat.new",
        "New conversation",
        "chat",
        "ask assistant question",
    ),
    (
        "automation.new",
        "New automation",
        "automations",
        "schedule cron create",
    ),
    (
        "theme.import",
        "Import theme pack",
        "settings/appearance",
        "theme colors pack json",
    ),
    (
        "hooks.install",
        "Install hooks",
        "integrations/claude-code/install",
        "claude code relay",
    ),
    (
        "settings.export",
        "Export settings",
        "settings/advanced/export-import",
        "backup bundle",
    ),
    (
        "diagnostics.open",
        "Open diagnostics",
        "settings/advanced/diagnostics",
        "logs debug",
    ),
    (
        "account.connect",
        "Connect an account",
        "integrations",
        "gmail slack google calendar sign in",
    ),
];

/// `(page, section, title, keywords)` of the Settings pages and sections (03 §8.1).
const SETTINGS_PAGES: [(&str, &str, &str, &str); 17] = [
    (
        "general",
        "",
        "General",
        "profile startup launch at login updates language rows",
    ),
    (
        "general",
        "sessions",
        "Sessions",
        "fold dismiss terminal app",
    ),
    (
        "appearance",
        "",
        "Appearance",
        "theme dark light mode pack colors font size",
    ),
    (
        "overlay",
        "",
        "Overlay",
        "notch top bar right edge placement pill hover",
    ),
    ("alerts", "", "Alerts", "notifications sounds rules"),
    (
        "alerts",
        "quiet-hours",
        "Quiet hours",
        "do not disturb focus",
    ),
    ("notes", "", "Notes", "vault folder capture statuses"),
    ("notes", "groupings", "Groupings", "board columns filters"),
    ("notes", "tags", "Tags", "labels"),
    (
        "urgency",
        "",
        "Urgency",
        "important people contacts keyword rules scoring",
    ),
    (
        "ai",
        "",
        "AI",
        "api key backend local model ollama lm studio budget tokens",
    ),
    ("privacy", "", "Privacy", "retention local-only data"),
    (
        "advanced",
        "",
        "Advanced",
        "diagnostics logs export import about",
    ),
    (
        "advanced",
        "diagnostics",
        "Diagnostics",
        "ipc database counters",
    ),
    ("advanced", "logs", "Logs", "log file"),
    (
        "advanced",
        "export-import",
        "Export & Import",
        "backup bundle restore",
    ),
    ("advanced", "about", "About", "version licenses"),
];

/// Where a settings key lives (03 §8.1; Hotkeys → Shortcuts, connectors → Integrations).
pub fn key_route(key: &str) -> Option<(&'static str, &'static str)> {
    let (prefix, _) = key.split_once('.')?;
    Some(match (prefix, key) {
        (
            _,
            "capture.hotkey"
            | "overlay.toggle_hotkey"
            | "overlay.pin_hotkey"
            | "window.hotkey"
            | "shortcuts.bindings",
        ) => ("shortcuts", "Shortcuts"),
        ("device" | "maintenance" | "window" | "onboarding" | "cards", _) => return None,
        ("general" | "profile" | "sessions", _) => ("settings/general", "General"),
        ("theme" | "sounds", _) => ("settings/appearance", "Appearance"),
        ("overlay" | "capture", _) => ("settings/overlay", "Overlay"),
        ("notifications", _) => ("settings/alerts", "Alerts"),
        ("notes", _) => ("settings/notes", "Notes"),
        ("urgency" | "inbox", _) => ("settings/urgency", "Urgency"),
        ("ai", _) => ("settings/ai", "AI"),
        ("retention" | "usage", _) => ("settings/privacy", "Privacy"),
        ("polling" | "gmail" | "slack" | "connectors", _) => ("integrations", "Integrations"),
        ("hooks", _) => ("integrations/claude-code", "Integrations"),
        ("automations", _) => ("automations", "Automations"),
        _ => return None,
    })
}

/// "theme.font_scale" → "Font scale".
pub fn humanise(key: &str) -> String {
    let last = key.rsplit('.').next().unwrap_or(key).replace('_', " ");
    let mut c = last.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn statics() -> Vec<Entry> {
    let mut out = Vec::new();
    for (name, route) in VIEWS {
        out.push(Entry::new(
            Kind::Command,
            &format!("view.{}", route.split('/').next().unwrap_or(route)),
            &format!("Go to {name}"),
            "View",
            route,
            name,
            0,
        ));
    }
    for (id, title, route, kw) in COMMANDS {
        out.push(Entry::new(
            Kind::Command,
            id,
            title,
            "Command",
            route,
            kw,
            0,
        ));
    }
    for (page, section, title, kw) in SETTINGS_PAGES {
        let route = if section.is_empty() {
            format!("settings/{page}")
        } else {
            format!("settings/{page}/{section}")
        };
        let detail = if section.is_empty() {
            "Settings".to_string()
        } else {
            format!("Settings · {}", humanise(page))
        };
        out.push(Entry::new(
            Kind::Setting,
            &route,
            title,
            &detail,
            &route,
            kw,
            0,
        ));
    }
    for k in crate::settings::KEYS {
        if crate::settings::INTERNAL_KEYS.contains(&k.key) {
            continue;
        }
        let Some((route, page)) = key_route(k.key) else {
            continue;
        };
        out.push(Entry::new(
            Kind::Setting,
            k.key,
            &humanise(k.key),
            &format!("{page} · {}", k.key),
            route,
            &k.key.replace(['.', '_'], " "),
            0,
        ));
    }
    out
}

fn ts(s: &str) -> i64 {
    parse_timestamp(s).map(|t| t.unix_timestamp()).unwrap_or(0)
}

/// "Wed 21 Oct 10:00" (all-day: "Wed 21 Oct · all day") in `tz`.
fn event_when(starts_at: &str, all_day: bool, start_date: Option<&str>, tz: &str) -> String {
    let zone = jiff::tz::TimeZone::get(tz).unwrap_or(jiff::tz::TimeZone::UTC);
    if all_day && let Some(d) = start_date.and_then(|d| d.parse::<jiff::civil::Date>().ok()) {
        return format!("{} · all day", d.strftime("%a %-d %b"));
    }
    match parse_timestamp(starts_at)
        .and_then(|t| jiff::Timestamp::from_second(t.unix_timestamp()).ok())
    {
        Some(t) => t.to_zoned(zone).strftime("%a %-d %b %H:%M").to_string(),
        None => starts_at.to_string(),
    }
}

/// Characters of a message's subject or snippet kept in its title.
pub const MESSAGE_PREVIEW_CHARS: usize = 80;

/// A message row's title, as the kit's `MsgRow` and the wireframe's
/// palette row read ("Sam · #release · can we ship v2 behind the flag
/// today?"): the sender, then the chat (03 §4.2: `#channel` for a Slack
/// channel or thread, `Inbox` for mail, nothing for a DM or group DM),
/// then the subject (mail) or the snippet's first line.
pub fn message_title(
    sender: &str,
    kind: &str,
    channel: Option<&str>,
    subject: Option<&str>,
    snippet: &str,
) -> String {
    let chat = match kind {
        "email" => Some("Inbox".to_string()),
        "channel" | "channel_thread" => channel
            .map(|c| c.trim().trim_start_matches('#'))
            .filter(|c| !c.is_empty())
            .map(|c| format!("#{c}")),
        _ => None,
    };
    let text = subject
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| snippet.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("");
    let text: String = text.chars().take(MESSAGE_PREVIEW_CHARS).collect();
    [
        sender.trim(),
        chat.as_deref().unwrap_or(""),
        text.trim_end(),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// When a message arrived, as rows show times (10 §12.1): `now`, `4m`,
/// `2h` within a day, then the date in `tz` (`18 Oct`, `21 Oct 2025` in
/// another year). `ts` and `now` are Unix seconds; empty when unknown.
pub fn received_label(ts: i64, now: i64, tz: &str) -> String {
    if ts <= 0 {
        return String::new();
    }
    let s = (now - ts).max(0);
    if s < 60 {
        return "now".into();
    }
    if s < 3_600 {
        return format!("{}m", s / 60);
    }
    if s < 86_400 {
        return format!("{}h", s / 3_600);
    }
    let zone = jiff::tz::TimeZone::get(tz).unwrap_or(jiff::tz::TimeZone::UTC);
    let (Ok(at), Ok(today)) = (
        jiff::Timestamp::from_second(ts),
        jiff::Timestamp::from_second(now),
    ) else {
        return String::new();
    };
    let (at, today) = (at.to_zoned(zone.clone()), today.to_zoned(zone));
    if at.year() == today.year() {
        at.strftime("%-d %b").to_string()
    } else {
        at.strftime("%-d %b %Y").to_string()
    }
}

/// Read every row (static and dynamic) as of `now`, event times in `tz`.
pub fn load(conn: &Connection) -> Result<Vec<Entry>> {
    load_at(
        conn,
        OffsetDateTime::now_utc(),
        &crate::scheduler::schedule::device_zone(),
    )
}

/// [`load`] with a clock and a zone.
pub fn load_at(conn: &Connection, now: OffsetDateTime, tz: &str) -> Result<Vec<Entry>> {
    let mut out = statics();
    let mut stmt = conn.prepare_cached(
        "SELECT id, display_name, COALESCE(title, ''), COALESCE(repo_root, cwd), state, last_event_at
           FROM sessions WHERE parent_session_id IS NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        let (id, name, title, repo, state, at): (String, String, String, String, String, String) = (
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        );
        Ok(Entry::new(
            Kind::Session,
            &id,
            &name,
            &format!("{state} · {repo}"),
            &format!("sessions/{id}"),
            &title,
            ts(&at),
        ))
    })?;
    for r in rows {
        out.push(r?);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT id, title, path, status, updated_at FROM notes
          WHERE deleted_at IS NULL AND archived_at IS NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        let (id, title, path, status, at): (String, String, String, String, String) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
        let folder = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let detail = if folder.is_empty() {
            status
        } else {
            format!("{folder}/ · {status}")
        };
        Ok(Entry::new(
            Kind::Note,
            &id,
            &title,
            &detail,
            &format!("notes/{id}"),
            "",
            ts(&at),
        ))
    })?;
    for r in rows {
        out.push(r?);
    }
    // Messages: "sender · chat · subject or snippet"; the received time is
    // the detail (rendered again at query time, `palette_query`), kept out
    // of the keywords.
    let mut stmt = conn.prepare_cached(
        "SELECT id, sender_name, kind, channel_name, subject, snippet, received_at
           FROM messages WHERE is_archived = 0",
    )?;
    let now_s = now.unix_timestamp();
    let rows = stmt.query_map([], |r| {
        let (id, sender, kind, channel, subject, snippet, at): (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
        ) = (
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        );
        let at = ts(&at);
        let title = message_title(
            &sender,
            &kind,
            channel.as_deref(),
            subject.as_deref(),
            &snippet,
        );
        let mut e = Entry::new(
            Kind::Message,
            &id,
            &title,
            "",
            &format!("inbox/message/{id}"),
            "",
            at,
        );
        e.detail = received_label(at, now_s, tz);
        Ok(e)
    })?;
    for r in rows {
        out.push(r?);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT id, name, schedule_json, updated_at FROM automations WHERE deleted_at IS NULL",
    )?;
    let device_tz = crate::scheduler::schedule::device_zone();
    let rows = stmt.query_map([], |r| {
        let (id, name, sched, at): (String, String, String, String) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        let phrase = serde_json::from_str::<crate::scheduler::schedule::Schedule>(&sched)
            .map(|s| crate::scheduler::schedule::trigger_phrase(&s, &device_tz))
            .unwrap_or_default();
        Ok(Entry::new(
            Kind::Automation,
            &id,
            &name,
            &format!("Automation · {phrase}"),
            &format!("automations/{id}"),
            "",
            ts(&at),
        ))
    })?;
    for r in rows {
        out.push(r?);
    }
    // Calendar events around now.
    let from = crate::db::format_timestamp(now - time::Duration::days(EVENTS_BACK_DAYS));
    let to = crate::db::format_timestamp(now + time::Duration::days(EVENTS_AHEAD_DAYS));
    let mut stmt = conn.prepare_cached(
        "SELECT id, title, starts_at, all_day, start_date, COALESCE(location, ''), COALESCE(calendar_name, '')
           FROM calendar_events
          WHERE status <> 'cancelled' AND starts_at >= ?1 AND starts_at <= ?2",
    )?;
    let rows = stmt.query_map([&from, &to], |r| {
        let (id, title, starts, all_day, date, location, cal): (
            String,
            String,
            String,
            bool,
            Option<String>,
            String,
            String,
        ) = (
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        );
        Ok(Entry::new(
            Kind::Event,
            &id,
            &title,
            &event_when(&starts, all_day, date.as_deref(), tz),
            &format!("calendar/{id}"),
            &format!("{location} {cal}"),
            ts(&starts),
        ))
    })?;
    for r in rows {
        out.push(r?);
    }
    // Chat conversations: the title record, else the first question.
    let mut stmt = conn.prepare_cached(
        "SELECT c.conversation_id, MAX(c.created_at),
                (SELECT t.content FROM chat_turns t WHERE t.id = c.conversation_id || ':title'),
                (SELECT u.content FROM chat_turns u
                  WHERE u.conversation_id = c.conversation_id AND u.role = 'user'
                  ORDER BY u.created_at, u.id LIMIT 1)
           FROM chat_turns c GROUP BY c.conversation_id HAVING SUM(c.role = 'user') > 0",
    )?;
    let rows = stmt.query_map([], |r| {
        let (id, at, title_rec, first): (String, String, Option<String>, Option<String>) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        let title = title_rec
            .and_then(
                |c| match serde_json::from_str::<crate::chat::store::Record>(&c) {
                    Ok(crate::chat::store::Record::Title { title }) => Some(title),
                    _ => None,
                },
            )
            .or_else(|| {
                first.map(|q| {
                    let line = q
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .trim();
                    line.chars().take(60).collect::<String>()
                })
            })
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "New conversation".into());
        Ok(Entry::new(
            Kind::Chat,
            &id,
            &title,
            "Chat",
            &format!("chat/{id}"),
            "conversation",
            ts(&at),
        ))
    })?;
    for r in rows {
        out.push(r?);
    }
    // Files the sessions touched (latest session per path).
    let mut stmt = conn.prepare_cached(
        "SELECT f.path, COALESCE(f.rel_path, f.path), f.session_id, MAX(f.last_seen_at),
                COALESCE(s.repo_root, s.cwd)
           FROM file_changes f JOIN sessions s ON s.id = f.session_id
          WHERE f.change_kind <> 'deleted'
          GROUP BY f.path ORDER BY MAX(f.last_seen_at) DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([MAX_FILES], |r| {
        let (path, rel, sid, at, repo): (String, String, String, String, String) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let repo_name = std::path::Path::new(&repo)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let detail = match (repo_name.is_empty(), dir.is_empty()) {
            (false, false) => format!("{repo_name} · {dir}/"),
            (false, true) => repo_name,
            (true, _) => dir.to_string(),
        };
        Ok(Entry::new(
            Kind::File,
            &path,
            &name,
            &detail,
            &format!("sessions/{sid}/files"),
            &rel,
            ts(&at),
        ))
    })?;
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// The built index.
#[derive(Debug, Default)]
pub struct Built {
    pub entries: Vec<Entry>,
    /// Notes and messages by id (where a body hit lands).
    pub by_id: std::collections::HashMap<(Kind, String), usize>,
}

impl Built {
    pub fn new(entries: Vec<Entry>) -> Built {
        let by_id = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e.kind, Kind::Note | Kind::Message))
            .map(|(i, e)| ((e.kind, e.id.clone()), i))
            .collect();
        Built { entries, by_id }
    }
}

/// The palette service: the database and the cached index.
pub struct Palette {
    me: Weak<Palette>,
    db: Arc<Db>,
    index: RwLock<Option<Arc<Built>>>,
    built_at: Mutex<Option<Instant>>,
    stale: AtomicBool,
    /// Rebuild on a background thread after an invalidation (the app).
    background: AtomicBool,
    rebuilding: AtomicBool,
    clock: Option<Arc<dyn Fn() -> OffsetDateTime + Send + Sync>>,
    tz: String,
}

impl std::fmt::Debug for Palette {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Palette").finish_non_exhaustive()
    }
}

impl Palette {
    pub fn new(db: Arc<Db>) -> Arc<Palette> {
        Palette::with_options(db, None, None)
    }

    /// A fixed clock for the recency boost (tests).
    pub fn with_clock(
        db: Arc<Db>,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    ) -> Arc<Palette> {
        Palette::with_options(db, Some(clock), None)
    }

    /// A clock and the zone event times are shown in (default the device's).
    pub fn with_options(
        db: Arc<Db>,
        clock: Option<Arc<dyn Fn() -> OffsetDateTime + Send + Sync>>,
        tz: Option<String>,
    ) -> Arc<Palette> {
        Arc::new_cyclic(|me| Palette {
            me: me.clone(),
            db,
            index: RwLock::new(None),
            built_at: Mutex::new(None),
            stale: AtomicBool::new(true),
            background: AtomicBool::new(false),
            rebuilding: AtomicBool::new(false),
            clock,
            tz: tz.unwrap_or_else(crate::scheduler::schedule::device_zone),
        })
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// The zone event and message times are shown in.
    pub fn tz(&self) -> &str {
        &self.tz
    }

    pub fn now(&self) -> OffsetDateTime {
        match &self.clock {
            Some(c) => c(),
            None => OffsetDateTime::now_utc(),
        }
    }

    /// Rebuild on a background thread after an invalidation, answering
    /// queries from the previous index meanwhile (the app turns it on).
    pub fn set_background(&self, on: bool) {
        self.background.store(on, Ordering::SeqCst);
    }

    /// Mark the index stale (the app calls this on `sessions.changed`,
    /// `notes.changed`, `inbox.changed`, `agenda.changed`,
    /// `automations.changed` and after Chat turns); in the background mode
    /// a rebuild starts at once.
    pub fn invalidate(&self) {
        self.stale.store(true, Ordering::SeqCst);
        if self.background.load(Ordering::SeqCst) {
            self.spawn_rebuild();
        }
    }

    /// One rebuild at a time; it repeats while invalidations keep coming.
    fn spawn_rebuild(&self) {
        if self.rebuilding.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.me.clone();
        let spawned = std::thread::Builder::new()
            .name("mode-palette".into())
            .spawn(move || {
                let Some(p) = me.upgrade() else { return };
                loop {
                    if let Err(e) = p.refresh() {
                        tracing::warn!("palette rebuild: {e}");
                    }
                    if !p.stale.load(Ordering::SeqCst) {
                        p.rebuilding.store(false, Ordering::SeqCst);
                        // An invalidation between the check and the flag.
                        if !p.stale.load(Ordering::SeqCst)
                            || p.rebuilding.swap(true, Ordering::SeqCst)
                        {
                            break;
                        }
                    }
                }
            });
        if spawned.is_err() {
            self.rebuilding.store(false, Ordering::SeqCst);
        }
    }

    /// A rebuild is running on the background thread.
    pub fn is_rebuilding(&self) -> bool {
        self.rebuilding.load(Ordering::SeqCst)
    }

    /// Rebuild now.
    pub fn refresh(&self) -> Result<Arc<Built>> {
        self.stale.store(false, Ordering::SeqCst);
        let (now, tz) = (self.now(), self.tz.clone());
        let entries = self.db.read(move |c| load_at(c, now, &tz))?;
        let built = Arc::new(Built::new(entries));
        *self.index.write().unwrap_or_else(|p| p.into_inner()) = Some(Arc::clone(&built));
        *self.built_at.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
        Ok(built)
    }

    /// The current index: rebuilt first when stale, missing or old, unless
    /// a background rebuild is on its way (then the previous one answers).
    pub fn index(&self) -> Result<Arc<Built>> {
        let old = self
            .built_at
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none_or(|t| t.elapsed() > MAX_AGE);
        let cur = self.index.read().unwrap_or_else(|p| p.into_inner()).clone();
        let stale = self.stale.load(Ordering::SeqCst);
        match cur {
            Some(b) if !old && !stale => Ok(b),
            Some(b) if self.background.load(Ordering::SeqCst) => {
                if old && !stale {
                    self.stale.store(true, Ordering::SeqCst);
                }
                self.spawn_rebuild();
                Ok(b)
            }
            _ => self.refresh(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_titles_follow_the_kit_row() {
        assert_eq!(
            message_title(
                "Sam",
                "channel",
                Some("release"),
                None,
                "can we ship v2 behind the flag today?"
            ),
            "Sam · #release · can we ship v2 behind the flag today?"
        );
        assert_eq!(
            message_title(
                "Priya",
                "email",
                None,
                Some("Re: API v2 rollout"),
                "see below"
            ),
            "Priya · Inbox · Re: API v2 rollout"
        );
        assert_eq!(
            message_title("Priya", "email", None, Some(" "), "first\nsecond"),
            "Priya · Inbox · first",
            "no subject: the snippet's first line"
        );
        assert_eq!(
            message_title("Lena", "dm", Some("@lena"), None, "PR 412 is up"),
            "Lena · PR 412 is up",
            "no chat for a DM"
        );
        assert_eq!(
            message_title("Sam", "channel_thread", Some("#release"), None, "ok"),
            "Sam · #release · ok"
        );
        let long = "x".repeat(200);
        assert_eq!(
            message_title("Sam", "channel", Some("release"), None, &long)
                .chars()
                .filter(|c| *c == 'x')
                .count(),
            MESSAGE_PREVIEW_CHARS
        );
    }

    #[test]
    fn received_labels_are_relative_within_a_day_then_dates() {
        // 2026-10-21T12:00:00Z.
        let now = 1_792_584_000;
        let tz = "Europe/Berlin";
        assert_eq!(received_label(now - 59, now, tz), "now");
        assert_eq!(received_label(now + 30, now, tz), "now", "clock skew");
        assert_eq!(received_label(now - 60, now, tz), "1m");
        assert_eq!(received_label(now - 3_599, now, tz), "59m");
        assert_eq!(received_label(now - 3_600, now, tz), "1h");
        assert_eq!(received_label(now - 86_399, now, tz), "23h");
        assert_eq!(received_label(now - 86_400, now, tz), "20 Oct");
        // 2026-10-17T23:30Z is already the 18th in Berlin.
        assert_eq!(
            received_label(now - 3 * 86_400 - 12 * 3_600 - 1_800, now, tz),
            "18 Oct"
        );
        assert_eq!(received_label(now - 365 * 86_400, now, tz), "21 Oct 2025");
        assert_eq!(received_label(0, now, tz), "");
    }
}
