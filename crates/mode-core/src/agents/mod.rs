//! Agent profiles (03 §7E, ADR-0058): a reusable configuration a session is
//! launched with. The brain (model, reasoning effort, backup models), what the
//! agent knows (instructions, memories, skills, tables), what it does (its
//! sessions' usage, the automations that run it), its rules (permissions,
//! notifications) and the advanced settings live in `agent_profiles` and
//! `agent_memories`; [`plan`] turns a profile into the `claude` flags and
//! environment a launch adds (05 §8.1), and `sessions.profile_id` says which
//! agent a session ran as.
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::db::Result;
use crate::repo::{self, AgentMemory, AgentProfile};
use crate::scheduler::actions::{FieldError, ferr};
use crate::scheduler::api::SaveError;

/// `--effort` levels the CLI accepts.
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// `--fallback-model` takes at most three.
pub const MAX_FALLBACKS: usize = 3;
pub const MAX_NAME: usize = 60;
pub const MAX_TITLE: usize = 120;
pub const MAX_INSTRUCTIONS: usize = 20_000;
pub const MAX_MEMORY: usize = 20_000;
/// A rule is one short instruction.
pub const MAX_RULE: usize = 500;

fn yes() -> bool {
    true
}

/// What the editor saves (`agent_save`); `id` empty on a new agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentDraft {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub avatar_variant: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub fallback_models: Vec<String>,
    #[serde(default = "yes")]
    pub ask_before_switching: bool,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default = "yes")]
    pub notify: bool,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub claude_path: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub isolate_config: bool,
    /// The provider its sessions run on (`ai_providers.id`, 06 §10.1A);
    /// `None` = Claude Code's own sign-in.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Repository folders (03 §7E.2 Does): a launch with no folder starts in
    /// the first; every repository but the session's folder is `--add-dir`.
    #[serde(default)]
    pub repos: Vec<String>,
    /// MCP servers by name, from the ones Claude Code has configured
    /// ([`mcp`]); `[]` leaves the CLI's own MCP config, else the launch gets
    /// `--mcp-config <file> --strict-mcp-config` with only these.
    #[serde(default)]
    pub connectors: Vec<String>,
    /// Short instructions ("Never edit files; report only"), a "Rules"
    /// section of the system prompt.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Off: the memories stay out of the system prompt.
    #[serde(default = "yes")]
    pub use_memories: bool,
}

/// One memory: a titled note the agent reads at every start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRow {
    pub id: String,
    pub title: String,
    pub body: String,
    pub updated_at: String,
}

/// The sessions of an agent, as the Does section counts them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentUsage {
    pub sessions: i64,
    pub sessions_7d: i64,
    pub tokens: i64,
    pub tokens_7d: i64,
    pub turns: i64,
}

/// An automation whose session action runs this agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationUse {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub last_state: Option<String>,
}

/// A row of the Agents list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub title: Option<String>,
    pub avatar_variant: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub notify: bool,
    pub sessions: i64,
    /// Sessions of the agent still open (`ended_at IS NULL`).
    pub live: i64,
    pub last_used_at: Option<String>,
    /// An automation runs this agent.
    pub automated: bool,
}

/// The edit page (`agent_get`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentView {
    #[serde(flatten)]
    pub draft: AgentDraft,
    pub memories: Vec<MemoryRow>,
    pub usage: AgentUsage,
    pub automations: Vec<AutomationUse>,
    pub last_used_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn list_of(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
}

fn json_of(list: &[String]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".into())
}

/// The draft of a stored profile (the pack exporter reads it too).
pub fn draft_of(r: &AgentProfile) -> AgentDraft {
    AgentDraft {
        id: Some(r.id.clone()),
        name: r.name.clone(),
        title: r.title.clone(),
        instructions: r.instructions.clone(),
        avatar_variant: r.avatar_variant.clone(),
        model: r.model.clone(),
        effort: r.effort.clone(),
        fallback_models: list_of(&r.fallback_models_json),
        ask_before_switching: r.ask_before_switching,
        skills: list_of(&r.skills_json),
        tables: list_of(&r.tables_json),
        permission_mode: r.permission_mode.clone(),
        allowed_tools: list_of(&r.allowed_tools_json),
        denied_tools: list_of(&r.denied_tools_json),
        notify: r.notify,
        workspace: r.workspace.clone(),
        claude_path: r.claude_path.clone(),
        extra_args: list_of(&r.extra_args_json),
        isolate_config: r.isolate_config,
        provider_id: r.provider_id.clone(),
        repos: list_of(&r.repos_json),
        connectors: list_of(&r.connectors_json),
        rules: list_of(&r.rules_json),
        use_memories: r.use_memories,
    }
}

fn memories_of(conn: &Connection, profile_id: &str) -> Result<Vec<MemoryRow>> {
    let rows = repo::query::<AgentMemory>(
        conn,
        "profile_id = ? ORDER BY position, created_at",
        &[&profile_id],
    )?;
    Ok(rows
        .into_iter()
        .map(|m| MemoryRow {
            id: m.id,
            title: m.title,
            body: m.body,
            updated_at: m.updated_at,
        })
        .collect())
}

/// `(sessions, live, last started)` of a profile's top-level rows.
type SessionCounts = HashMap<String, (i64, i64, Option<String>)>;

fn session_counts(conn: &Connection) -> Result<SessionCounts> {
    let mut stmt = conn.prepare(
        "SELECT profile_id, COUNT(*), COALESCE(SUM(ended_at IS NULL), 0), MAX(started_at) FROM sessions
          WHERE profile_id IS NOT NULL AND parent_session_id IS NULL GROUP BY profile_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            (
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
            ),
        ))
    })?;
    let mut out = HashMap::new();
    for r in rows {
        let (k, v) = r?;
        out.insert(k, v);
    }
    Ok(out)
}

fn automations_of(conn: &Connection, profile_id: &str) -> Result<Vec<AutomationUse>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, a.enabled,
                (SELECT started_at FROM automation_runs r WHERE r.automation_id = a.id ORDER BY started_at DESC LIMIT 1),
                (SELECT state FROM automation_runs r WHERE r.automation_id = a.id ORDER BY started_at DESC LIMIT 1)
           FROM automations a
          WHERE a.deleted_at IS NULL AND a.action_kind = 'start_session'
            AND json_extract(a.action_json, '$.profile_id') = ?1
          ORDER BY a.position, a.name",
    )?;
    let rows = stmt.query_map(params![profile_id], |r| {
        Ok(AutomationUse {
            id: r.get(0)?,
            name: r.get(1)?,
            enabled: r.get(2)?,
            last_run_at: r.get(3)?,
            last_state: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn usage_of(conn: &Connection, profile_id: &str, now: &str) -> Result<AgentUsage> {
    let week_ago = crate::db::parse_timestamp(now)
        .map(|t| crate::db::format_timestamp(t - time::Duration::days(7)))
        .unwrap_or_else(|| now.to_string());
    Ok(conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN started_at >= ?2 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens), 0),
                COALESCE(SUM(CASE WHEN started_at >= ?2 THEN input_tokens + output_tokens + cache_read_tokens + cache_write_tokens ELSE 0 END), 0),
                COALESCE(SUM(num_turns), 0)
           FROM sessions WHERE profile_id = ?1 AND parent_session_id IS NULL",
        params![profile_id, week_ago],
        |r| {
            Ok(AgentUsage {
                sessions: r.get(0)?,
                sessions_7d: r.get(1)?,
                tokens: r.get(2)?,
                tokens_7d: r.get(3)?,
                turns: r.get(4)?,
            })
        },
    )?)
}

/// The Agents list, in position order.
pub fn list(conn: &Connection) -> Result<Vec<AgentSummary>> {
    let rows = repo::query::<AgentProfile>(conn, "1 ORDER BY position, name", &[])?;
    let counts = session_counts(conn)?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let (sessions, live, last) = counts.get(&r.id).cloned().unwrap_or((0, 0, None));
        let automated = !automations_of(conn, &r.id)?.is_empty();
        out.push(AgentSummary {
            id: r.id.clone(),
            name: r.name,
            title: r.title,
            avatar_variant: r.avatar_variant,
            model: r.model,
            effort: r.effort,
            notify: r.notify,
            sessions,
            live,
            last_used_at: last,
            automated,
        });
    }
    Ok(out)
}

/// One agent with everything its page shows; `None` for an unknown id.
pub fn get(conn: &Connection, id: &str, now: &str) -> Result<Option<AgentView>> {
    let Some(r) = repo::get_by_id::<AgentProfile>(conn, id)? else {
        return Ok(None);
    };
    let counts = session_counts(conn)?;
    Ok(Some(AgentView {
        draft: draft_of(&r),
        memories: memories_of(conn, id)?,
        usage: usage_of(conn, id, now)?,
        automations: automations_of(conn, id)?,
        last_used_at: counts.get(id).and_then(|(_, _, l)| l.clone()),
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

/// The inline errors of a draft (03 §7E.2); empty when it can be saved.
pub fn validate(d: &AgentDraft) -> Vec<FieldError> {
    let mut out = Vec::new();
    let name = d.name.trim();
    if name.is_empty() {
        out.push(ferr("name", "Give the agent a name."));
    } else if name.chars().count() > MAX_NAME {
        out.push(ferr("name", format!("At most {MAX_NAME} characters.")));
    }
    if d.title
        .as_deref()
        .is_some_and(|t| t.chars().count() > MAX_TITLE)
    {
        out.push(ferr("title", format!("At most {MAX_TITLE} characters.")));
    }
    if d.instructions
        .as_deref()
        .is_some_and(|t| t.chars().count() > MAX_INSTRUCTIONS)
    {
        out.push(ferr(
            "instructions",
            format!("At most {MAX_INSTRUCTIONS} characters."),
        ));
    }
    let model = d.model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    if let Some(m) = model
        && let Err(e) = crate::pty::launch::check_model(m)
    {
        out.push(ferr("model", e));
    }
    if let Some(e) = d.effort.as_deref()
        && !EFFORTS.contains(&e)
    {
        out.push(ferr("effort", "Choose low, medium, high, xhigh or max."));
    }
    if d.fallback_models.len() > MAX_FALLBACKS {
        out.push(ferr(
            "fallback_models",
            format!("At most {MAX_FALLBACKS} backup models."),
        ));
    }
    let mut seen = BTreeSet::new();
    for m in &d.fallback_models {
        let m = m.trim();
        if m.is_empty() {
            out.push(ferr("fallback_models", "A backup model is empty."));
        } else if let Err(e) = crate::pty::launch::check_model(m) {
            out.push(ferr("fallback_models", e));
        } else if !seen.insert(m.to_string()) || model == Some(m) {
            out.push(ferr("fallback_models", format!("{m} is listed twice.")));
        }
    }
    if let Some(p) = d.permission_mode.as_deref()
        && let Err(e) = crate::pty::launch::check_permission_mode(p)
    {
        out.push(ferr("permission_mode", e));
    }
    for (field, list) in [
        ("allowed_tools", &d.allowed_tools),
        ("denied_tools", &d.denied_tools),
        ("skills", &d.skills),
        ("tables", &d.tables),
        ("repos", &d.repos),
        ("connectors", &d.connectors),
        ("rules", &d.rules),
    ] {
        if list.iter().any(|t| t.trim().is_empty()) {
            out.push(ferr(field, "An entry is empty."));
        }
    }
    if d.repos
        .iter()
        .map(|r| r.trim())
        .any(|r| !r.is_empty() && !is_full_path(r))
    {
        out.push(ferr("repos", "Use a full path (/… or ~/…)."));
    }
    if d.rules.iter().any(|r| r.trim().chars().count() > MAX_RULE) {
        out.push(ferr(
            "rules",
            format!("A rule is at most {MAX_RULE} characters."),
        ));
    }
    for a in &d.extra_args {
        let a = a.trim();
        if a == "--bare" {
            out.push(ferr(
                "extra_args",
                "--bare would skip hooks and memory (ADR-0021).",
            ));
        } else if a == "--dangerously-skip-permissions" {
            out.push(ferr("extra_args", "Choose the permission mode instead."));
        } else if a.is_empty() {
            out.push(ferr("extra_args", "An argument is empty."));
        }
    }
    out
}

/// `/abs`, `~/…` or a Windows drive path: a repository must not depend on
/// where Mode was started.
fn is_full_path(p: &str) -> bool {
    p.starts_with("~/") || p == "~" || Path::new(p).is_absolute() || p.starts_with('/')
}

fn invalid(fields: Vec<FieldError>) -> SaveError {
    SaveError {
        code: "invalid".into(),
        message: "The agent cannot be saved as it is.".into(),
        fields,
    }
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn trimmed(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Insert or update (`id` known) a profile; the saved view back.
pub fn save(
    conn: &Connection,
    d: &AgentDraft,
    now: &str,
) -> std::result::Result<AgentView, SaveError> {
    let mut errors = validate(d);
    if let Some(id) = clean(&d.provider_id) {
        match crate::connectors::ai::providers::get(conn, &id)? {
            None => errors.push(ferr("provider_id", "No such provider.")),
            Some(p) if !p.kind.session_capable() => errors.push(ferr(
                "provider_id",
                format!("{} cannot run a session.", p.name),
            )),
            Some(p) if !p.enabled => {
                errors.push(ferr("provider_id", format!("{} is turned off.", p.name)))
            }
            Some(_) => {}
        }
    }
    if !errors.is_empty() {
        return Err(invalid(errors));
    }
    let view = save_row(conn, d, now)?;
    view.ok_or_else(|| SaveError {
        code: "not_found".into(),
        message: "The agent is gone.".into(),
        fields: Vec::new(),
    })
}

fn save_row(conn: &Connection, d: &AgentDraft, now: &str) -> Result<Option<AgentView>> {
    let existing = match d.id.as_deref().filter(|i| !i.is_empty()) {
        Some(id) => repo::get_by_id::<AgentProfile>(conn, id)?,
        None => None,
    };
    let id = existing
        .as_ref()
        .map(|r| r.id.clone())
        .unwrap_or_else(repo::new_id);
    let position = match &existing {
        Some(r) => r.position,
        None => conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM agent_profiles",
            [],
            |r| r.get::<_, i64>(0),
        )?,
    };
    let row = AgentProfile {
        id: id.clone(),
        name: d.name.trim().to_string(),
        title: clean(&d.title),
        instructions: clean(&d.instructions),
        avatar_variant: d.avatar_variant.trim().to_string(),
        model: clean(&d.model),
        effort: d.effort.clone(),
        fallback_models_json: json_of(&trimmed(&d.fallback_models)),
        ask_before_switching: d.ask_before_switching,
        skills_json: json_of(&trimmed(&d.skills)),
        tables_json: json_of(&trimmed(&d.tables)),
        permission_mode: clean(&d.permission_mode),
        allowed_tools_json: json_of(&trimmed(&d.allowed_tools)),
        denied_tools_json: json_of(&trimmed(&d.denied_tools)),
        notify: d.notify,
        workspace: clean(&d.workspace),
        claude_path: clean(&d.claude_path),
        extra_args_json: json_of(&trimmed(&d.extra_args)),
        isolate_config: d.isolate_config,
        position,
        created_at: existing
            .as_ref()
            .map(|r| r.created_at.clone())
            .unwrap_or_else(|| now.to_string()),
        updated_at: now.to_string(),
        provider_id: clean(&d.provider_id),
        repos_json: json_of(&trimmed(&d.repos)),
        connectors_json: json_of(&trimmed(&d.connectors)),
        rules_json: json_of(&trimmed(&d.rules)),
        use_memories: d.use_memories,
    };
    if existing.is_some() {
        conn.execute(
            "UPDATE agent_profiles SET name = ?2, title = ?3, instructions = ?4, avatar_variant = ?5,
                    model = ?6, effort = ?7, fallback_models_json = ?8, ask_before_switching = ?9,
                    skills_json = ?10, tables_json = ?11, permission_mode = ?12, allowed_tools_json = ?13,
                    denied_tools_json = ?14, notify = ?15, workspace = ?16, claude_path = ?17,
                    extra_args_json = ?18, isolate_config = ?19, updated_at = ?20, provider_id = ?21,
                    repos_json = ?22, connectors_json = ?23, rules_json = ?24, use_memories = ?25
              WHERE id = ?1",
            params![
                row.id,
                row.name,
                row.title,
                row.instructions,
                row.avatar_variant,
                row.model,
                row.effort,
                row.fallback_models_json,
                row.ask_before_switching,
                row.skills_json,
                row.tables_json,
                row.permission_mode,
                row.allowed_tools_json,
                row.denied_tools_json,
                row.notify,
                row.workspace,
                row.claude_path,
                row.extra_args_json,
                row.isolate_config,
                row.updated_at,
                row.provider_id,
                row.repos_json,
                row.connectors_json,
                row.rules_json,
                row.use_memories
            ],
        )?;
    } else {
        repo::insert(conn, &row)?;
    }
    get(conn, &id, now)
}

/// Remove a profile; its sessions keep their rows and the automations that
/// ran it run without it (the Agents page says how many before the delete).
pub fn delete(conn: &Connection, id: &str) -> Result<bool> {
    conn.execute(
        "UPDATE sessions SET profile_id = NULL WHERE profile_id = ?1",
        params![id],
    )?;
    conn.execute(
        "UPDATE automations SET action_json = json_remove(action_json, '$.profile_id')
          WHERE action_kind = 'start_session' AND json_extract(action_json, '$.profile_id') = ?1",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM agent_memories WHERE profile_id = ?1",
        params![id],
    )?;
    repo::delete_by_id::<AgentProfile>(conn, id)
}

/// Add or rewrite a memory of a profile.
pub fn memory_save(
    conn: &Connection,
    profile_id: &str,
    id: Option<&str>,
    title: &str,
    body: &str,
    now: &str,
) -> std::result::Result<MemoryRow, SaveError> {
    let title = title.trim();
    let mut errors = Vec::new();
    if title.is_empty() {
        errors.push(ferr("title", "Give the memory a title."));
    }
    if body.chars().count() > MAX_MEMORY {
        errors.push(ferr("body", format!("At most {MAX_MEMORY} characters.")));
    }
    if repo::get_by_id::<AgentProfile>(conn, profile_id)?.is_none() {
        errors.push(ferr("profile_id", "No such agent."));
    }
    if !errors.is_empty() {
        return Err(invalid(errors));
    }
    Ok(memory_row(conn, profile_id, id, title, body, now)?)
}

fn memory_row(
    conn: &Connection,
    profile_id: &str,
    id: Option<&str>,
    title: &str,
    body: &str,
    now: &str,
) -> Result<MemoryRow> {
    let existing = match id.filter(|i| !i.is_empty()) {
        Some(id) => repo::get_by_id::<AgentMemory>(conn, id)?,
        None => None,
    };
    let row = match existing {
        Some(m) => {
            conn.execute(
                "UPDATE agent_memories SET title = ?2, body = ?3, updated_at = ?4 WHERE id = ?1",
                params![m.id, title, body, now],
            )?;
            AgentMemory {
                title: title.into(),
                body: body.into(),
                updated_at: now.into(),
                ..m
            }
        }
        None => {
            let position: i64 = conn.query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM agent_memories WHERE profile_id = ?1",
                params![profile_id],
                |r| r.get(0),
            )?;
            let m = AgentMemory {
                id: repo::new_id(),
                profile_id: profile_id.into(),
                title: title.into(),
                body: body.into(),
                position,
                created_at: now.into(),
                updated_at: now.into(),
            };
            repo::insert(conn, &m)?;
            m
        }
    };
    conn.execute(
        "UPDATE agent_profiles SET updated_at = ?2 WHERE id = ?1",
        params![profile_id, now],
    )?;
    Ok(MemoryRow {
        id: row.id,
        title: row.title,
        body: row.body,
        updated_at: row.updated_at,
    })
}

pub fn memory_delete(conn: &Connection, id: &str) -> Result<bool> {
    repo::delete_by_id::<AgentMemory>(conn, id)
}

/// The profiles whose sessions raise no alerts (`notify = 0`).
pub fn muted_profiles(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM agent_profiles WHERE notify = 0")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ------------------------------------------------------------ the launch

/// What a launch adds for a profile (05 §8.1): the flags after the
/// invocation's own, the environment and a binary of its own. `model` and
/// `permission_mode` are the profile's defaults; a launch that names them
/// wins.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LaunchFlags {
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub effort: Option<String>,
    /// `--fallback-model a,b`; empty when the agent asks before switching.
    pub fallback_models: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    /// The instructions and memories, inline (`--append-system-prompt`) …
    pub system_prompt: Option<String>,
    /// … or as a file (`--append-system-prompt-file`) when a folder was given.
    pub system_prompt_file: Option<PathBuf>,
    /// `--add-dir` per table folder, and per repository other than the
    /// session's folder once [`LaunchFlags::for_cwd`] has run.
    pub add_dirs: Vec<String>,
    /// The agent's repositories, resolved; the first is the folder of a
    /// launch that names none.
    pub repos: Vec<String>,
    /// `--mcp-config`: the per-launch file (or the JSON inline when no
    /// folder was given) holding only the agent's MCP servers …
    pub mcp_config: Option<String>,
    /// … and `--strict-mcp-config`, so no other MCP server loads.
    pub strict_mcp_config: bool,
    /// `--settings` inline: the skills turned off.
    pub settings_json: Option<String>,
    pub extra_args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub claude_path: Option<String>,
}

impl LaunchFlags {
    /// Nothing to add.
    pub fn is_empty(&self) -> bool {
        *self == LaunchFlags::default()
    }

    /// Where a launch that names no folder starts: the first repository.
    pub fn start_dir(&self) -> Option<&str> {
        self.repos.first().map(String::as_str)
    }

    /// The flags for a session in `cwd`: every repository but `cwd` joins
    /// `add_dirs` (once; calling it again changes nothing).
    pub fn for_cwd(mut self, cwd: &Path) -> Self {
        let here = crate::paths::canon(&cwd.to_string_lossy());
        for r in &self.repos {
            if crate::paths::canon(r) != here && !self.add_dirs.contains(r) {
                self.add_dirs.push(r.clone());
            }
        }
        self
    }
}

/// What a plan may reach: the app's agents folder (the system prompt file,
/// an isolated `CLAUDE_CONFIG_DIR`), the notes vault (relative tables) and
/// the keyring (a session provider's secret, 06 §10.1A).
#[derive(Clone, Default)]
pub struct PlanDirs {
    pub agents_dir: Option<PathBuf>,
    pub vault: Option<PathBuf>,
    pub keyring: Option<std::sync::Arc<crate::keyring::Keyring>>,
    /// Claude Code's config file holding `mcpServers` (`None`: the CLI's
    /// own, [`mcp::claude_json`]).
    pub claude_json: Option<PathBuf>,
}

impl std::fmt::Debug for PlanDirs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanDirs")
            .field("agents_dir", &self.agents_dir)
            .field("vault", &self.vault)
            .field("keyring", &self.keyring.is_some())
            .field("claude_json", &self.claude_json)
            .finish()
    }
}

/// The text appended to the system prompt: the instructions, the rules,
/// then the memories (left out when the agent does not use them).
pub fn system_prompt(d: &AgentDraft, memories: &[MemoryRow]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(i) = d
        .instructions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        parts.push(i.to_string());
    }
    let rules = trimmed(&d.rules);
    if !rules.is_empty() {
        let mut r = String::from("# Rules\n\nAlways follow these rules.\n\n");
        for rule in &rules {
            r.push_str(&format!("- {rule}\n"));
        }
        parts.push(r.trim_end().to_string());
    }
    if d.use_memories && !memories.is_empty() {
        let mut m = String::from("# Memories\n\nWhat this agent was told to remember.\n");
        for r in memories {
            m.push_str(&format!("\n## {}\n\n{}\n", r.title.trim(), r.body.trim()));
        }
        parts.push(m);
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// A repository as a folder: `~/` expanded; a relative entry is refused
/// by [`validate`] and skipped here.
pub fn resolve_repo(r: &str) -> Option<String> {
    let r = r.trim();
    let p = match r.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()?.join(rest),
        None if r == "~" => dirs::home_dir()?,
        None => PathBuf::from(r),
    };
    (p.is_absolute() || r.starts_with('/')).then(|| p.to_string_lossy().into_owned())
}

/// Write a file only its owner reads (the MCP config may carry the
/// servers' environment, as `~/.claude.json` does).
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// A table entry as a folder `--add-dir` takes: `~/` expanded, relative to
/// the workspace (or the vault), a file replaced by its folder.
pub fn resolve_table(t: &str, base: Option<&Path>) -> Option<String> {
    let p = match t.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()?.join(rest),
        None => PathBuf::from(t),
    };
    // `/x` counts as absolute on every OS, as in `resolve_repo` (on Windows std says it is not).
    let p = if p.is_absolute() || t.starts_with('/') {
        p
    } else {
        base?.join(p)
    };
    let dir = if p.is_file() {
        p.parent()?.to_path_buf()
    } else {
        p
    };
    Some(dir.to_string_lossy().into_owned())
}

/// The `--settings` JSON that turns off every installed skill not in `on`
/// (`None` when nothing is turned off).
pub fn skill_overrides(installed: &[String], on: &[String]) -> Option<String> {
    let off: serde_json::Map<String, serde_json::Value> = installed
        .iter()
        .filter(|s| !on.contains(s))
        .map(|s| (s.clone(), serde_json::Value::from("off")))
        .collect();
    (!off.is_empty()).then(|| serde_json::json!({ "skillOverrides": off }).to_string())
}

/// The flags of a profile, or `None` for an unknown id.
pub fn plan(conn: &Connection, profile_id: &str, dirs: &PlanDirs) -> Result<Option<LaunchFlags>> {
    let Some(r) = repo::get_by_id::<AgentProfile>(conn, profile_id)? else {
        return Ok(None);
    };
    let d = draft_of(&r);
    let memories = memories_of(conn, profile_id)?;
    let mut f = LaunchFlags {
        model: d.model.clone(),
        permission_mode: d.permission_mode.clone(),
        effort: d.effort.clone(),
        fallback_models: if d.ask_before_switching {
            Vec::new()
        } else {
            d.fallback_models.clone()
        },
        allowed_tools: d.allowed_tools.clone(),
        denied_tools: d.denied_tools.clone(),
        extra_args: d.extra_args.clone(),
        claude_path: d.claude_path.clone(),
        ..Default::default()
    };
    let own = dirs.agents_dir.as_ref().map(|a| a.join(profile_id));
    if let Some(text) = system_prompt(&d, &memories) {
        let written = own.as_ref().and_then(|dir| {
            std::fs::create_dir_all(dir).ok()?;
            let path = dir.join("system.md");
            std::fs::write(&path, &text).ok()?;
            Some(path)
        });
        match written {
            Some(p) => f.system_prompt_file = Some(p),
            None => f.system_prompt = Some(text),
        }
    }
    let base = d
        .workspace
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| dirs.vault.clone());
    f.add_dirs = d
        .tables
        .iter()
        .filter_map(|t| resolve_table(t, base.as_deref()))
        .collect();
    f.repos = d.repos.iter().filter_map(|r| resolve_repo(r)).collect();
    if !d.connectors.is_empty() {
        let claude_json = dirs.claude_json.clone().or_else(mcp::claude_json);
        let folders: Vec<PathBuf> = f
            .repos
            .iter()
            .map(PathBuf::from)
            .chain(d.workspace.as_deref().map(PathBuf::from))
            .collect();
        let config = mcp::config_for(&d.connectors, claude_json.as_deref(), &folders);
        let written = own.as_ref().and_then(|dir| {
            std::fs::create_dir_all(dir).ok()?;
            let path = dir.join("mcp.json");
            write_private(&path, &config).ok()?;
            Some(path.to_string_lossy().into_owned())
        });
        f.mcp_config = Some(written.unwrap_or(config));
        f.strict_mcp_config = true;
    }
    if !d.skills.is_empty() {
        let installed = skills::installed(d.workspace.as_deref().map(Path::new));
        f.settings_json = skill_overrides(&installed, &d.skills);
    }
    if d.isolate_config
        && let Some(dir) = own.as_ref()
    {
        let cfg = dir.join("claude");
        if std::fs::create_dir_all(&cfg).is_ok() {
            f.env.push((
                "CLAUDE_CONFIG_DIR".into(),
                cfg.to_string_lossy().into_owned(),
            ));
        }
    }
    if let Some(pid) = d.provider_id.as_deref() {
        f.env
            .extend(session_env(conn, pid, dirs.keyring.as_deref())?);
    }
    Ok(Some(f))
}

/// The environment a session on the profile's provider gets (06 §10.1A;
/// `providers::session_env`): nothing for an unknown, disabled or
/// session-incapable provider (the launch then runs on Claude Code's own
/// sign-in). The secret is read here and goes only into the child's
/// environment.
fn session_env(
    conn: &Connection,
    provider_id: &str,
    keyring: Option<&crate::keyring::Keyring>,
) -> Result<Vec<(String, String)>> {
    use crate::connectors::ai::providers;
    let Some(p) = providers::get(conn, provider_id)? else {
        return Ok(Vec::new());
    };
    if !p.enabled || !p.kind.session_capable() {
        return Ok(Vec::new());
    }
    let settings = crate::connectors::ai::AiSettings::load(conn)?;
    let url = p.url(&settings);
    let key = keyring.and_then(|k| k.get(&p.key_entry()).ok().flatten());
    Ok(providers::session_env(&p, url.as_deref(), key.as_deref()))
}

pub mod mcp;

/// The skills Claude Code would load: `~/.claude/skills/<name>/SKILL.md`,
/// the workspace's `.claude/skills/<name>/SKILL.md`.
pub mod skills {
    use std::path::Path;

    fn names_in(dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.join("SKILL.md").is_file()
                && let Some(n) = p.file_name().and_then(|n| n.to_str())
            {
                out.push(n.to_string());
            }
        }
    }

    /// Installed skill names, sorted, without duplicates.
    pub fn installed(workspace: Option<&Path>) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(home) = dirs::home_dir() {
            names_in(&home.join(".claude").join("skills"), &mut out);
        }
        if let Some(w) = workspace {
            names_in(&w.join(".claude").join("skills"), &mut out);
        }
        out.sort();
        out.dedup();
        out
    }
}
