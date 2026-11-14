//! The app's [`Dispatch`]: *Start a Claude Code session* through the PTY
//! host (`session_launch` per 03 §3.5 with `overview_only` for headless,
//! then `session_prompt` as the first turn), *Create a note* through the
//! notes service (template body, grouping fields as ⊕ Add task, tags, title
//! pattern; `source_kind = 'manual'`), *Ask Chat* through one Chat turn
//! saved as a note (`source_kind = 'chat'`, citations as `references`
//! links).

use std::sync::Arc;

use super::actions::{self, Action, AskChat, CreateNote, StartSession};
use super::engine::{Dispatch, RunContext};
use crate::chat::Chat;
use crate::chat::citations::Citation;
use crate::notes::Notes;
use crate::notes::{capture, groupings, reconcile, write};
use crate::pty::Pty;

/// The live dispatcher; a missing service makes its action skip.
#[derive(Clone, Default)]
pub struct LiveDispatch {
    pub notes: Option<Arc<Notes>>,
    pub pty: Option<Arc<Pty>>,
    pub chat: Option<Arc<Chat>>,
}

impl std::fmt::Debug for LiveDispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveDispatch")
            .field("notes", &self.notes.is_some())
            .field("pty", &self.pty.is_some())
            .field("chat", &self.chat.is_some())
            .finish()
    }
}

/// The 03 §7B.3 skip reason for a missing `claude`.
pub const CLAUDE_MISSING: &str = "claude not found";
/// The notes service is not running (no vault).
pub const NOTES_MISSING: &str = "the notes folder is not available";

impl Dispatch for LiveDispatch {
    fn preflight(&self, action: &Action) -> Option<String> {
        match action {
            Action::StartSession(_) => match &self.pty {
                None => Some(CLAUDE_MISSING.into()),
                Some(p) => match crate::pty::api::claude_locate(p) {
                    Ok(l) if l.found => None,
                    _ => Some(CLAUDE_MISSING.into()),
                },
            },
            Action::CreateNote(_) => self.notes.is_none().then(|| NOTES_MISSING.into()),
            Action::AskChat(a) => match (&self.chat, &self.notes) {
                (None, _) => Some(crate::chat::NO_BACKEND.into()),
                (_, None) => Some(NOTES_MISSING.into()),
                (Some(c), Some(_)) => c.preflight(&a.effort),
            },
        }
    }

    fn start_session(&self, a: &StartSession, _ctx: &RunContext) -> Result<String, String> {
        let pty = self.pty.as_ref().ok_or(CLAUDE_MISSING)?;
        let launched = crate::pty::api::session_launch(
            pty,
            crate::pty::api::LaunchArgs {
                cwd: a.cwd.clone(),
                permission_mode: Some(a.permission_mode.clone()).filter(|m| m != "default"),
                profile_id: a.profile_id.clone(),
                overview_only: Some(a.mode == "headless"),
                ..Default::default()
            },
        )
        .map_err(|e| e.message)?;
        if let Some(e) = launched.error {
            return Err(e);
        }
        crate::pty::api::session_prompt(
            pty,
            crate::pty::api::PromptArgs {
                session_id: launched.session_id.clone(),
                text: a.prompt.clone(),
            },
        )
        .map_err(|e| e.message)?;
        Ok(launched.session_id)
    }

    fn stop_session(&self, session_id: &str) -> Result<(), String> {
        let pty = self.pty.as_ref().ok_or(CLAUDE_MISSING)?;
        crate::pty::api::turn_stop(
            pty,
            crate::pty::api::SessionArgs {
                session_id: session_id.into(),
            },
        )
        .map_err(|e| e.message)
    }

    fn create_note(&self, a: &CreateNote, ctx: &RunContext) -> Result<String, String> {
        let notes = self.notes.as_ref().ok_or(NOTES_MISSING)?;
        create_note_with(notes, a, ctx)
    }

    fn ask_chat(&self, a: &AskChat, ctx: &RunContext) -> Result<String, String> {
        let notes = self.notes.as_ref().ok_or(NOTES_MISSING)?;
        let chat = self.chat.as_ref().ok_or(crate::chat::NO_BACKEND)?;
        let answer = chat.ask_once(&a.prompt, &a.effort)?;
        save_answer(notes, a, ctx, &answer.text, &answer.citations)
    }
}

fn folder_dir(cx: &crate::notes::ctx::Cx, folder_id: Option<&str>) -> crate::db::Result<String> {
    match folder_id {
        None => capture::capture_dir(cx),
        Some("root") | Some("") => Ok(String::new()),
        Some(id) => reconcile::folder_path(cx.conn, id),
    }
}

/// *Create a note* (03 §7B.2): the template's body, the grouping's fields
/// (as ⊕ Add task assigns them), the tags, the rendered title pattern.
pub fn create_note_with(notes: &Notes, a: &CreateNote, ctx: &RunContext) -> Result<String, String> {
    notes.ensure_writable().map_err(|e| e.to_string())?;
    let title = actions::render_title(&a.title_pattern, &ctx.local());
    let a = a.clone();
    let note = notes
        .write(move |cx| {
            let dir = folder_dir(cx, a.folder_id.as_deref())?;
            let mut c = write::new_canonical(cx);
            c.write_status = true;
            if let Some(slug) = a.grouping_slug.as_deref().filter(|s| !s.is_empty()) {
                let g = groupings::by_slug(cx, slug)?;
                let (filter, _) =
                    groupings::parsed(&g).map_err(|e| crate::notes::ctx::invalid("grouping", e))?;
                let plan = groupings::add_task_plan(cx, &filter)?;
                c.status = plan.status;
                c.urgent = plan.urgent;
                c.when = match plan.when.as_deref() {
                    Some("today") => Some(cx.clock.today.clone()),
                    Some("tomorrow") => Some(cx.clock.tomorrow.clone()),
                    _ => None,
                };
                c.tags = plan.tags;
            }
            for t in &a.tags {
                let t = t.trim().trim_start_matches('#');
                if !t.is_empty() && !c.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                    c.tags.push(t.to_string());
                }
            }
            let body = match a.template_note_id.as_deref().filter(|t| !t.is_empty()) {
                Some(tid) => {
                    let tpl = reconcile::live_note(cx.conn, tid)?;
                    format!("# {title}\n\n{}", strip_h1(&tpl.body))
                }
                None => format!("# {title}\n"),
            };
            write::create_file(cx, &dir, &title, &c, &body, "manual")
        })
        .map_err(|e| e.to_string())?;
    notes.changed();
    Ok(note.id)
}

/// A template's body without its first H1 and the blank lines around it
/// (the note gets its own title).
fn strip_h1(body: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut seen_h1 = false;
    for l in body.lines() {
        if out.is_empty() && l.trim().is_empty() {
            continue;
        }
        if out.is_empty() && !seen_h1 && l.trim_start().starts_with("# ") {
            seen_h1 = true;
            continue;
        }
        out.push(l);
    }
    let mut s = out.join("\n");
    if !s.is_empty() {
        s.push('\n');
    }
    s
}

/// *Ask Chat* (03 §7B.4): the answer as a note in the folder,
/// `source_kind = 'chat'`, each citation a `references` link.
pub fn save_answer(
    notes: &Notes,
    a: &AskChat,
    ctx: &RunContext,
    text: &str,
    citations: &[Citation],
) -> Result<String, String> {
    notes.ensure_writable().map_err(|e| e.to_string())?;
    let title = actions::render_title(&format!("{} {{date}}", ctx.automation_name), &ctx.local());
    let mut body = format!("# {title}\n\n{}\n", text.trim());
    if !citations.is_empty() {
        body.push_str("\n## Sources\n\n");
        for c in citations {
            body.push_str(&format!("{}. {} · {}\n", c.n, c.kind_label(), c.title));
        }
    }
    let folder = a.folder_id.clone();
    let cites = citations.to_vec();
    let note = notes
        .write(move |cx| {
            let dir = folder_dir(cx, folder.as_deref())?;
            let mut c = write::new_canonical(cx);
            c.write_status = true;
            let note = write::create_file(cx, &dir, &title, &c, &body, "chat")?;
            for cite in &cites {
                if let Some(kind) = cite.link_kind() {
                    write::link(
                        cx,
                        ("note", &note.id),
                        (kind, &cite.id),
                        "references",
                        None,
                        "ai",
                    )?;
                }
            }
            Ok(note)
        })
        .map_err(|e| e.to_string())?;
    notes.changed();
    Ok(note.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_h1_is_dropped() {
        assert_eq!(strip_h1("# Daily template\n\n- [ ] one\n"), "- [ ] one\n");
        assert_eq!(strip_h1("plain\n"), "plain\n");
    }
}
