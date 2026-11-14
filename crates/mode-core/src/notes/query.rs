//! The query bar (04 §8): typed text → chips → `filter_json`. The pipeline
//! is deterministic and local: tokenize (words, quoted strings, prefixes),
//! prefixed tokens (`#tag`, `@person`, `in:`, `repo:`, `status:` / `is:`,
//! `when:`, `type:`, `tag:`, `folder:`), the lexicon of §8.2 matched
//! longest phrase first over the whole input, entity matching against tags,
//! folders, contacts, repos and statuses with the ambiguity rules of §8.3,
//! the remainder as one text chip, merging, and compilation through the
//! same compiler as the groupings (§8.4). Card titles follow §8.5 and the
//! autocomplete list §8.7.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use time::{Date, Duration, Month};

use super::ctx::Cx;
use super::fields::{self, WeekStart};
use super::groupings::{self, NoteListRow, RunOpts};
use crate::db::Result;
use crate::queries::filter::{
    Filter, FolderFilter, InList, PersonFilter, RepoFilter, SortKey, StatusFilter, TagsFilter,
    WhenFilter,
};
use crate::settings::StatusDef;

// ------------------------------------------------------------------ chips

/// A tag chip's value: one tag (→ `tags.all`) or an `or` group (→ `tags.any`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TagValue {
    One(String),
    Any { any: Vec<String> },
}

/// A folder chip's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderRef {
    pub id: String,
}

/// The kind and value of a chip (04 §8.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ChipValue {
    Person(PersonFilter),
    /// `notes` | `tasks` | `messages` | `sessions` | `calendar_events`.
    Type(String),
    /// A status slug, or `urgent` / `not-urgent`.
    Status(String),
    When(WhenFilter),
    Tag(TagValue),
    Repo(RepoFilter),
    Folder(FolderRef),
    Text(String),
    /// `pinned` | `archived` | `source:<kind>`.
    Flag(String),
}

impl ChipValue {
    pub fn kind(&self) -> &'static str {
        match self {
            ChipValue::Person(_) => "person",
            ChipValue::Type(_) => "type",
            ChipValue::Status(_) => "status",
            ChipValue::When(_) => "when",
            ChipValue::Tag(_) => "tag",
            ChipValue::Repo(_) => "repo",
            ChipValue::Folder(_) => "folder",
            ChipValue::Text(_) => "text",
            ChipValue::Flag(_) => "flag",
        }
    }
}

fn typed() -> String {
    "typed".into()
}

/// A chip (04 §8.1): `{"kind", "value", "label", "glyph", "origin"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    #[serde(flatten)]
    pub value: ChipValue,
    pub label: String,
    pub glyph: Option<String>,
    /// `typed` | `autocomplete` | `refine` | `saved`.
    #[serde(default = "typed")]
    pub origin: String,
    /// A person chip's avatar initials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initials: Option<String>,
    /// A when chip's relative form (`this week` → `week_start`…`week_end`),
    /// stored by "Save as grouping" so the grouping stays correct (§8.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<WhenFilter>,
    /// The other kinds the same words matched (§8.3 `▾`), best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Chip>,
}

impl Chip {
    fn new(value: ChipValue, label: impl Into<String>, glyph: Option<&str>) -> Chip {
        Chip {
            value,
            label: label.into(),
            glyph: glyph.map(str::to_string),
            origin: typed(),
            initials: None,
            relative: None,
            alternatives: Vec::new(),
        }
    }

    pub fn kind(&self) -> &'static str {
        self.value.kind()
    }

    fn key(&self) -> String {
        format!(
            "{}:{}",
            self.kind(),
            serde_json::to_string(&self.value).unwrap_or_default()
        )
    }
}

const WHEN_GLYPH: &str = "\u{25A6}";

fn month_name(m: Month) -> &'static str {
    match m {
        Month::January => "Jan",
        Month::February => "Feb",
        Month::March => "Mar",
        Month::April => "Apr",
        Month::May => "May",
        Month::June => "Jun",
        Month::July => "Jul",
        Month::August => "Aug",
        Month::September => "Sep",
        Month::October => "Oct",
        Month::November => "Nov",
        Month::December => "Dec",
    }
}

fn short(d: Date) -> String {
    format!("{} {}", month_name(d.month()), d.day())
}

/// "Oct 19", "Oct 20 – 26", "Oct 26 – Nov 1", "from Oct 19", "until Oct 19".
pub fn range_label(from: Option<Date>, to: Option<Date>) -> String {
    match (from, to) {
        (Some(a), Some(b)) if a == b => short(a),
        (Some(a), Some(b)) if a.month() == b.month() && a.year() == b.year() => {
            format!("{} \u{2013} {}", short(a), b.day())
        }
        (Some(a), Some(b)) => format!("{} \u{2013} {}", short(a), short(b)),
        (Some(a), None) => format!("from {}", short(a)),
        (None, Some(b)) => format!("until {}", short(b)),
        (None, None) => "Any date".into(),
    }
}

fn bucket_label(b: &str) -> &'static str {
    match b {
        "overdue" => "Overdue",
        "today" => "Today",
        "tomorrow" => "Tomorrow",
        "scheduled" => "Scheduled",
        _ => "Unscheduled",
    }
}

fn when_chip(buckets: &[&str]) -> Chip {
    let label = if buckets == ["overdue", "today", "tomorrow", "scheduled"] {
        "Due".to_string()
    } else {
        buckets
            .iter()
            .map(|b| bucket_label(b))
            .collect::<Vec<_>>()
            .join(" / ")
    };
    Chip::new(
        ChipValue::When(WhenFilter::Buckets {
            r#in: buckets.iter().map(|s| s.to_string()).collect(),
        }),
        label,
        Some(WHEN_GLYPH),
    )
}

fn range_chip(
    from: Option<Date>,
    to: Option<Date>,
    relative: Option<(Option<&str>, Option<&str>)>,
) -> Chip {
    let mut c = Chip::new(
        ChipValue::When(WhenFilter::Range {
            from: from.map(fields::fmt_date),
            to: to.map(fields::fmt_date),
        }),
        range_label(from, to),
        Some(WHEN_GLYPH),
    );
    c.relative = relative.map(|(f, t)| WhenFilter::Range {
        from: f.map(str::to_string),
        to: t.map(str::to_string),
    });
    c
}

/// A type chip (`tasks`, `notes`, `messages`, `sessions`, `calendar_events`).
pub fn type_chip(t: &str) -> Chip {
    let (label, glyph) = match t {
        "tasks" => ("Tasks", "\u{2610}"),
        "messages" => ("Messages", "\u{2709}"),
        "sessions" => ("Sessions", "\u{25B6}"),
        "calendar_events" => ("Events", WHEN_GLYPH),
        _ => ("Notes", "\u{2261}"),
    };
    Chip::new(ChipValue::Type(t.into()), label, Some(glyph))
}

fn status_chip(slug: &str, statuses: &[StatusDef]) -> Chip {
    match slug {
        "urgent" => Chip::new(
            ChipValue::Status("urgent".into()),
            "Urgent",
            Some("\u{2731}"),
        ),
        "not-urgent" => Chip::new(ChipValue::Status("not-urgent".into()), "Not urgent", None),
        s => {
            let def = statuses.iter().find(|d| d.slug == s);
            Chip::new(
                ChipValue::Status(s.into()),
                def.map(|d| d.label.clone())
                    .unwrap_or_else(|| s.to_string()),
                Some(
                    def.and_then(|d| d.glyph.as_deref())
                        .unwrap_or(fields::UNKNOWN_GLYPH),
                ),
            )
        }
    }
}

fn tag_chip(norm: &str) -> Chip {
    Chip::new(
        ChipValue::Tag(TagValue::One(norm.into())),
        format!("#{norm}"),
        None,
    )
}

fn flag_chip(flag: &str) -> Chip {
    let (label, glyph) = match flag {
        "pinned" => ("Pinned", Some("\u{2316}")),
        "archived" => ("Archived", Some("\u{25AB}")),
        "source:capture" => ("Captured", None),
        "source:message" => ("From messages", None),
        "source:session" => ("From sessions", None),
        "source:import" => ("Imported", None),
        other => (other, None),
    };
    Chip::new(ChipValue::Flag(flag.into()), label, glyph)
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

fn person_chip(c: &Contact) -> Chip {
    let mut chip = Chip::new(
        ChipValue::Person(PersonFilter {
            contact_id: c.id.clone(),
        }),
        c.name.clone(),
        Some("\u{25C9}"),
    );
    chip.initials = Some(initials(&c.name));
    chip
}

fn repo_chip(root: &str) -> Chip {
    Chip::new(
        ChipValue::Repo(RepoFilter {
            repo_root: root.to_string(),
        }),
        basename(root),
        Some("\u{2387}"),
    )
}

fn folder_chip(f: &FolderEnt) -> Chip {
    Chip::new(
        ChipValue::Folder(FolderRef { id: f.id.clone() }),
        f.name.clone(),
        Some("\u{25B8}"),
    )
}

fn text_chip(t: &str) -> Chip {
    Chip::new(ChipValue::Text(t.to_string()), t.to_string(), None)
}

fn basename(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_string()
}

// --------------------------------------------------------------- entities

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderEnt {
    pub id: String,
    pub path: String,
    pub name: String,
}

/// What the resolver matches against (one snapshot per resolution).
#[derive(Debug, Clone, Default)]
pub struct Entities {
    /// `(display name, name_norm)`, most used first.
    pub tags: Vec<(String, String)>,
    pub folders: Vec<FolderEnt>,
    pub contacts: Vec<Contact>,
    /// `sessions.repo_root` values, most recent first.
    pub repos: Vec<String>,
    pub statuses: Vec<StatusDef>,
    pub today: Option<Date>,
    pub week_start: WeekStart,
    /// `19/10` is the 19th of October (day first) or not (month first).
    pub day_first: bool,
}

/// Day-first numeric dates unless the OS locale is US-style (04 §8.3).
pub fn locale_day_first() -> bool {
    let lang = ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    !(lang.is_empty()
        || lang.starts_with("en_US")
        || lang.starts_with("C")
        || lang.starts_with("POSIX"))
}

impl Entities {
    pub fn load(cx: &Cx) -> Result<Entities> {
        let conn = cx.conn;
        let mut stmt = conn.prepare(
            "SELECT t.name, t.name_norm FROM tags t
               LEFT JOIN note_tags nt ON nt.tag_id = t.id
              WHERE t.deleted_at IS NULL
              GROUP BY t.id ORDER BY COUNT(nt.note_id) DESC, t.name_norm",
        )?;
        let tags = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path, f.name FROM folders f WHERE f.deleted_at IS NULL
              ORDER BY (SELECT COUNT(*) FROM notes n WHERE n.folder_id = f.id AND n.deleted_at IS NULL) DESC, f.path",
        )?;
        let folders = stmt
            .query_map([], |r| {
                Ok(FolderEnt {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare(
            "SELECT id, display_name FROM contacts WHERE deleted_at IS NULL ORDER BY message_count DESC, display_name",
        )?;
        let contacts = stmt
            .query_map([], |r| {
                Ok(Contact {
                    id: r.get(0)?,
                    name: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare(
            "SELECT repo_root FROM sessions WHERE repo_root IS NOT NULL AND repo_root <> ''
              GROUP BY repo_root ORDER BY MAX(last_event_at) DESC",
        )?;
        let repos = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(Entities {
            tags,
            folders,
            contacts,
            repos,
            statuses: cx.statuses().to_vec(),
            today: Some(cx.today()),
            week_start: cx.week_start,
            day_first: locale_day_first(),
        })
    }

    fn tag(&self, words: &str) -> Option<String> {
        let w = fields::tag_norm(words)?;
        self.tags
            .iter()
            .find(|(_, n)| *n == w)
            .map(|(_, n)| n.clone())
    }

    fn folders_named(&self, words: &str) -> Vec<&FolderEnt> {
        let w = words.trim_matches('/').to_lowercase();
        self.folders
            .iter()
            .filter(|f| f.name.to_lowercase() == w || f.path.to_lowercase() == w)
            .collect()
    }

    fn repos_named(&self, words: &str) -> Vec<&String> {
        let w = words.to_lowercase();
        self.repos
            .iter()
            .filter(|r| basename(r).to_lowercase() == w)
            .collect()
    }

    /// Contacts whose display name or first name equals (`exact`) or starts
    /// with `words`, case-insensitively.
    fn contacts_matching(&self, words: &str, exact: bool) -> Vec<&Contact> {
        let w = words.to_lowercase();
        if w.is_empty() {
            return Vec::new();
        }
        let full: Vec<&Contact> = self
            .contacts
            .iter()
            .filter(|c| {
                let name = c.name.to_lowercase();
                let first = name.split_whitespace().next().unwrap_or("").to_string();
                name == w || first == w
            })
            .collect();
        if exact || !full.is_empty() {
            return full;
        }
        self.contacts
            .iter()
            .filter(|c| {
                let name = c.name.to_lowercase();
                name.starts_with(&w) || name.split_whitespace().any(|p| p.starts_with(&w))
            })
            .collect()
    }

    fn status(&self, words: &str) -> Option<String> {
        let w = words.to_lowercase();
        self.statuses
            .iter()
            .find(|s| s.slug == w || s.label.to_lowercase() == w)
            .map(|s| s.slug.clone())
    }
}

// ------------------------------------------------------------- tokenizer

#[derive(Debug, Clone)]
struct Tok {
    raw: String,
    norm: String,
    used: bool,
}

/// Quoted strings out (as text chips with their position), words in.
fn tokenise(text: &str) -> (Vec<Tok>, Vec<(usize, String)>) {
    let mut toks = Vec::new();
    let mut quoted = Vec::new();
    let mut rest = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\u{201C}' {
            let mut q = String::new();
            let mut closed = false;
            for d in chars.by_ref() {
                if d == '"' || d == '\u{201D}' {
                    closed = true;
                    break;
                }
                q.push(d);
            }
            if closed && !q.trim().is_empty() {
                // Words before the quote, then a placeholder position.
                for w in rest.split_whitespace() {
                    toks.push(word(w));
                }
                rest.clear();
                quoted.push((toks.len(), q.trim().to_string()));
            } else {
                rest.push_str(&q);
            }
            continue;
        }
        rest.push(c);
    }
    for w in rest.split_whitespace() {
        toks.push(word(w));
    }
    (toks, quoted)
}

fn word(w: &str) -> Tok {
    let norm = w
        .replace(['\u{2019}', '\u{2018}'], "'")
        .trim_end_matches(['?', '.', ',', ';'])
        .to_lowercase();
    Tok {
        raw: w.to_string(),
        norm,
        used: false,
    }
}

const STOP_WORDS: &[&str] = &[
    "show", "list", "find", "what", "which", "are", "is", "my", "the", "all", "any", "please",
    "for", "with", "of", "and", "me", "about",
];

// ------------------------------------------------------------- the lexicon

/// A match at a position: how many tokens, the chip, and a rank for ties.
struct Cand {
    start: usize,
    len: usize,
    chip: Chip,
}

fn num_word(s: &str) -> Option<i64> {
    if let Ok(n) = s.parse::<i64>() {
        return (0..=3650).contains(&n).then_some(n);
    }
    let words = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve", "thirteen", "fourteen",
    ];
    words.iter().position(|w| *w == s).map(|i| i as i64)
}

fn month_from(s: &str) -> Option<Month> {
    let s = s.trim_end_matches('.');
    let m = match s.get(..3)? {
        "jan" => Month::January,
        "feb" => Month::February,
        "mar" => Month::March,
        "apr" => Month::April,
        "may" => Month::May,
        "jun" => Month::June,
        "jul" => Month::July,
        "aug" => Month::August,
        "sep" => Month::September,
        "oct" => Month::October,
        "nov" => Month::November,
        "dec" => Month::December,
        _ => return None,
    };
    let full = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let ok = s.len() == 3 || s == "sept" || full.contains(&s);
    ok.then_some(m)
}

fn day_num(s: &str) -> Option<u8> {
    let s = s
        .trim_end_matches("st")
        .trim_end_matches("nd")
        .trim_end_matches("rd")
        .trim_end_matches("th");
    s.parse::<u8>().ok().filter(|d| (1..=31).contains(d))
}

/// A date phrase at `i`: ISO, `19/10` (locale), `oct 19`, `19 oct`, a
/// weekday, `today` / `tomorrow` / `yesterday`. Returns the date and the
/// number of words used.
fn date_at(toks: &[Tok], i: usize, e: &Entities) -> Option<(Date, usize)> {
    let today = e.today?;
    let w = toks.get(i)?.norm.as_str();
    if let Some(d) = fields::parse_date(w) {
        return Some((d, 1));
    }
    match w {
        "today" => return Some((today, 1)),
        "tomorrow" | "tmrw" | "tmr" => return Some((today + Duration::days(1), 1)),
        "yesterday" => return Some((today - Duration::days(1), 1)),
        _ => {}
    }
    if let Some(wd) = fields::weekday_from_name(w) {
        return Some((fields::next_weekday(today, wd), 1));
    }
    let year_of = |m: Month, d: u8| -> Option<Date> {
        let this = Date::from_calendar_date(today.year(), m, d).ok()?;
        if (today - this).whole_days() > 180 {
            Date::from_calendar_date(today.year() + 1, m, d).ok()
        } else {
            Some(this)
        }
    };
    // 19/10, 10/19, 19/10/2026
    if w.contains('/') {
        let parts: Vec<&str> = w.split('/').collect();
        if (2..=3).contains(&parts.len()) {
            let a: u8 = parts[0].parse().ok()?;
            let b: u8 = parts[1].parse().ok()?;
            let (d, m) = if e.day_first { (a, b) } else { (b, a) };
            let m = Month::try_from(m).ok()?;
            let date = match parts.get(2) {
                Some(y) => {
                    let y: i32 = y.parse().ok()?;
                    let y = if y < 100 { 2000 + y } else { y };
                    Date::from_calendar_date(y, m, d).ok()?
                }
                None => year_of(m, d)?,
            };
            return Some((date, 1));
        }
    }
    // oct 19 [2026] | 19 oct [2026]
    let next = toks.get(i + 1).map(|t| t.norm.as_str());
    let (m, d) = if let (Some(m), Some(d)) = (month_from(w), next.and_then(day_num)) {
        (m, d)
    } else if let (Some(d), Some(m)) = (day_num(w), next.and_then(month_from)) {
        (m, d)
    } else {
        return None;
    };
    if let Some(y) = toks
        .get(i + 2)
        .and_then(|t| t.norm.parse::<i32>().ok())
        .filter(|y| (1900..=2999).contains(y))
    {
        return Some((Date::from_calendar_date(y, m, d).ok()?, 3));
    }
    Some((year_of(m, d)?, 2))
}

/// Fixed phrases → chips (§8.2), longest listed first per family.
fn fixed_phrases() -> Vec<(&'static str, Chip)> {
    let b = |l: &[&str]| when_chip(l);
    vec![
        ("due today", b(&["today"])),
        ("today", b(&["today"])),
        ("tonight", b(&["today"])),
        ("tomorrow", b(&["tomorrow"])),
        ("tmrw", b(&["tomorrow"])),
        ("tmr", b(&["tomorrow"])),
        ("past due", b(&["overdue"])),
        ("overdue", b(&["overdue"])),
        ("late", b(&["overdue"])),
        ("with a date", b(&["scheduled"])),
        ("scheduled", b(&["scheduled"])),
        ("dated", b(&["scheduled"])),
        ("no date", b(&["none"])),
        ("unscheduled", b(&["none"])),
        ("undated", b(&["none"])),
        ("someday", b(&["none"])),
        ("due", b(&["overdue", "today", "tomorrow", "scheduled"])),
        (
            "upcoming",
            b(&["overdue", "today", "tomorrow", "scheduled"]),
        ),
        ("not started", status_chip("todo", &[])),
        ("to-do", status_chip("todo", &[])),
        ("to do", status_chip("todo", &[])),
        ("todo", status_chip("todo", &[])),
        ("open", status_chip("todo", &[])),
        ("in progress", status_chip("doing", &[])),
        ("doing", status_chip("doing", &[])),
        ("wip", status_chip("doing", &[])),
        ("started", status_chip("doing", &[])),
        ("done", status_chip("done", &[])),
        ("completed", status_chip("done", &[])),
        ("finished", status_chip("done", &[])),
        ("closed", status_chip("done", &[])),
        ("not urgent", status_chip("not-urgent", &[])),
        ("unflagged", status_chip("not-urgent", &[])),
        ("urgent", status_chip("urgent", &[])),
        ("flagged", status_chip("urgent", &[])),
        ("important", status_chip("urgent", &[])),
        ("!", status_chip("urgent", &[])),
        ("tasks", type_chip("tasks")),
        ("task", type_chip("tasks")),
        ("todos", type_chip("tasks")),
        ("to-dos", type_chip("tasks")),
        ("items", type_chip("tasks")),
        ("things", type_chip("tasks")),
        ("notes", type_chip("notes")),
        ("note", type_chip("notes")),
        ("everything", type_chip("notes")),
        ("slack messages", type_chip("messages")),
        ("messages", type_chip("messages")),
        ("emails", type_chip("messages")),
        ("mail", type_chip("messages")),
        ("dms", type_chip("messages")),
        ("claude sessions", type_chip("sessions")),
        ("sessions", type_chip("sessions")),
        ("events", type_chip("calendar_events")),
        ("meetings", type_chip("calendar_events")),
        ("calendar", type_chip("calendar_events")),
        ("pinned", flag_chip("pinned")),
        ("in archive", flag_chip("archived")),
        ("archived", flag_chip("archived")),
        ("captured", flag_chip("source:capture")),
        ("from messages", flag_chip("source:message")),
        ("from sessions", flag_chip("source:session")),
        ("imported", flag_chip("source:import")),
    ]
}

fn words_at(toks: &[Tok], i: usize, n: usize) -> Option<String> {
    let slice = toks.get(i..i + n)?;
    if slice.iter().any(|t| t.used) {
        return None;
    }
    Some(
        slice
            .iter()
            .map(|t| t.norm.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Lexicon candidates over the unused tokens (§8.2).
fn lexicon(toks: &[Tok], e: &Entities) -> Vec<Cand> {
    let mut out = Vec::new();
    let phrases = fixed_phrases();
    let Some(today) = e.today else {
        return out;
    };
    let ws = e.week_start;
    let tok = |t: &str| fields::resolve_date_token(t, today, ws);
    for i in 0..toks.len() {
        if toks[i].used {
            continue;
        }
        for (p, chip) in &phrases {
            let n = p.split(' ').count();
            if words_at(toks, i, n).as_deref() == Some(*p) {
                let mut chip = chip.clone();
                if let ChipValue::Status(s) = &chip.value {
                    chip = status_chip(&s.clone(), &e.statuses);
                }
                out.push(Cand {
                    start: i,
                    len: n,
                    chip,
                });
            }
        }
        // Custom statuses by label or slug (up to three words).
        for n in 1..=3 {
            if let Some(w) = words_at(toks, i, n)
                && let Some(slug) = e.status(&w)
            {
                out.push(Cand {
                    start: i,
                    len: n,
                    chip: status_chip(&slug, &e.statuses),
                });
            }
        }
        let w0 = toks[i].norm.as_str();
        let w1 = toks.get(i + 1).filter(|t| !t.used).map(|t| t.norm.as_str());
        // this / next week, this / next month
        match (w0, w1) {
            ("this", Some("week")) => out.push(Cand {
                start: i,
                len: 2,
                chip: range_chip(
                    tok("week_start"),
                    tok("week_end"),
                    Some((Some("week_start"), Some("week_end"))),
                ),
            }),
            ("next", Some("week")) => out.push(Cand {
                start: i,
                len: 2,
                chip: range_chip(
                    tok("week_start+7"),
                    tok("week_end+7"),
                    Some((Some("week_start+7"), Some("week_end+7"))),
                ),
            }),
            ("this", Some("month")) => out.push(Cand {
                start: i,
                len: 2,
                chip: range_chip(
                    tok("month_start"),
                    tok("month_end"),
                    Some((Some("month_start"), Some("month_end"))),
                ),
            }),
            ("next", Some("month")) => out.push(Cand {
                start: i,
                len: 2,
                chip: range_chip(
                    tok("next_month_start"),
                    tok("next_month_end"),
                    Some((Some("next_month_start"), Some("next_month_end"))),
                ),
            }),
            _ => {}
        }
        // next N days / in N days
        if let (Some(n), Some("days" | "day")) = (
            w1.and_then(num_word),
            toks.get(i + 2).filter(|t| !t.used).map(|t| t.norm.as_str()),
        ) {
            if w0 == "next" {
                let to = format!("today+{n}");
                out.push(Cand {
                    start: i,
                    len: 3,
                    chip: range_chip(
                        Some(today),
                        tok(&to),
                        Some((Some("today"), Some(to.as_str()))),
                    ),
                });
            } else if w0 == "in" {
                let d = format!("today+{n}");
                out.push(Cand {
                    start: i,
                    len: 3,
                    chip: range_chip(tok(&d), tok(&d), Some((Some(d.as_str()), Some(d.as_str())))),
                });
            }
        }
        // yesterday (a one-day range, relative)
        if w0 == "yesterday" {
            out.push(Cand {
                start: i,
                len: 1,
                chip: range_chip(
                    tok("today-1"),
                    tok("today-1"),
                    Some((Some("today-1"), Some("today-1"))),
                ),
            });
        }
        // next <weekday>: strictly after today
        if w0 == "next"
            && let Some(wd) = w1.and_then(fields::weekday_from_name)
        {
            let d = fields::next_weekday(today + Duration::days(1), wd);
            out.push(Cand {
                start: i,
                len: 2,
                chip: range_chip(Some(d), Some(d), None),
            });
        }
        // on / by / before / until / after / from <date>
        let prep = matches!(w0, "on" | "by" | "before" | "until" | "after" | "from");
        if prep
            && let Some((d, n)) =
                date_at(toks, i + 1, e).filter(|(_, n)| (i + 1..i + 1 + n).all(|j| !toks[j].used))
        {
            let chip = match w0 {
                "on" => range_chip(Some(d), Some(d), None),
                "after" | "from" => range_chip(Some(d), None, None),
                _ => range_chip(None, Some(d), None),
            };
            out.push(Cand {
                start: i,
                len: n + 1,
                chip,
            });
        }
        // a bare date or weekday (not today / tomorrow / yesterday: phrases above)
        if !matches!(w0, "today" | "tomorrow" | "tmrw" | "tmr" | "yesterday")
            && let Some((d, n)) =
                date_at(toks, i, e).filter(|(_, n)| (i..i + n).all(|j| !toks[j].used))
        {
            out.push(Cand {
                start: i,
                len: n,
                chip: range_chip(Some(d), Some(d), None),
            });
        }
    }
    out
}

/// Entity candidates (§8.2 rows with names, §8.3 priorities).
fn entities(toks: &[Tok], e: &Entities, ambiguous: &mut Vec<Vec<Chip>>) -> Vec<Cand> {
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if toks[i].used {
            continue;
        }
        let w0 = toks[i].norm.clone();
        // Name's (possessive), one or two words.
        for n in 1..=2 {
            if let Some(words) = words_at(toks, i, n)
                && let Some(name) = words
                    .strip_suffix("'s")
                    .or_else(|| words.strip_suffix('\''))
            {
                let m = e.contacts_matching(name, false);
                if m.len() == 1 {
                    out.push(Cand {
                        start: i,
                        len: n,
                        chip: person_chip(m[0]),
                    });
                } else if m.len() > 1 {
                    ambiguous.push(m.iter().map(|c| person_chip(c)).collect());
                }
            }
        }
        // for / from / with Name
        if matches!(w0.as_str(), "for" | "from" | "with") {
            for n in (1..=2).rev() {
                if let Some(name) = words_at(toks, i + 1, n) {
                    let m = e.contacts_matching(&name, false);
                    if m.len() == 1 {
                        out.push(Cand {
                            start: i,
                            len: n + 1,
                            chip: person_chip(m[0]),
                        });
                        break;
                    } else if m.len() > 1 {
                        ambiguous.push(m.iter().map(|c| person_chip(c)).collect());
                        break;
                    }
                }
            }
        }
        // tag name / tagged name
        if matches!(w0.as_str(), "tag" | "tagged") {
            for n in (1..=3).rev() {
                if let Some(name) = words_at(toks, i + 1, n)
                    && let Some(t) = e.tag(&name)
                {
                    out.push(Cand {
                        start: i,
                        len: n + 1,
                        chip: tag_chip(&t),
                    });
                    break;
                }
            }
        }
        // in repo name / repo name / in name / folder name
        let rest_at = |skip: usize| -> Vec<(usize, String)> {
            (1..=3)
                .rev()
                .filter_map(|n| words_at(toks, i + skip, n).map(|w| (n, w)))
                .collect()
        };
        if w0 == "in" && toks.get(i + 1).is_some_and(|t| t.norm == "repo") {
            for (n, name) in rest_at(2) {
                if let Some(r) = e.repos_named(&name).first() {
                    out.push(Cand {
                        start: i,
                        len: n + 2,
                        chip: repo_chip(r),
                    });
                    break;
                }
            }
        }
        if w0 == "repo" {
            for (n, name) in rest_at(1) {
                if let Some(r) = e.repos_named(&name).first() {
                    out.push(Cand {
                        start: i,
                        len: n + 1,
                        chip: repo_chip(r),
                    });
                    break;
                }
            }
        }
        if w0 == "folder" {
            for (n, name) in rest_at(1) {
                if let Some(f) = e.folders_named(&name).first() {
                    out.push(Cand {
                        start: i,
                        len: n + 1,
                        chip: folder_chip(f),
                    });
                    break;
                }
            }
        }
        if w0 == "in" {
            for (n, name) in rest_at(1) {
                // A repo first; a folder when no repo has that name.
                if let Some(r) = e.repos_named(&name).first() {
                    out.push(Cand {
                        start: i,
                        len: n + 1,
                        chip: repo_chip(r),
                    });
                    break;
                }
                if let Some(f) = e.folders_named(&name).first() {
                    out.push(Cand {
                        start: i,
                        len: n + 1,
                        chip: folder_chip(f),
                    });
                    break;
                }
            }
        }
        // A bare word (or words): tag > folder > contact > repo.
        for n in (1..=3).rev() {
            let Some(words) = words_at(toks, i, n) else {
                continue;
            };
            if STOP_WORDS.contains(&words.as_str()) {
                continue;
            }
            let mut found: Vec<Chip> = Vec::new();
            if let Some(t) = e.tag(&words) {
                found.push(tag_chip(&t));
            }
            if let Some(f) = e.folders_named(&words).first() {
                found.push(folder_chip(f));
            }
            let people = e.contacts_matching(&words, true);
            if people.len() == 1 {
                found.push(person_chip(people[0]));
            } else if people.len() > 1 && found.is_empty() {
                ambiguous.push(people.iter().map(|c| person_chip(c)).collect());
            }
            if let Some(r) = e.repos_named(&words).first() {
                found.push(repo_chip(r));
            }
            if !found.is_empty() {
                let mut chip = found.remove(0);
                chip.alternatives = found;
                out.push(Cand {
                    start: i,
                    len: n,
                    chip,
                });
                break;
            }
        }
    }
    out
}

/// Pick non-overlapping candidates, longest first, then leftmost.
fn select(toks: &mut [Tok], mut cands: Vec<Cand>) -> Vec<(usize, Chip)> {
    cands.sort_by(|a, b| b.len.cmp(&a.len).then(a.start.cmp(&b.start)));
    let mut out = Vec::new();
    for c in cands {
        if (c.start..c.start + c.len).any(|j| toks[j].used) {
            continue;
        }
        for t in &mut toks[c.start..c.start + c.len] {
            t.used = true;
        }
        out.push((c.start, c.chip));
    }
    out
}

/// Prefixed tokens (§8.2 / 03 §5.9.3 step 1): explicit prefixes win.
fn prefixed(toks: &mut [Tok], e: &Entities, ambiguous: &mut Vec<Vec<Chip>>) -> Vec<(usize, Chip)> {
    let mut out = Vec::new();
    for (i, tok) in toks.iter_mut().enumerate() {
        let raw = tok.raw.clone();
        let norm = tok.norm.clone();
        let chip = if let Some(t) = raw.strip_prefix('#') {
            fields::tag_norm(t).map(|n| tag_chip(&n))
        } else if let Some(name) = norm.strip_prefix('@') {
            let m = e.contacts_matching(name, false);
            match m.len() {
                1 => Some(person_chip(m[0])),
                0 => None,
                _ => {
                    ambiguous.push(m.iter().map(|c| person_chip(c)).collect());
                    None
                }
            }
        } else if let Some((p, v)) = norm.split_once(':').filter(|(_, v)| !v.is_empty()) {
            let v_raw = &raw[p.len() + 1..];
            match p {
                "in" | "folder" => {
                    e.folders_named(v)
                        .first()
                        .map(|f| folder_chip(f))
                        .or_else(|| {
                            (p == "in")
                                .then(|| e.repos_named(v).first().map(|r| repo_chip(r)))
                                .flatten()
                        })
                }
                "repo" => e.repos_named(v).first().map(|r| repo_chip(r)),
                "tag" => fields::tag_norm(v_raw).map(|n| tag_chip(&n)),
                "type" => match v {
                    "tasks" | "task" => Some(type_chip("tasks")),
                    "notes" | "note" => Some(type_chip("notes")),
                    "messages" => Some(type_chip("messages")),
                    "sessions" => Some(type_chip("sessions")),
                    "events" | "calendar" | "calendar_events" => Some(type_chip("calendar_events")),
                    _ => None,
                },
                "status" | "is" => match v {
                    "urgent" => Some(status_chip("urgent", &e.statuses)),
                    "overdue" | "today" | "tomorrow" | "scheduled" => Some(when_chip(&[v])),
                    "pinned" | "archived" => Some(flag_chip(v)),
                    other => e.status(other).map(|s| status_chip(&s, &e.statuses)),
                },
                "when" => {
                    let words: Vec<Tok> = v.replace(['-', '_'], " ").split(' ').map(word).collect();
                    let mut sub = words.clone();
                    let picked = select(&mut sub, lexicon(&words, e));
                    picked
                        .into_iter()
                        .find(|(_, c)| c.kind() == "when" && sub.iter().all(|t| t.used))
                        .map(|(_, c)| c)
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(c) = chip {
            tok.used = true;
            out.push((i, c));
        }
    }
    out
}

/// The result of resolving typed text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    /// New chips in text order (not yet merged with the bar's chips).
    pub chips: Vec<Chip>,
    /// Words that stay typed (an ambiguous name, §8.3).
    pub rest: String,
    /// Candidate lists for ambiguous names.
    pub ambiguous: Vec<Vec<Chip>>,
}

/// Run the §8 pipeline on `text`.
pub fn resolve_text(text: &str, e: &Entities) -> Resolved {
    let (mut toks, quoted) = tokenise(text);
    let mut ambiguous = Vec::new();
    let mut picked = prefixed(&mut toks, e, &mut ambiguous);
    let lex = lexicon(&toks, e);
    picked.extend(select(&mut toks, lex));
    let ents = entities(&toks, e, &mut ambiguous);
    picked.extend(select(&mut toks, ents));
    picked.sort_by_key(|(i, _)| *i);

    // `or` between two tag chips → tags.any (§8.2).
    let mut merged: Vec<(usize, Chip)> = Vec::new();
    for (pos, chip) in picked {
        if let (Some((ppos, prev)), ChipValue::Tag(TagValue::One(t))) =
            (merged.last_mut(), &chip.value)
            && let ChipValue::Tag(pv) = &prev.value
        {
            let between: Vec<usize> = (*ppos + 1..pos).collect();
            let prev_end = between.iter().take_while(|j| toks[**j].used).count();
            let gap: Vec<&Tok> = between.iter().skip(prev_end).map(|j| &toks[*j]).collect();
            if gap.len() == 1 && gap[0].norm == "or" && !gap[0].used {
                let mut any = match pv {
                    TagValue::One(a) => vec![a.clone()],
                    TagValue::Any { any } => any.clone(),
                };
                if !any.contains(t) {
                    any.push(t.clone());
                }
                let label = any
                    .iter()
                    .map(|a| format!("#{a}"))
                    .collect::<Vec<_>>()
                    .join(" or ");
                prev.value = ChipValue::Tag(TagValue::Any { any });
                prev.label = label;
                toks[*ppos + 1 + prev_end].used = true;
                *ppos = pos;
                continue;
            }
        }
        merged.push((pos, chip));
    }

    // The remainder: one text chip of the words left, stop words dropped.
    let mut out: Vec<(usize, Chip)> = merged;
    let mut rest_words: Vec<String> = Vec::new();
    let mut text_words: Vec<(usize, String)> = Vec::new();
    let ambiguous_words: Vec<String> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if t.used || STOP_WORDS.contains(&t.norm.as_str()) || t.norm.is_empty() {
            continue;
        }
        let is_ambiguous_name = ambiguous.iter().flatten().any(|c| {
            c.label
                .to_lowercase()
                .split_whitespace()
                .any(|p| p == t.norm.trim_start_matches('@').trim_end_matches("'s"))
        });
        if is_ambiguous_name {
            rest_words.push(t.raw.clone());
            continue;
        }
        text_words.push((i, t.raw.trim_end_matches(['?', ',', ';']).to_string()));
    }
    let _ = ambiguous_words;
    if let Some((first, _)) = text_words.first() {
        let joined = text_words
            .iter()
            .map(|(_, w)| w.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        out.push((*first, text_chip(&joined)));
    }
    for (pos, q) in quoted {
        out.push((pos, text_chip(&q)));
    }
    out.sort_by_key(|(i, _)| *i);
    Resolved {
        chips: out.into_iter().map(|(_, c)| c).collect(),
        rest: rest_words.join(" "),
        ambiguous,
    }
}

// ------------------------------------------------------------------ merge

/// Merge `new` chips into the bar's `chips` (04 §8.3): bucket chips union,
/// a range (or a bucket after a range) replaces the earlier when; person,
/// repo, folder and type replace; duplicates collapse; status, tag and text
/// chips stay separate (status OR-ed at compile).
pub fn merge(chips: &[Chip], new: &[Chip]) -> Vec<Chip> {
    let mut out: Vec<Chip> = chips.to_vec();
    for c in new {
        if out.iter().any(|x| x.key() == c.key()) {
            continue;
        }
        match &c.value {
            ChipValue::When(WhenFilter::Buckets { r#in }) => {
                if let Some(prev) = out
                    .iter_mut()
                    .find(|x| matches!(x.value, ChipValue::When(WhenFilter::Buckets { .. })))
                {
                    if let ChipValue::When(WhenFilter::Buckets { r#in: have }) = &prev.value {
                        let mut all = have.clone();
                        for b in r#in {
                            if !all.contains(b) {
                                all.push(b.clone());
                            }
                        }
                        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
                        let mut merged = when_chip(&refs);
                        merged.origin = prev.origin.clone();
                        *prev = merged;
                    }
                    continue;
                }
                out.retain(|x| x.kind() != "when");
                out.push(c.clone());
            }
            ChipValue::When(_)
            | ChipValue::Person(_)
            | ChipValue::Repo(_)
            | ChipValue::Folder(_)
            | ChipValue::Type(_) => {
                let k = c.kind();
                out.retain(|x| x.kind() != k);
                out.push(c.clone());
            }
            ChipValue::Status(s) if s == "urgent" || s == "not-urgent" => {
                out.retain(|x| !matches!(&x.value, ChipValue::Status(v) if v == "urgent" || v == "not-urgent"));
                out.push(c.clone());
            }
            _ => out.push(c.clone()),
        }
    }
    out
}

// ---------------------------------------------------------------- compile

/// The result type of a chip set: `notes` unless a type chip says otherwise.
pub fn item_type(chips: &[Chip]) -> String {
    chips
        .iter()
        .rev()
        .find_map(|c| match &c.value {
            ChipValue::Type(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "notes".into())
}

fn compile_with(chips: &[Chip], relative: bool) -> Filter {
    let mut f = Filter::default();
    let mut statuses: Vec<String> = Vec::new();
    let mut all: Vec<String> = Vec::new();
    let mut any: Vec<String> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    for c in chips {
        match &c.value {
            ChipValue::Status(s) if s == "urgent" => f.urgent = Some(true),
            ChipValue::Status(s) if s == "not-urgent" => f.urgent = Some(false),
            ChipValue::Status(s) => {
                if !statuses.contains(s) {
                    statuses.push(s.clone());
                }
            }
            ChipValue::Type(t) if t == "tasks" => {
                f.status.get_or_insert_with(StatusFilter::default).not_in =
                    Some(vec!["closed".into()]);
            }
            ChipValue::Type(_) => {}
            ChipValue::When(w) => {
                f.when = Some(if relative {
                    c.relative.clone().unwrap_or_else(|| w.clone())
                } else {
                    w.clone()
                });
            }
            ChipValue::Tag(TagValue::One(t)) => {
                if !all.contains(t) {
                    all.push(t.clone());
                }
            }
            ChipValue::Tag(TagValue::Any { any: a }) => {
                for t in a {
                    if !any.contains(t) {
                        any.push(t.clone());
                    }
                }
            }
            ChipValue::Person(p) => f.person = Some(p.clone()),
            ChipValue::Repo(r) => f.repo = Some(r.clone()),
            ChipValue::Folder(fr) => {
                f.folder = Some(FolderFilter {
                    id: fr.id.clone(),
                    recursive: true,
                })
            }
            ChipValue::Text(t) => texts.push(t.clone()),
            ChipValue::Flag(fl) => match fl.as_str() {
                "pinned" => f.pinned = Some(true),
                "archived" => f.archived = Some(true),
                s => {
                    if let Some(k) = s.strip_prefix("source:")
                        && !sources.iter().any(|x| x == k)
                    {
                        sources.push(k.to_string());
                    }
                }
            },
        }
    }
    if !statuses.is_empty() {
        f.status.get_or_insert_with(StatusFilter::default).r#in = Some(statuses);
    }
    if !all.is_empty() || !any.is_empty() {
        f.tags = Some(TagsFilter {
            all: (!all.is_empty()).then_some(all),
            any: (!any.is_empty()).then_some(any),
            none: None,
        });
    }
    if !texts.is_empty() {
        f.text = Some(texts.join(" "));
    }
    if !sources.is_empty() {
        f.source_kind = Some(InList { r#in: sources });
    }
    f
}

/// Chips → `filter_json` with literal dates (§8.4, the §8.6 examples).
pub fn compile(chips: &[Chip]) -> Filter {
    compile_with(chips, false)
}

/// Chips → `filter_json` with relative date tokens where the chips were
/// relative ("Save as grouping", §8.5).
pub fn compile_relative(chips: &[Chip]) -> Filter {
    compile_with(chips, true)
}

// ------------------------------------------------------------ card title

fn type_word(t: &str) -> &'static str {
    match t {
        "tasks" => "tasks",
        "messages" => "messages",
        "sessions" => "sessions",
        "calendar_events" => "events",
        _ => "notes",
    }
}

fn chip_text(c: &Chip) -> String {
    match &c.value {
        ChipValue::Text(t) => format!("\u{201C}{t}\u{201D}"),
        _ => match &c.glyph {
            Some(g) => format!("{g} {}", c.label),
            None => c.label.clone(),
        },
    }
}

/// The card title (04 §8.5, 03 §5.9.5): `<Person>'s <type> · <chips> +n`;
/// without a person the type word leads only when it is not notes; at most
/// three segments after the first.
pub fn card_title(chips: &[Chip]) -> String {
    let person = chips.iter().find(|c| c.kind() == "person");
    let ty = item_type(chips);
    let mut first: Option<String> = None;
    if let Some(p) = person {
        first = Some(format!("{}'s {}", p.label, type_word(&ty)));
    } else if ty != "notes" {
        let w = type_word(&ty);
        let mut cs = w.chars();
        first = cs
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + cs.as_str());
    }
    let mut segs: Vec<String> = first.into_iter().collect();
    for c in chips {
        if c.kind() == "person" || c.kind() == "type" {
            continue;
        }
        segs.push(chip_text(c));
    }
    if segs.is_empty() {
        return "Notes".into();
    }
    let shown: Vec<String> = segs.iter().take(4).cloned().collect();
    let folded = segs.len() - shown.len();
    let mut t = shown.join(" \u{00B7} ");
    if folded > 0 {
        t.push_str(&format!(" +{folded}"));
    }
    t
}

// ------------------------------------------------------------ suggestions

/// One autocomplete row (§8.7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub label: String,
    pub glyph: Option<String>,
    /// The kind, or a count, as a muted hint.
    pub hint: String,
    /// Ghost text after the caret when the label completes the typed prefix.
    pub completion: Option<String>,
    /// The bar after accepting: its chips and what stays typed.
    pub apply: Applied,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    pub chips: Vec<Chip>,
    pub rest: String,
}

const MAX_SUGGESTIONS: usize = 6;

fn kind_hint(c: &Chip) -> String {
    match c.kind() {
        "when" => "date".into(),
        k => k.into(),
    }
}

/// Up to six suggestions for the text being typed (§8.7): lexicon phrases
/// completing the last words, then tag / folder / contact / repo / status
/// prefix matches, then "Search for '…'".
pub fn suggest(text: &str, bar: &[Chip], e: &Entities, origin: &str) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    let ends_with_space = text.ends_with(char::is_whitespace);
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return out;
    }
    let push = |chip: Chip,
                typed_words: usize,
                typed: &str,
                hint: Option<String>,
                out: &mut Vec<Suggestion>| {
        if out.len() >= MAX_SUGGESTIONS {
            return;
        }
        let id = chip.key();
        if out.iter().any(|s| s.id == id) {
            return;
        }
        let before = words[..words.len() - typed_words].join(" ");
        let res = resolve_text(&before, e);
        let mut chip = chip;
        chip.origin = "autocomplete".into();
        let mut added = res.chips;
        for c in &mut added {
            c.origin = origin.into();
        }
        added.push(chip.clone());
        let label_l = chip.label.to_lowercase();
        let typed_l = typed.to_lowercase();
        let completion = (!typed.is_empty() && label_l.starts_with(&typed_l) && label_l != typed_l)
            .then(|| {
                chip.label
                    .chars()
                    .skip(typed.chars().count())
                    .collect::<String>()
            });
        out.push(Suggestion {
            id,
            label: chip.label.clone(),
            glyph: chip.glyph.clone(),
            hint: hint.unwrap_or_else(|| kind_hint(&chip)),
            completion,
            apply: Applied {
                chips: merge(bar, &added),
                rest: res.rest,
            },
        });
    };
    if !ends_with_space {
        // Lexicon phrases completing the last one to three words.
        let phrases = fixed_phrases();
        let extra: Vec<(&str, Chip)> = vec![
            (
                "this week",
                Chip::new(ChipValue::Text(String::new()), "", None),
            ),
            (
                "next week",
                Chip::new(ChipValue::Text(String::new()), "", None),
            ),
            (
                "this month",
                Chip::new(ChipValue::Text(String::new()), "", None),
            ),
            (
                "next month",
                Chip::new(ChipValue::Text(String::new()), "", None),
            ),
            (
                "yesterday",
                Chip::new(ChipValue::Text(String::new()), "", None),
            ),
        ];
        for k in (1..=words.len().min(3)).rev() {
            let typed = words[words.len() - k..].join(" ").to_lowercase();
            let mut exact: Vec<(Chip, String)> = Vec::new();
            let mut prefix: Vec<(Chip, String)> = Vec::new();
            for (p, chip) in phrases.iter().chain(extra.iter()) {
                if !p.starts_with(&typed) || p.split(' ').count() < k {
                    continue;
                }
                let chip = if chip.label.is_empty() {
                    // Resolve the dated phrases against today.
                    let toks: Vec<Tok> = p.split(' ').map(word).collect();
                    let mut sub = toks.clone();
                    match select(&mut sub, lexicon(&toks, e)).into_iter().next() {
                        Some((_, c)) => c,
                        None => continue,
                    }
                } else if let ChipValue::Status(s) = &chip.value {
                    status_chip(s, &e.statuses)
                } else {
                    chip.clone()
                };
                if *p == typed {
                    exact.push((chip, p.to_string()));
                } else {
                    prefix.push((chip, p.to_string()));
                }
            }
            for (chip, _) in exact.into_iter().chain(prefix) {
                push(chip, k, &words[words.len() - k..].join(" "), None, &mut out);
            }
        }
        // Entities by prefix (most used first).
        let last = *words.last().unwrap_or(&"");
        let (want, p) = match last.chars().next() {
            Some('#') => ("tag", &last[1..]),
            Some('@') => ("person", &last[1..]),
            _ if last.starts_with("in:") => ("folder", &last[3..]),
            _ => ("any", last),
        };
        let p_l = p.to_lowercase();
        if !p_l.is_empty() {
            if want == "any" || want == "tag" {
                for (name, norm) in e.tags.iter().filter(|(_, n)| n.starts_with(&p_l)) {
                    let _ = name;
                    push(tag_chip(norm), 1, p, None, &mut out);
                }
            }
            if want == "any" || want == "folder" {
                for f in e.folders.iter().filter(|f| {
                    f.name.to_lowercase().starts_with(&p_l)
                        || f.path.to_lowercase().starts_with(&p_l)
                }) {
                    push(folder_chip(f), 1, p, Some(f.path.clone()), &mut out);
                }
            }
            if want == "any" || want == "person" {
                for c in e.contacts_matching(&p_l, false) {
                    push(person_chip(c), 1, p, None, &mut out);
                }
            }
            if want == "any" {
                for r in e
                    .repos
                    .iter()
                    .filter(|r| basename(r).to_lowercase().starts_with(&p_l))
                {
                    push(repo_chip(r), 1, p, None, &mut out);
                }
                for s in e.statuses.iter().filter(|s| {
                    s.label.to_lowercase().starts_with(&p_l) || s.slug.starts_with(&p_l)
                }) {
                    push(status_chip(&s.slug, &e.statuses), 1, p, None, &mut out);
                }
            }
        }
    }
    // Ambiguous names: the candidates.
    let res = resolve_text(text, e);
    for group in &res.ambiguous {
        for c in group {
            push(c.clone(), 1, "", Some("person".into()), &mut out);
        }
    }
    // Search for '…' (the text fallback), always last.
    out.truncate(MAX_SUGGESTIONS - 1);
    let trimmed = text.trim();
    let mut chips = res.chips.clone();
    for c in &mut chips {
        c.origin = origin.into();
    }
    out.push(Suggestion {
        id: format!("search:{trimmed}"),
        label: format!("Search for \u{2018}{trimmed}\u{2019}"),
        glyph: None,
        hint: "text".into(),
        completion: None,
        apply: Applied {
            chips: merge(bar, &[text_chip(trimmed)]),
            rest: String::new(),
        },
    });
    out
}

// -------------------------------------------------------------- the API

/// `query_resolve` (§8): per keystroke the chips stay as they are and the
/// list suggests; on Enter (`final`) the whole text resolves into chips.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub chips: Vec<Chip>,
    pub rest: String,
    pub suggestions: Vec<Suggestion>,
}

pub fn resolve(
    cx: &Cx,
    text: &str,
    bar: &[Chip],
    final_: bool,
    origin: &str,
) -> Result<Resolution> {
    let e = Entities::load(cx)?;
    Ok(resolve_with(&e, text, bar, final_, origin))
}

pub fn resolve_with(
    e: &Entities,
    text: &str,
    bar: &[Chip],
    final_: bool,
    origin: &str,
) -> Resolution {
    if !final_ {
        return Resolution {
            chips: bar.to_vec(),
            rest: text.to_string(),
            suggestions: if text.trim().is_empty() {
                Vec::new()
            } else {
                suggest(text, bar, e, origin)
            },
        };
    }
    let mut r = resolve_text(text, e);
    for c in &mut r.chips {
        c.origin = origin.into();
    }
    let suggestions = if r.ambiguous.is_empty() {
        Vec::new()
    } else {
        r.ambiguous
            .iter()
            .flatten()
            .take(MAX_SUGGESTIONS)
            .map(|c| Suggestion {
                id: c.key(),
                label: c.label.clone(),
                glyph: c.glyph.clone(),
                hint: "person".into(),
                completion: None,
                apply: Applied {
                    chips: merge(&merge(bar, &r.chips), std::slice::from_ref(c)),
                    rest: String::new(),
                },
            })
            .collect()
    };
    Resolution {
        chips: merge(bar, &r.chips),
        rest: r.rest,
        suggestions,
    }
}

/// `query_run` (§8.4): rows and count of a chip set, through the groupings
/// compiler; non-note types (v1.0) return nothing yet.
#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    pub rows: Vec<NoteListRow>,
    pub count: i64,
    /// With relative date tokens where chips were relative (for saving).
    pub filter: Filter,
    /// With literal dates (what ran).
    pub resolved_filter: Filter,
    pub item_type: String,
    pub title: String,
}

pub fn run(cx: &Cx, chips: &[Chip], sort: Option<Vec<SortKey>>, limit: i64) -> Result<RunResult> {
    let ty = item_type(chips);
    let resolved = compile(chips);
    let relative = compile_relative(chips);
    let title = card_title(chips);
    if ty != "notes" && ty != "tasks" {
        return Ok(RunResult {
            rows: Vec::new(),
            count: 0,
            filter: relative,
            resolved_filter: resolved,
            item_type: ty,
            title,
        });
    }
    let opts = RunOpts {
        sort: sort.unwrap_or_else(groupings::today_sort),
        limit,
        ..RunOpts::default()
    };
    let (rows, count) = groupings::run(cx, &resolved, &opts)?;
    Ok(RunResult {
        rows,
        count,
        filter: relative,
        resolved_filter: resolved,
        item_type: ty,
        title,
    })
}

/// A contact's display name for a person filter (labels of saved chips).
pub fn contact_name(cx: &Cx, id: &str) -> Result<Option<String>> {
    Ok(cx
        .conn
        .query_row(
            "SELECT display_name FROM contacts WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ents() -> Entities {
        Entities {
            tags: vec![
                ("gym".into(), "gym".into()),
                ("groceries".into(), "groceries".into()),
                ("errands".into(), "errands".into()),
            ],
            folders: vec![FolderEnt {
                id: "f-gym".into(),
                path: "Gym".into(),
                name: "Gym".into(),
            }],
            contacts: vec![
                Contact {
                    id: "c-bjorn".into(),
                    name: "Bjorn Larsen".into(),
                },
                Contact {
                    id: "c-tim".into(),
                    name: "Tim".into(),
                },
                Contact {
                    id: "c-ana1".into(),
                    name: "Ana Lopez".into(),
                },
                Contact {
                    id: "c-ana2".into(),
                    name: "Ana Silva".into(),
                },
            ],
            repos: vec!["/Users/me/code/mode".into()],
            statuses: serde_json::from_value(crate::settings::default_for("notes.statuses"))
                .unwrap(),
            today: fields::parse_date("2026-10-18"),
            week_start: WeekStart::Monday,
            day_first: true,
        }
    }

    fn filter_of(text: &str) -> serde_json::Value {
        let r = resolve_text(text, &ents());
        serde_json::to_value(compile(&merge(&[], &r.chips))).unwrap()
    }

    #[test]
    fn chips_serialise_as_the_contract_shape() {
        let c = when_chip(&["tomorrow"]);
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(
            v,
            json!({"kind":"when","value":{"in":["tomorrow"]},"label":"Tomorrow","glyph":"▦","origin":"typed"})
        );
        let back: Chip = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);
        let t: Chip = serde_json::from_value(json!({"kind":"tag","value":{"any":["a","b"]},"label":"#a or #b","glyph":null,"origin":"refine"})).unwrap();
        assert_eq!(
            t.value,
            ChipValue::Tag(TagValue::Any {
                any: vec!["a".into(), "b".into()]
            })
        );
    }

    #[test]
    fn ambiguity_and_priorities() {
        // gym is a tag and a folder: the tag wins, the folder is an alternative.
        let r = resolve_text("gym", &ents());
        assert_eq!(r.chips[0].kind(), "tag");
        assert_eq!(r.chips[0].alternatives[0].kind(), "folder");
        // An explicit prefix wins.
        assert_eq!(resolve_text("in:Gym", &ents()).chips[0].kind(), "folder");
        // Two Anas: no chip, the candidates are offered.
        let r = resolve_text("@ana tasks", &ents());
        assert_eq!(r.chips.len(), 1);
        assert_eq!(r.ambiguous[0].len(), 2);
        assert_eq!(r.rest, "@ana");
        // Two buckets merge; a range replaces.
        let f = filter_of("overdue today");
        assert_eq!(f["when"], json!({"in":["overdue","today"]}));
        let f = filter_of("today this week");
        assert_eq!(f["when"], json!({"from":"2026-10-12","to":"2026-10-18"}));
        // Stop words only: nothing.
        assert!(resolve_text("show me all the", &ents()).chips.is_empty());
        // Dates in the locale; the label names the month.
        let r = resolve_text("10/11", &ents());
        assert_eq!(r.chips[0].label, "Nov 10");
        let r = resolve_text("by oct 25", &ents());
        assert_eq!(
            serde_json::to_value(&r.chips[0].value).unwrap()["value"],
            json!({"to":"2026-10-25"})
        );
    }

    #[test]
    fn titles() {
        let e = ents();
        let r = resolve_text("Bjorn's tasks tomorrow", &e);
        assert_eq!(
            card_title(&r.chips),
            "Bjorn Larsen's tasks \u{b7} \u{25A6} Tomorrow"
        );
        let r = resolve_text("plumber", &e);
        assert_eq!(card_title(&r.chips), "\u{201C}plumber\u{201D}");
        let r = resolve_text("urgent doing #gym today pinned captured", &e);
        assert!(
            card_title(&r.chips).ends_with(" +2"),
            "{}",
            card_title(&r.chips)
        );
    }

    #[test]
    fn suggestions() {
        let e = ents();
        let s = suggest("bjorn tasks tomo", &[], &e, "typed");
        assert_eq!(s[0].label, "Tomorrow");
        assert_eq!(s[0].completion.as_deref(), Some("rrow"));
        let kinds: Vec<&str> = s[0].apply.chips.iter().map(Chip::kind).collect();
        assert_eq!(kinds, vec!["person", "type", "when"]);
        assert!(s.last().unwrap().label.starts_with("Search for"));
        let s = suggest("#gr", &[], &e, "typed");
        assert_eq!(s[0].label, "#groceries");
        assert!(s.len() <= MAX_SUGGESTIONS);
    }
}
