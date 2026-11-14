//! Packs: shareable bundles of agents, automations and theme packs (03
//! §7C.6; ADR-0060). A pack is one JSON document (`kind: "mode-pack"`,
//! `format: 1`) carrying up to three lists; it is written by **Export pack**
//! on an agent or automation row and by **Create pack…** on the Marketplace
//! page, read back by **Import pack…** (a file, a drop, a paste) and by the
//! catalog (a read-only index on a GitHub repository, [`catalog`]).
//!
//! Nothing here stores anything of its own: an install writes through the
//! stores that already own the rows ([`crate::agents::save`], the scheduler's
//! `automation_save`, [`crate::themes::api::theme_import`]), so every rule
//! those enforce (validation, `next_run_at`, slug conflicts, the
//! `theme.changed` push) holds for an installed item too. Installed
//! automations arrive **disabled**, as a duplicate does, so a shared pack
//! never fires a session the user did not switch on.
//!
//! What a pack never carries: an agent's memories (its private notes), a
//! provider (`provider_id`, a keyring entry of this machine), the path to
//! `claude` and the workspace, an automation's note folder, template or
//! `profile_id` (replaced by the pack-local `profile_key` of an agent in the
//! same pack). A `start_session` keeps its folder as a hint; when the folder
//! is not on this computer the preview says so and asks for one.

pub mod api;
pub mod catalogue;

use std::collections::BTreeSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agents::{self, AgentDraft};
use crate::db::Result;
use crate::repo::{self, AgentProfile, Automation, Theme};
use crate::scheduler::actions::{Action, FieldError, ferr};
use crate::scheduler::schedule::{self, Schedule};
use crate::scheduler::store::{self as auto_store, AutomationDraft};
use crate::themes::{builtin, validate as theme_validate};

/// The pack format this build writes and reads.
pub const FORMAT: i64 = 1;
/// `kind` of a pack document.
pub const KIND: &str = "mode-pack";
/// A pack file (or a fetched pack) larger than this is refused.
pub const MAX_PACK_BYTES: usize = 2 * 1024 * 1024;
/// Items per list.
pub const MAX_ITEMS: usize = 50;
/// The file name a pack is written under: `<slug>.mode-pack.json`.
pub const FILE_SUFFIX: &str = ".mode-pack.json";

/// One agent of a pack: the draft without this machine's parts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentItem {
    /// Pack-local id (`[a-z0-9-]`), what an automation's `profile_key` names.
    pub key: String,
    #[serde(flatten)]
    pub draft: AgentDraft,
}

/// One automation of a pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationItem {
    pub key: String,
    pub name: String,
    pub schedule: Schedule,
    pub action_kind: String,
    /// The action's fields; `start_session` carries `profile_key` (an agent
    /// of this pack) in place of `profile_id`.
    pub action: Value,
    #[serde(default)]
    pub notify_on_failure: Option<bool>,
    #[serde(default)]
    pub keep_runs: Option<i64>,
}

/// A pack document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pack {
    pub format: i64,
    pub kind: String,
    /// `[a-z0-9-]{2,48}`: the file stem and the catalog's id.
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub agents: Vec<AgentItem>,
    #[serde(default)]
    pub automations: Vec<AutomationItem>,
    /// Theme pack documents (10 §4.2), as `theme_export` writes them.
    #[serde(default)]
    pub themes: Vec<Value>,
}

impl Pack {
    pub fn empty(slug: &str, name: &str) -> Pack {
        Pack {
            format: FORMAT,
            kind: KIND.into(),
            slug: slug.into(),
            name: name.into(),
            version: Some("1".into()),
            author: None,
            description: None,
            agents: Vec::new(),
            automations: Vec::new(),
            themes: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.automations.is_empty() && self.themes.is_empty()
    }

    /// "2 agents · 1 automation · 1 theme".
    pub fn counts_line(&self) -> String {
        counts_line(self.agents.len(), self.automations.len(), self.themes.len())
    }
}

/// "2 agents · 1 automation · 1 theme" (an empty count is left out; all
/// empty reads "empty").
pub fn counts_line(agents: usize, automations: usize, themes: usize) -> String {
    let mut parts = Vec::new();
    let word = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    if agents > 0 {
        parts.push(word(agents, "agent", "agents"));
    }
    if automations > 0 {
        parts.push(word(automations, "automation", "automations"));
    }
    if themes > 0 {
        parts.push(word(themes, "theme", "themes"));
    }
    if parts.is_empty() {
        "empty".into()
    } else {
        parts.join(" · ")
    }
}

/// `[a-z0-9-]{2,48}`, no leading or trailing dash.
pub fn slug_ok(s: &str) -> bool {
    let n = s.chars().count();
    (2..=48).contains(&n)
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// A slug from a name: lowercase, runs of anything else become one dash.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    let out: String = out.chars().take(48).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.chars().count() < 2 {
        "pack".into()
    } else {
        out
    }
}

/// A pack-local key from a name, unique among `taken`.
pub fn unique_key(name: &str, taken: &BTreeSet<String>) -> String {
    let base = slugify(name);
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|k| !taken.contains(k))
        .unwrap_or(base)
}

// ------------------------------------------------------------------ export

/// An agent as a pack carries it: no id, no machine paths, no provider.
pub fn agent_item(key: &str, d: &AgentDraft) -> AgentItem {
    let mut draft = d.clone();
    draft.id = None;
    draft.workspace = None;
    draft.claude_path = None;
    draft.provider_id = None;
    AgentItem {
        key: key.into(),
        draft,
    }
}

/// An automation as a pack carries it: `profile_id` becomes `profile_key`
/// when the agent is in the pack (`agent_keys`: profile id → key), else is
/// dropped; a note folder and template (ids of this vault) are dropped.
pub fn automation_item(
    key: &str,
    a: &Automation,
    agent_keys: &dyn Fn(&str) -> Option<String>,
) -> AutomationItem {
    let mut action: Value = serde_json::from_str(&a.action_json).unwrap_or_else(|_| json!({}));
    if let Some(o) = action.as_object_mut() {
        if let Some(pid) = o
            .remove("profile_id")
            .and_then(|v| v.as_str().map(str::to_string))
            && let Some(k) = agent_keys(&pid)
        {
            o.insert("profile_key".into(), json!(k));
        }
        o.remove("folder_id");
        o.remove("template_note_id");
    }
    AutomationItem {
        key: key.into(),
        name: a.name.clone(),
        schedule: auto_store::schedule_of(a),
        action_kind: a.action_kind.clone(),
        action,
        notify_on_failure: Some(a.notify_on_failure),
        keep_runs: Some(a.keep_runs),
    }
}

/// What **Export pack** / **Create pack…** collect.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub agents: Vec<String>,
    pub automations: Vec<String>,
    pub themes: Vec<String>,
}

/// Build a pack from rows of this database; an unknown id is an error
/// naming it. Theme documents come from the stored pack (built-ins from
/// code), as `theme_export` writes them.
pub fn build(conn: &Connection, sel: &Selection, slug: &str, name: &str) -> Result<Pack> {
    let mut pack = Pack::empty(slug, name);
    let mut keys = BTreeSet::new();
    let mut agent_key_of: Vec<(String, String)> = Vec::new();
    for id in &sel.agents {
        let Some(r) = repo::get_by_id::<AgentProfile>(conn, id)? else {
            return Err(crate::db::DbError::NotFound {
                table: "agent_profiles",
                key: id.clone(),
            });
        };
        let draft = agents::draft_of(&r);
        let key = unique_key(&draft.name, &keys);
        keys.insert(key.clone());
        agent_key_of.push((r.id.clone(), key.clone()));
        pack.agents.push(agent_item(&key, &draft));
    }
    let mut auto_keys = BTreeSet::new();
    for id in &sel.automations {
        let Some(a) = auto_store::load(conn, id)? else {
            return Err(crate::db::DbError::NotFound {
                table: "automations",
                key: id.clone(),
            });
        };
        let key = unique_key(&a.name, &auto_keys);
        auto_keys.insert(key.clone());
        let lookup = |pid: &str| {
            agent_key_of
                .iter()
                .find(|(i, _)| i == pid)
                .map(|(_, k)| k.clone())
        };
        pack.automations.push(automation_item(&key, &a, &lookup));
    }
    for id in &sel.themes {
        let rows = repo::query::<Theme>(
            conn,
            "deleted_at IS NULL AND (id = ?1 OR slug = ?1) LIMIT 1",
            &[&id],
        )?;
        let Some(t) = rows.into_iter().next() else {
            return Err(crate::db::DbError::NotFound {
                table: "themes",
                key: id.clone(),
            });
        };
        let doc = if t.kind == "builtin" {
            builtin::by_slug(&t.slug)
                .map(|b| b.document())
                .unwrap_or_else(|| json!({}))
        } else {
            serde_json::from_str(&t.tokens_json).unwrap_or_else(|_| json!({}))
        };
        pack.themes.push(doc);
    }
    Ok(pack)
}

// ----------------------------------------------------------------- reading

/// A theme pack document on its own (10 §4.2) is read as a pack of one
/// theme, so a file written by **Export pack** on the Appearance page
/// imports here too.
pub fn parse(text: &str) -> std::result::Result<Pack, String> {
    if text.len() > MAX_PACK_BYTES {
        return Err("The pack is larger than 2 MB.".into());
    }
    let doc: Value =
        serde_json::from_str(text).map_err(|e| format!("Not a JSON document: {e}."))?;
    let Some(obj) = doc.as_object() else {
        return Err("A pack is one JSON object.".into());
    };
    let kind = obj.get("kind").and_then(Value::as_str);
    if kind != Some(KIND) {
        // A bare theme document: `colors` with no lists.
        if obj.contains_key("colors") && obj.get("format").and_then(Value::as_i64).is_some() {
            let slug = obj.get("slug").and_then(Value::as_str).unwrap_or("theme");
            let name = obj.get("name").and_then(Value::as_str).unwrap_or("Theme");
            let mut p = Pack::empty(slug, name);
            p.author = obj
                .get("author")
                .and_then(Value::as_str)
                .map(str::to_string);
            p.version = obj
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_string);
            p.themes.push(doc.clone());
            return Ok(p);
        }
        return Err(match kind {
            Some(k) => format!("This is a {k} file, not a Mode pack."),
            None => "This file is not a Mode pack (no `kind`).".into(),
        });
    }
    let format = obj.get("format").and_then(Value::as_i64).unwrap_or(0);
    if format < 1 {
        return Err("`format` is missing.".into());
    }
    if format > FORMAT {
        return Err(format!(
            "This pack needs a newer Mode (format {format}; this Mode reads format {FORMAT})."
        ));
    }
    let pack: Pack =
        serde_json::from_value(doc).map_err(|e| format!("The pack is incomplete: {e}."))?;
    if !slug_ok(&pack.slug) {
        return Err("`slug` must be 2–48 characters of a–z, 0–9 and -.".into());
    }
    if pack.name.trim().is_empty() {
        return Err("`name` is missing.".into());
    }
    if pack.agents.len() > MAX_ITEMS
        || pack.automations.len() > MAX_ITEMS
        || pack.themes.len() > MAX_ITEMS
    {
        return Err(format!("A pack holds at most {MAX_ITEMS} items of a kind."));
    }
    let mut seen = BTreeSet::new();
    for k in pack.agents.iter().map(|a| &a.key) {
        if !slug_ok(k) || !seen.insert(k.clone()) {
            return Err(format!("Agent key {k:?} is not unique or not a key."));
        }
    }
    seen.clear();
    for k in pack.automations.iter().map(|a| &a.key) {
        if !slug_ok(k) || !seen.insert(k.clone()) {
            return Err(format!("Automation key {k:?} is not unique or not a key."));
        }
    }
    Ok(pack)
}

// ----------------------------------------------------------------- preview

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Agent,
    Automation,
    Theme,
}

impl ItemKind {
    pub fn word(self) -> &'static str {
        match self {
            ItemKind::Agent => "agent",
            ItemKind::Automation => "automation",
            ItemKind::Theme => "theme",
        }
    }
}

/// One item of the preview (03 §7C.6): what it is, what it would replace,
/// what is wrong with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemPreview {
    pub kind: ItemKind,
    /// The pack key (themes: the slug).
    pub key: String,
    pub name: String,
    /// "claude-opus-5-5 · high · 2 skills" / "Weekdays 08:00 · Start session in mode · …" / "dark · by Ana".
    pub detail: String,
    /// The name of the row with the same name (agents, automations) or slug
    /// (themes) that **Replace** would overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing: Option<String>,
    /// Field errors that stop the item (an invalid draft).
    #[serde(default)]
    pub issues: Vec<FieldError>,
    /// `start_session`: the folder the automation names is not on this
    /// computer; the install needs one (`overrides`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_folder: Option<String>,
    /// Theme contrast warnings (10 §4.3), shown, not blocking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// An agent's accessory (the bot mark the row draws).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_variant: Option<String>,
    /// A theme's gallery swatches (the color cube the row draws).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swatches: Option<crate::themes::api::Swatches>,
}

impl ItemPreview {
    pub fn installable(&self) -> bool {
        self.issues.is_empty()
    }
}

fn agent_detail(d: &AgentDraft) -> String {
    let mut parts = Vec::new();
    if let Some(t) = d.title.as_deref().filter(|t| !t.trim().is_empty()) {
        parts.push(t.trim().to_string());
    }
    if let Some(m) = d.model.as_deref().filter(|m| !m.trim().is_empty()) {
        parts.push(m.trim().to_string());
    }
    if let Some(e) = d.effort.as_deref().filter(|e| !e.trim().is_empty()) {
        parts.push(e.trim().to_string());
    }
    if !d.skills.is_empty() {
        parts.push(format!(
            "{} {}",
            d.skills.len(),
            if d.skills.len() == 1 {
                "skill"
            } else {
                "skills"
            }
        ));
    }
    if parts.is_empty() {
        "agent".into()
    } else {
        parts.join(" · ")
    }
}

/// The draft an automation item becomes: `profile_key` resolved through
/// `agent_id` (an agent installed from the same pack), the folder override
/// applied, `enabled` false.
pub fn automation_draft(
    item: &AutomationItem,
    agent_id: &dyn Fn(&str) -> Option<String>,
    folder: Option<&str>,
) -> AutomationDraft {
    let mut action = item.action.clone();
    if let Some(o) = action.as_object_mut() {
        let key = o
            .remove("profile_key")
            .and_then(|v| v.as_str().map(str::to_string));
        o.remove("profile_id");
        if let Some(id) = key.and_then(|k| agent_id(&k)) {
            o.insert("profile_id".into(), json!(id));
        }
        if let Some(f) = folder.filter(|f| !f.trim().is_empty())
            && item.action_kind == "start_session"
        {
            o.insert("cwd".into(), json!(f.trim()));
        }
    }
    AutomationDraft {
        id: None,
        name: item.name.clone(),
        enabled: Some(false),
        schedule: item.schedule.clone(),
        action_kind: item.action_kind.clone(),
        action,
        notify_on_failure: item.notify_on_failure,
        keep_runs: item.keep_runs,
        position: None,
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The agent of this database with the item's name.
pub fn existing_agent(conn: &Connection, name: &str) -> Result<Option<AgentProfile>> {
    Ok(repo::query::<AgentProfile>(conn, "1", &[])?
        .into_iter()
        .find(|r| same_name(&r.name, name)))
}

/// The live automation with the item's name.
pub fn existing_automation(conn: &Connection, name: &str) -> Result<Option<Automation>> {
    Ok(repo::query::<Automation>(conn, "deleted_at IS NULL", &[])?
        .into_iter()
        .find(|r| same_name(&r.name, name)))
}

/// The user theme pack with the slug (a built-in slug is reserved: the
/// install renames, never replaces).
pub fn existing_theme(conn: &Connection, slug: &str) -> Result<Option<Theme>> {
    Ok(repo::query::<Theme>(
        conn,
        "deleted_at IS NULL AND slug = ?1 AND kind != 'builtin' LIMIT 1",
        &[&slug],
    )?
    .into_iter()
    .next())
}

/// Preview every item against this database: the existing rows a Replace
/// would overwrite, the field errors of each draft (the stores' own
/// validators), the missing folders. `device_tz` compiles the schedules.
pub fn preview(conn: &Connection, pack: &Pack, device_tz: &str) -> Result<Vec<ItemPreview>> {
    let mut out = Vec::new();
    for a in &pack.agents {
        let mut issues = agents::validate(&a.draft);
        // The install drops the machine parts anyway; a pack that carried
        // them is not wrong, so their errors are not the pack's.
        issues.retain(|e| {
            !matches!(
                e.field.as_str(),
                "workspace" | "claude_path" | "provider_id"
            )
        });
        out.push(ItemPreview {
            kind: ItemKind::Agent,
            key: a.key.clone(),
            name: a.draft.name.trim().to_string(),
            detail: agent_detail(&a.draft),
            existing: existing_agent(conn, &a.draft.name)?.map(|r| r.name),
            issues,
            needs_folder: None,
            warnings: Vec::new(),
            avatar_variant: Some(a.draft.avatar_variant.trim().to_string()),
            swatches: None,
        });
    }
    for item in &pack.automations {
        let draft = automation_draft(item, &|_| None, None);
        let mut needs_folder = None;
        let mut issues = match auto_store::check_draft(conn, &draft, device_tz)? {
            Ok(_) => Vec::new(),
            Err(e) => e,
        };
        if item.action_kind == "start_session" {
            let cwd = item
                .action
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let missing = issues.iter().any(|e| e.field == "action.cwd");
            if missing {
                issues.retain(|e| e.field != "action.cwd");
                needs_folder = Some(cwd);
            }
        }
        let trigger = schedule::trigger_phrase(&item.schedule, device_tz);
        let action = match Action::parse(&item.action_kind, &draft.action) {
            Ok(a) => crate::scheduler::actions::action_phrase(&a, &|id| {
                auto_store::folder_label(conn, id)
            }),
            Err(_) => item.action_kind.clone(),
        };
        out.push(ItemPreview {
            kind: ItemKind::Automation,
            key: item.key.clone(),
            name: item.name.trim().to_string(),
            detail: format!("{trigger} · {action}"),
            existing: existing_automation(conn, &item.name)?.map(|r| r.name),
            issues,
            needs_folder,
            warnings: Vec::new(),
            avatar_variant: None,
            swatches: None,
        });
    }
    for (i, doc) in pack.themes.iter().enumerate() {
        let report = theme_validate::validate(doc);
        let slug = doc
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let name = doc
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let issues: Vec<FieldError> = report
            .errors
            .iter()
            .map(|e| ferr(&format!("themes[{i}].{}", e.path), e.message.clone()))
            .collect();
        let warnings: Vec<String> = report
            .warnings
            .iter()
            .map(|w| format!("{} {}", w.path, w.message))
            .collect();
        let appearance = doc
            .get("appearance")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        let mut detail = appearance.to_string();
        if let Some(a) = doc
            .get("author")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
        {
            detail.push_str(&format!(" · by {a}"));
        }
        let existing = if slug.is_empty() {
            None
        } else {
            existing_theme(conn, &slug)?.map(|t| t.name)
        };
        out.push(ItemPreview {
            kind: ItemKind::Theme,
            key: if slug.is_empty() {
                format!("theme-{}", i + 1)
            } else {
                slug
            },
            name: if name.is_empty() {
                format!("Theme {}", i + 1)
            } else {
                name
            },
            detail,
            existing,
            issues,
            needs_folder: None,
            warnings,
            avatar_variant: None,
            swatches: report.valid.then(|| crate::themes::api::swatches(doc)),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_keys() {
        assert!(slug_ok("reviewer-starter"));
        assert!(!slug_ok("Reviewer"));
        assert!(!slug_ok("-a"));
        assert!(!slug_ok("a"));
        assert_eq!(slugify("Nightly runner (prod)"), "nightly-runner-prod");
        assert_eq!(slugify("!!"), "pack");
        let taken: BTreeSet<String> = ["reviewer".to_string()].into_iter().collect();
        assert_eq!(unique_key("Reviewer", &taken), "reviewer-2");
        assert_eq!(counts_line(2, 1, 1), "2 agents · 1 automation · 1 theme");
        assert_eq!(counts_line(0, 0, 0), "empty");
    }

    #[test]
    fn parse_refuses_what_is_not_a_pack_and_reads_a_bare_theme() {
        assert!(parse("nope").unwrap_err().starts_with("Not a JSON"));
        assert!(
            parse(r#"{"kind":"mode-settings","format":1}"#)
                .unwrap_err()
                .contains("mode-settings")
        );
        assert!(
            parse(r#"{"kind":"mode-pack","format":9,"slug":"x-y","name":"X"}"#)
                .unwrap_err()
                .contains("newer Mode")
        );
        assert!(
            parse(r#"{"kind":"mode-pack","format":1,"slug":"X","name":"X"}"#)
                .unwrap_err()
                .contains("slug")
        );
        let p = parse(
            r#"{"format":1,"slug":"nord-ish","name":"Nordish","appearance":"dark","colors":{}}"#,
        )
        .unwrap();
        assert_eq!(p.themes.len(), 1);
        assert_eq!(p.slug, "nord-ish");
        let p = parse(
            r#"{"kind":"mode-pack","format":1,"slug":"s-t","name":"S","agents":[{"key":"a","name":"A"},{"key":"a","name":"B"}]}"#,
        );
        assert!(p.unwrap_err().contains("not unique"));
    }

    #[test]
    fn an_automation_item_swaps_the_profile_for_a_key_and_back() {
        let a = Automation {
            id: "au1".into(),
            name: "Nightly".into(),
            enabled: true,
            trigger_kind: "schedule".into(),
            schedule_json: r#"{"kind":"daily","time":"23:00"}"#.into(),
            action_kind: "start_session".into(),
            action_json: r#"{"cwd":"/x","prompt":"go","permission_mode":"default","mode":"headless","profile_id":"ag1","folder_id":"f1"}"#.into(),
            notify_on_failure: true,
            keep_runs: 20,
            last_run_at: None,
            next_run_at: None,
            position: 0,
            created_at: String::new(),
            updated_at: String::new(),
            rev: 1,
            device_id: String::new(),
            deleted_at: None,
        };
        let item = automation_item("nightly", &a, &|id| {
            (id == "ag1").then(|| "reviewer".to_string())
        });
        assert_eq!(item.action["profile_key"], "reviewer");
        assert!(item.action.get("profile_id").is_none());
        assert!(item.action.get("folder_id").is_none());
        let d = automation_draft(
            &item,
            &|k| (k == "reviewer").then(|| "ag9".to_string()),
            Some("/y"),
        );
        assert_eq!(d.action["profile_id"], "ag9");
        assert!(d.action.get("profile_key").is_none());
        assert_eq!(d.action["cwd"], "/y");
        assert_eq!(d.enabled, Some(false));
        // Not in the pack: dropped.
        let item2 = automation_item("nightly", &a, &|_| None);
        assert!(item2.action.get("profile_key").is_none());
    }
}
