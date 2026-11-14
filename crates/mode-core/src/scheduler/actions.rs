//! The three automation actions of 03 §7B.2 and their `action_json`
//! (09 §5.4A): *Start a Claude Code session*, *Create a note*, *Ask Chat and
//! save the answer as a note*; field validation and the action phrase of
//! 10 §12.1 ("Start session in mode · summarize inbox and calendar").

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `permission_mode` values (03 §7B.2).
pub const PERMISSION_MODES: [&str; 6] = [
    "default",
    "plan",
    "acceptEdits",
    "auto",
    "dontAsk",
    "bypassPermissions",
];

/// Chat efforts (03 §7.2).
pub const EFFORTS: [&str; 4] = ["auto", "quick", "thorough", "local_only"];

fn default_permission_mode() -> String {
    "default".into()
}

fn default_mode() -> String {
    "headless".into()
}

fn default_effort() -> String {
    "auto".into()
}

/// `start_session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartSession {
    pub cwd: String,
    pub prompt: String,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    /// `headless` (default) | `embedded`.
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Integer tokens; the run is stopped with SIGINT once the session passes it.
    #[serde(default)]
    pub token_cap: Option<i64>,
    /// The agent profile the session runs as (03 §7E); none = the plain launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

/// `create_note`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateNote {
    #[serde(default)]
    pub template_note_id: Option<String>,
    /// `None` = the capture folder; `root` = the vault root.
    #[serde(default)]
    pub folder_id: Option<String>,
    /// Fields pre-set to match the grouping, as ⊕ Add task does (03 §5.5).
    #[serde(default)]
    pub grouping_slug: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// "Daily plan {date}": `{date}` (22 Oct), `{iso_date}`, `{weekday}`, `{time}`.
    pub title_pattern: String,
}

/// `ask_chat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskChat {
    pub prompt: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    /// `auto` | `quick` | `thorough` | `local_only`.
    #[serde(default = "default_effort")]
    pub effort: String,
}

/// One action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    StartSession(StartSession),
    CreateNote(CreateNote),
    AskChat(AskChat),
}

/// An invalid field (shown inline under it, 03 §7B.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    /// `name`, `schedule.cron`, `action.cwd`, `action.folder_id`, …
    pub field: String,
    pub message: String,
}

pub fn ferr(field: &str, message: impl Into<String>) -> FieldError {
    FieldError {
        field: field.into(),
        message: message.into(),
    }
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Action::StartSession(_) => "start_session",
            Action::CreateNote(_) => "create_note",
            Action::AskChat(_) => "ask_chat",
        }
    }

    /// From `action_kind` + `action_json`.
    pub fn parse(kind: &str, json: &Value) -> Result<Action, FieldError> {
        let shape = |e: serde_json::Error| ferr("action", format!("The action is incomplete: {e}"));
        Ok(match kind {
            "start_session" => {
                Action::StartSession(serde_json::from_value(json.clone()).map_err(shape)?)
            }
            "create_note" => {
                Action::CreateNote(serde_json::from_value(json.clone()).map_err(shape)?)
            }
            "ask_chat" => Action::AskChat(serde_json::from_value(json.clone()).map_err(shape)?),
            other => {
                return Err(ferr(
                    "action_kind",
                    format!("{other:?} is not an action (start_session, create_note, ask_chat)"),
                ));
            }
        })
    }

    pub fn to_json(&self) -> Value {
        match self {
            Action::StartSession(a) => serde_json::to_value(a),
            Action::CreateNote(a) => serde_json::to_value(a),
            Action::AskChat(a) => serde_json::to_value(a),
        }
        .unwrap_or(Value::Null)
    }

    /// Structural checks (no database): required fields and vocabularies.
    pub fn check(&self) -> Vec<FieldError> {
        let mut out = Vec::new();
        match self {
            Action::StartSession(a) => {
                if a.cwd.trim().is_empty() {
                    out.push(ferr("action.cwd", "Choose a repository."));
                } else if !std::path::Path::new(&a.cwd).is_dir() {
                    out.push(ferr("action.cwd", "That folder no longer exists."));
                }
                if a.prompt.trim().is_empty() {
                    out.push(ferr("action.prompt", "Write the prompt."));
                }
                if !PERMISSION_MODES.contains(&a.permission_mode.as_str()) {
                    out.push(ferr(
                        "action.permission_mode",
                        format!("{:?} is not a permission mode", a.permission_mode),
                    ));
                }
                if !matches!(a.mode.as_str(), "headless" | "embedded") {
                    out.push(ferr("action.mode", "Choose headless or embedded."));
                }
                if a.token_cap.is_some_and(|c| c <= 0) {
                    out.push(ferr("action.token_cap", "The token cap must be above 0."));
                }
            }
            Action::CreateNote(a) => {
                if a.title_pattern.trim().is_empty() {
                    out.push(ferr("action.title_pattern", "Write a title pattern."));
                }
            }
            Action::AskChat(a) => {
                if a.prompt.trim().is_empty() {
                    out.push(ferr("action.prompt", "Write the question."));
                }
                if !EFFORTS.contains(&a.effort.as_str()) {
                    out.push(ferr(
                        "action.effort",
                        "Choose Auto, Quick, Thorough or Local-only.",
                    ));
                }
            }
        }
        out
    }
}

/// The prompt's first words for a phrase: one line, lower-cased first
/// letter, an ellipsis past 40 characters.
pub fn first_words(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut chars = line.chars();
    let mut s: String = match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    };
    if s.chars().count() > 40 {
        s = s
            .chars()
            .take(39)
            .collect::<String>()
            .trim_end()
            .to_string();
        s.push('…');
    }
    s
}

/// A title pattern without its placeholders ("Daily plan {date}" → "Daily plan").
pub fn pattern_stem(p: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in p.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The action phrase of 10 §12.1; `folder` resolves a folder id to its
/// vault path (`""` for the root).
pub fn action_phrase(a: &Action, folder: &dyn Fn(Option<&str>) -> String) -> String {
    match a {
        Action::StartSession(s) => {
            let repo = std::path::Path::new(s.cwd.trim_end_matches(['/', '\\']))
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| s.cwd.clone());
            format!("Start session in {repo} · {}", first_words(&s.prompt))
        }
        Action::CreateNote(n) => {
            let f = folder(n.folder_id.as_deref());
            let at = if f.is_empty() {
                "the vault".to_string()
            } else {
                format!("{f}/")
            };
            format!("Create note in {at} · {}", pattern_stem(&n.title_pattern))
        }
        Action::AskChat(c) => format!("Ask Chat · {}", first_words(&c.prompt)),
    }
}

/// Render a title pattern for a local date-time.
pub fn render_title(pattern: &str, local: &jiff::civil::DateTime) -> String {
    const MON: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const WD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let d = local.date();
    pattern
        .replace(
            "{date}",
            &format!("{} {}", d.day(), MON[(d.month() - 1) as usize]),
        )
        .replace(
            "{iso_date}",
            &format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()),
        )
        .replace(
            "{weekday}",
            WD[(d.weekday().to_monday_one_offset() - 1) as usize],
        )
        .replace(
            "{time}",
            &format!("{:02}:{:02}", local.hour(), local.minute()),
        )
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn phrases_read_like_the_copy_examples() {
        let a = Action::parse(
            "start_session",
            &json!({"cwd": "/home/ana/mode", "prompt": "Summarize inbox and calendar"}),
        )
        .unwrap();
        assert_eq!(
            action_phrase(&a, &|_| String::new()),
            "Start session in mode · summarize inbox and calendar"
        );
        let n = Action::parse(
            "create_note",
            &json!({"folder_id": "f1", "title_pattern": "Daily plan {date}"}),
        )
        .unwrap();
        assert_eq!(
            action_phrase(&n, &|_| "Unsorted".into()),
            "Create note in Unsorted/ · Daily plan"
        );
        let c = Action::parse("ask_chat", &json!({"prompt": "What is due today"})).unwrap();
        assert_eq!(
            action_phrase(&c, &|_| String::new()),
            "Ask Chat · what is due today"
        );
        assert!(Action::parse("send_mail", &json!({})).is_err());
    }

    #[test]
    fn titles_render_placeholders() {
        let dt = jiff::civil::date(2026, 10, 14).at(8, 0, 0, 0);
        assert_eq!(render_title("Daily plan {date}", &dt), "Daily plan 14 Oct");
        assert_eq!(
            render_title("{weekday} {iso_date} {time}", &dt),
            "Wed 2026-10-14 08:00"
        );
    }
}
