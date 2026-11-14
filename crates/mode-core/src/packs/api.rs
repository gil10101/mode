//! The Marketplace commands (03 §7C.6; `ui/src/state/contract-packs.ts`):
//!
//! | Command | Args | Result |
//! |---|---|---|
//! | `pack_export` | `{agents, automations, themes, name?, slug?, author?, description?, path?}` | `PackExportResult {path, slug, name, counts}` |
//! | `pack_import` | `{text? \| path? \| url?, apply?, on_conflict?, skip?, folders?}` | `PackImportResult` (a preview, or the install's outcome per item) |
//! | `catalogue_get` | `{refresh?}` | `CatalogueResult {catalog?, error?, url}` |
//!
//! `pack_import` without `apply` validates and previews (nothing is
//! written); with `apply: true` it installs every item not in `skip`
//! through the owning stores. The catalog is fetched once per run and on
//! `refresh`; a fetched pack's text is kept so Install needs no second
//! fetch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::catalogue::{self, Catalogue};
use super::{FILE_SUFFIX, ItemKind, ItemPreview, Pack, Selection};
use crate::agents;
use crate::connectors::http::HttpClient;
use crate::db::Db;
use crate::scheduler::Scheduler;
use crate::scheduler::api::{self as sched_api, SaveArgs};
use crate::settings;
use crate::themes::Themes;
use crate::themes::api::{
    self as theme_api, ImportArgs as ThemeImportArgs, ImportOutcome, OnConflict,
};

pub use crate::notes::api::{ApiError, ApiResult};

fn err(code: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.into(),
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The packs service: the database, the stores an install writes through,
/// the HTTP client of the catalog, where exports go.
pub struct Packs {
    db: Arc<Db>,
    scheduler: Option<Arc<Scheduler>>,
    themes: Arc<Themes>,
    http: Arc<dyn HttpClient>,
    export_dir: PathBuf,
    catalogue: Mutex<Option<Catalogue>>,
    /// Pack text by URL, from the last fetch (Install after a preview).
    fetched: Mutex<HashMap<String, String>>,
    clock: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl std::fmt::Debug for Packs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packs")
            .field("export_dir", &self.export_dir)
            .finish_non_exhaustive()
    }
}

impl Packs {
    pub fn new(
        db: Arc<Db>,
        scheduler: Option<Arc<Scheduler>>,
        themes: Arc<Themes>,
        http: Arc<dyn HttpClient>,
        export_dir: PathBuf,
    ) -> Arc<Packs> {
        Arc::new(Packs {
            db,
            scheduler,
            themes,
            http,
            export_dir,
            catalogue: Mutex::new(None),
            fetched: Mutex::new(HashMap::new()),
            clock: None,
        })
    }

    /// A fixed clock (tests).
    pub fn with_clock(
        mut self: Arc<Self>,
        f: impl Fn() -> String + Send + Sync + 'static,
    ) -> Arc<Packs> {
        if let Some(p) = Arc::get_mut(&mut self) {
            p.clock = Some(Arc::new(f));
        }
        self
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    fn now(&self) -> String {
        match &self.clock {
            Some(c) => c(),
            None => crate::db::now_rfc3339(),
        }
    }

    fn device_tz(&self) -> String {
        match &self.scheduler {
            Some(s) => s.device_tz().to_string(),
            None => crate::scheduler::schedule::device_zone(),
        }
    }

    /// `marketplace.index_url`.
    pub fn index_url(&self) -> String {
        settings::get_string(&self.db, "marketplace.index_url")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| catalogue::DEFAULT_INDEX_URL.to_string())
    }
}

// ------------------------------------------------------------------ export

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportArgs {
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub automations: Vec<String>,
    #[serde(default)]
    pub themes: Vec<String>,
    /// The pack's name; default: the one item's name, else "Mode pack".
    #[serde(default)]
    pub name: Option<String>,
    /// Default: from the name.
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// The file to write; default `<export dir>/<slug>.mode-pack.json`.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackExportResult {
    pub path: String,
    pub slug: String,
    pub name: String,
    /// "2 agents · 1 automation".
    pub counts: String,
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `pack_export`: build the pack from the chosen rows and write it.
pub fn pack_export(p: &Packs, args: ExportArgs) -> ApiResult<PackExportResult> {
    let sel = Selection {
        agents: args.agents.clone(),
        automations: args.automations.clone(),
        themes: args.themes.clone(),
    };
    if sel.agents.is_empty() && sel.automations.is_empty() && sel.themes.is_empty() {
        return Err(err("invalid", "Choose at least one item for the pack."));
    }
    let given_name = clean(&args.name);
    let (slug, name) = {
        let sel2 = sel.clone();
        let given = given_name.clone();
        p.db.read(move |c| {
            // The name defaults to the single item's name.
            let name = match given {
                Some(n) => n,
                None => {
                    let single =
                        sel2.agents.len() + sel2.automations.len() + sel2.themes.len() == 1;
                    let mut n = "Mode pack".to_string();
                    if single {
                        if let Some(id) = sel2.agents.first()
                            && let Some(r) =
                                crate::repo::get_by_id::<crate::repo::AgentProfile>(c, id)?
                        {
                            n = r.name;
                        } else if let Some(id) = sel2.automations.first()
                            && let Some(a) = crate::scheduler::store::load(c, id)?
                        {
                            n = a.name;
                        } else if let Some(id) = sel2.themes.first() {
                            let rows = crate::repo::query::<crate::repo::Theme>(
                                c,
                                "deleted_at IS NULL AND (id = ?1 OR slug = ?1) LIMIT 1",
                                &[&id],
                            )?;
                            if let Some(t) = rows.into_iter().next() {
                                n = t.name;
                            }
                        }
                    }
                    n
                }
            };
            Ok((super::slugify(&name), name))
        })?
    };
    let slug = match clean(&args.slug) {
        Some(s) if super::slug_ok(&s) => s,
        Some(s) => {
            return Err(err(
                "invalid",
                format!("{s:?} is not a slug (a–z, 0–9 and -)."),
            ));
        }
        None => slug,
    };
    let (slug2, name2, sel2) = (slug.clone(), name.clone(), sel.clone());
    let mut pack = p.db.read(move |c| super::build(c, &sel2, &slug2, &name2))?;
    pack.author = clean(&args.author);
    pack.description = clean(&args.description);
    let path = match clean(&args.path) {
        Some(s) => PathBuf::from(s),
        None => p.export_dir.join(format!("{slug}{FILE_SUFFIX}")),
    };
    write_json(&path, &serde_json::to_value(&pack)?)?;
    Ok(PackExportResult {
        path: path.to_string_lossy().into_owned(),
        slug,
        name,
        counts: pack.counts_line(),
    })
}

fn write_json(path: &Path, doc: &Value) -> ApiResult<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| err("io", e.to_string()))?;
    }
    let mut text = serde_json::to_string_pretty(doc)?;
    text.push('\n');
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| err("io", e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| err("io", e.to_string()))?;
    Ok(())
}

// ------------------------------------------------------------------ import

/// What to do with an item whose name (agents, automations) or slug
/// (themes) is already here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackConflict {
    /// The existing row is updated in place.
    Replace,
    /// Installed beside it: "<name> 2", a unique theme slug.
    #[default]
    KeepBoth,
}

/// A folder for a `start_session` automation whose folder is not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderOverride {
    pub key: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportArgs {
    /// The pack text (a paste, a dropped file read by the webview).
    #[serde(default)]
    pub text: Option<String>,
    /// A `.mode-pack.json` file.
    #[serde(default)]
    pub path: Option<String>,
    /// A pack of the catalog (fetched; kept for the install).
    #[serde(default)]
    pub url: Option<String>,
    /// `true` installs; `false` or absent previews.
    #[serde(default)]
    pub apply: Option<bool>,
    #[serde(default)]
    pub on_conflict: Option<PackConflict>,
    /// Items left out: `agent:<key>`, `automation:<key>`, `theme:<slug>`.
    #[serde(default)]
    pub skip: Vec<String>,
    #[serde(default)]
    pub folders: Vec<FolderOverride>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackOutcome {
    /// Validated, nothing written.
    Preview,
    /// Every chosen item was written (or skipped by choice).
    Installed,
    /// Some items were written, some failed.
    Partial,
    /// The document is not a pack: nothing to show.
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemOutcome {
    Created,
    Replaced,
    Skipped,
    /// The draft was refused (the preview's issues, or the store's).
    Invalid,
    /// The store failed (the message says why).
    Failed,
}

/// One installed (or not) item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledItem {
    pub kind: ItemKind,
    pub key: String,
    pub name: String,
    pub outcome: ItemOutcome,
    /// The row's id (created / replaced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackImportResult {
    /// The document is a pack (its items may still have issues).
    pub ok: bool,
    pub outcome: PackOutcome,
    pub slug: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    /// "reviewer.mode-pack.json" / the catalog URL / "pasted text".
    pub source: String,
    /// "2 agents · 1 automation · 1 theme".
    pub counts: String,
    pub items: Vec<ItemPreview>,
    /// `Invalid`: why ("Not a JSON document: …").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// After an install.
    #[serde(default)]
    pub installed: Vec<InstalledItem>,
}

impl PackImportResult {
    fn invalid(source: String, message: String) -> PackImportResult {
        PackImportResult {
            ok: false,
            outcome: PackOutcome::Invalid,
            slug: None,
            name: None,
            version: None,
            author: None,
            description: None,
            source,
            counts: String::new(),
            items: Vec::new(),
            message: Some(message),
            installed: Vec::new(),
        }
    }

    fn of(pack: &Pack, source: String, items: Vec<ItemPreview>) -> PackImportResult {
        PackImportResult {
            ok: true,
            outcome: PackOutcome::Preview,
            slug: Some(pack.slug.clone()),
            name: Some(pack.name.clone()),
            version: pack.version.clone(),
            author: pack.author.clone(),
            description: pack.description.clone(),
            source,
            counts: pack.counts_line(),
            items,
            message: None,
            installed: Vec::new(),
        }
    }
}

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// The pack text and its label from the three sources.
fn read_source(p: &Packs, args: &ImportArgs) -> ApiResult<(String, String)> {
    if let Some(t) = &args.text {
        let label = args
            .path
            .as_deref()
            .map(file_label)
            .unwrap_or_else(|| "pasted text".into());
        return Ok((t.clone(), label));
    }
    if let Some(path) = &args.path {
        let meta = std::fs::metadata(path).map_err(|e| err("io", format!("{path}: {e}")))?;
        if meta.len() > super::MAX_PACK_BYTES as u64 {
            return Err(err("invalid", format!("{path} is larger than 2 MB")));
        }
        let text = std::fs::read_to_string(path).map_err(|e| err("io", format!("{path}: {e}")))?;
        return Ok((text, file_label(path)));
    }
    if let Some(url) = &args.url {
        if let Some(t) = lock(&p.fetched).get(url) {
            return Ok((t.clone(), url.clone()));
        }
        let index = catalogue::check_url(&p.index_url()).map_err(|m| err("invalid", m))?;
        let u = catalogue::resolve_pack_url(&index, url).map_err(|m| err("invalid", m))?;
        let text = catalogue::fetch_text(&p.http, &u, super::MAX_PACK_BYTES)
            .map_err(|m| err("offline", m))?;
        lock(&p.fetched).insert(url.clone(), text.clone());
        return Ok((text, url.clone()));
    }
    Err(err(
        "invalid",
        "Give a file, the pack text or a catalog address.",
    ))
}

fn skipped(skip: &[String], kind: ItemKind, key: &str) -> bool {
    let tag = format!("{}:{key}", kind.word());
    skip.iter().any(|s| s == &tag)
}

/// `pack_import`: read, validate and preview; with `apply` install the
/// chosen items through the stores (agents, then automations so a
/// `profile_key` finds its agent, then themes).
pub fn pack_import(p: &Packs, args: ImportArgs) -> ApiResult<PackImportResult> {
    let (text, source) = read_source(p, &args)?;
    let pack = match super::parse(&text) {
        Ok(pk) => pk,
        Err(m) => return Ok(PackImportResult::invalid(source, m)),
    };
    let tz = p.device_tz();
    let pack2 = pack.clone();
    let tz2 = tz.clone();
    let items = p.db.read(move |c| super::preview(c, &pack2, &tz2))?;
    let mut result = PackImportResult::of(&pack, source, items);
    if !args.apply.unwrap_or(false) {
        return Ok(result);
    }
    if pack.is_empty() {
        result.outcome = PackOutcome::Installed;
        result.message = Some("The pack is empty.".into());
        return Ok(result);
    }
    let conflict = args.on_conflict.unwrap_or_default();
    let now = p.now();
    let mut installed = Vec::new();
    let mut agent_ids: HashMap<String, String> = HashMap::new();

    // Agents, in one write.
    {
        let drafts: Vec<(super::AgentItem, ItemPreview)> = pack
            .agents
            .iter()
            .cloned()
            .zip(
                result
                    .items
                    .iter()
                    .filter(|i| i.kind == ItemKind::Agent)
                    .cloned(),
            )
            .collect();
        let skip = args.skip.clone();
        let now2 = now.clone();
        let out: Vec<(InstalledItem, Option<String>)> = p.db.write(move |c| {
            let mut out = Vec::new();
            for (item, pv) in &drafts {
                let base = InstalledItem {
                    kind: ItemKind::Agent,
                    key: item.key.clone(),
                    name: pv.name.clone(),
                    outcome: ItemOutcome::Skipped,
                    id: None,
                    message: None,
                };
                if skipped(&skip, ItemKind::Agent, &item.key) {
                    // A skipped agent that already exists still answers a
                    // `profile_key`, so an automation runs as the agent it names.
                    let existing = super::existing_agent(c, &item.draft.name)?.map(|r| r.id);
                    out.push((base, existing));
                    continue;
                }
                if !pv.installable() {
                    out.push((
                        InstalledItem {
                            outcome: ItemOutcome::Invalid,
                            message: pv.issues.first().map(|e| e.message.clone()),
                            ..base
                        },
                        None,
                    ));
                    continue;
                }
                let mut draft = item.draft.clone();
                draft.id = None;
                draft.workspace = None;
                draft.claude_path = None;
                draft.provider_id = None;
                draft.name = draft.name.trim().to_string();
                let existing = super::existing_agent(c, &draft.name)?;
                let replaced = match (&existing, conflict) {
                    (Some(e), PackConflict::Replace) => {
                        draft.id = Some(e.id.clone());
                        true
                    }
                    (Some(_), PackConflict::KeepBoth) => {
                        draft.name = next_name(c, &draft.name, |c, n| {
                            super::existing_agent(c, n).map(|e| e.is_some())
                        })?;
                        false
                    }
                    (None, _) => false,
                };
                match agents::save(c, &draft, &now2) {
                    Ok(view) => {
                        let id = view.draft.id.clone().unwrap_or_default();
                        out.push((
                            InstalledItem {
                                outcome: if replaced {
                                    ItemOutcome::Replaced
                                } else {
                                    ItemOutcome::Created
                                },
                                id: Some(id.clone()),
                                name: draft.name.clone(),
                                ..base
                            },
                            Some(id),
                        ));
                    }
                    Err(e) => out.push((
                        InstalledItem {
                            outcome: if e.code == "invalid" {
                                ItemOutcome::Invalid
                            } else {
                                ItemOutcome::Failed
                            },
                            message: Some(
                                e.fields
                                    .first()
                                    .map(|f| f.message.clone())
                                    .unwrap_or(e.message),
                            ),
                            ..base
                        },
                        None,
                    )),
                }
            }
            Ok(out)
        })?;
        for (item, id) in out {
            if let Some(id) = id {
                agent_ids.insert(item.key.clone(), id);
            }
            installed.push(item);
        }
    }

    // Automations, one save each through the scheduler.
    for (item, pv) in pack.automations.iter().zip(
        result
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Automation),
    ) {
        let base = InstalledItem {
            kind: ItemKind::Automation,
            key: item.key.clone(),
            name: pv.name.clone(),
            outcome: ItemOutcome::Skipped,
            id: None,
            message: None,
        };
        if skipped(&args.skip, ItemKind::Automation, &item.key) {
            installed.push(base);
            continue;
        }
        if !pv.installable() {
            installed.push(InstalledItem {
                outcome: ItemOutcome::Invalid,
                message: pv.issues.first().map(|e| e.message.clone()),
                ..base
            });
            continue;
        }
        let Some(s) = &p.scheduler else {
            installed.push(InstalledItem {
                outcome: ItemOutcome::Failed,
                message: Some("Automations are unavailable on this database.".into()),
                ..base
            });
            continue;
        };
        let folder = args
            .folders
            .iter()
            .find(|f| f.key == item.key)
            .map(|f| f.cwd.as_str());
        if pv.needs_folder.is_some() && folder.map(str::trim).unwrap_or("").is_empty() {
            installed.push(InstalledItem {
                outcome: ItemOutcome::Invalid,
                message: Some("Choose a folder for its sessions.".into()),
                ..base
            });
            continue;
        }
        let ids = agent_ids.clone();
        let mut draft = super::automation_draft(item, &|k| ids.get(k).cloned(), folder);
        draft.name = draft.name.trim().to_string();
        let name = draft.name.clone();
        let existing = p.db.read(move |c| super::existing_automation(c, &name))?;
        let replaced = match (&existing, conflict) {
            (Some(e), PackConflict::Replace) => {
                draft.id = Some(e.id.clone());
                true
            }
            (Some(_), PackConflict::KeepBoth) => {
                let n = draft.name.clone();
                draft.name = p.db.read(move |c| {
                    next_name(c, &n, |c, n| {
                        super::existing_automation(c, n).map(|e| e.is_some())
                    })
                })?;
                false
            }
            (None, _) => false,
        };
        match sched_api::automation_save(
            s,
            SaveArgs {
                draft: draft.clone(),
            },
        ) {
            Ok(row) => installed.push(InstalledItem {
                outcome: if replaced {
                    ItemOutcome::Replaced
                } else {
                    ItemOutcome::Created
                },
                id: Some(row.id),
                name: draft.name,
                ..base
            }),
            Err(e) => installed.push(InstalledItem {
                outcome: if e.code == "invalid" {
                    ItemOutcome::Invalid
                } else {
                    ItemOutcome::Failed
                },
                message: Some(
                    e.fields
                        .first()
                        .map(|f| f.message.clone())
                        .unwrap_or(e.message),
                ),
                ..base
            }),
        }
    }

    // Themes, through theme_import (its own conflict rules).
    for (doc, pv) in pack
        .themes
        .iter()
        .zip(result.items.iter().filter(|i| i.kind == ItemKind::Theme))
    {
        let base = InstalledItem {
            kind: ItemKind::Theme,
            key: pv.key.clone(),
            name: pv.name.clone(),
            outcome: ItemOutcome::Skipped,
            id: None,
            message: None,
        };
        if skipped(&args.skip, ItemKind::Theme, &pv.key) {
            installed.push(base);
            continue;
        }
        if !pv.installable() {
            installed.push(InstalledItem {
                outcome: ItemOutcome::Invalid,
                message: pv.issues.first().map(|e| e.message.clone()),
                ..base
            });
            continue;
        }
        let on_conflict = match conflict {
            PackConflict::Replace => OnConflict::Replace,
            PackConflict::KeepBoth => OnConflict::KeepBoth,
        };
        let r = theme_api::theme_import(
            &p.themes,
            ThemeImportArgs {
                text: Some(doc.to_string()),
                path: None,
                apply: Some(true),
                dry_run: None,
                on_conflict: Some(on_conflict),
            },
        );
        match r {
            Ok(r) if r.saved => installed.push(InstalledItem {
                outcome: if r.outcome == ImportOutcome::Replaced {
                    ItemOutcome::Replaced
                } else {
                    ItemOutcome::Created
                },
                id: r.row.as_ref().map(|row| row.id.clone()),
                name: r.name.unwrap_or(pv.name.clone()),
                ..base
            }),
            Ok(r) => installed.push(InstalledItem {
                outcome: ItemOutcome::Invalid,
                message: r
                    .message
                    .or_else(|| r.issues.first().map(|i| i.message.clone())),
                ..base
            }),
            Err(e) => installed.push(InstalledItem {
                outcome: ItemOutcome::Failed,
                message: Some(e.message),
                ..base
            }),
        }
    }

    let failed = installed
        .iter()
        .filter(|i| matches!(i.outcome, ItemOutcome::Failed | ItemOutcome::Invalid))
        .count();
    let written = installed
        .iter()
        .filter(|i| matches!(i.outcome, ItemOutcome::Created | ItemOutcome::Replaced))
        .count();
    result.outcome = if failed == 0 {
        PackOutcome::Installed
    } else {
        PackOutcome::Partial
    };
    result.message = Some(match (written, failed) {
        (0, 0) => "Nothing was chosen.".into(),
        (w, 0) => format!("Installed {w} {}.", if w == 1 { "item" } else { "items" }),
        (w, f) => format!(
            "Installed {w} {}; {f} could not be.",
            if w == 1 { "item" } else { "items" }
        ),
    });
    result.installed = installed;
    Ok(result)
}

/// "<name> 2", "<name> 3", … until `taken` says no.
fn next_name<F>(c: &rusqlite::Connection, name: &str, taken: F) -> crate::db::Result<String>
where
    F: Fn(&rusqlite::Connection, &str) -> crate::db::Result<bool>,
{
    for n in 2..100 {
        let cand = format!("{name} {n}");
        if !taken(c, &cand)? {
            return Ok(cand);
        }
    }
    Ok(format!("{name} copy"))
}

// --------------------------------------------------------------- catalog

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueArgs {
    /// Fetch again even when a catalog is held.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueResult {
    /// `marketplace.index_url` as used.
    pub url: String,
    /// The index's host ("raw.githubusercontent.com").
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogue: Option<Catalogue>,
    /// Why there is none (offline, a bad index, a refused URL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `catalogue_get {refresh?}`: the held catalog, fetched when none is
/// held or on `refresh`, or on a changed index URL.
pub fn catalogue_get(p: &Packs, args: CatalogueArgs) -> ApiResult<CatalogueResult> {
    let url = p.index_url();
    let host = catalogue::check_url(&url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    if !args.refresh
        && let Some(c) = lock(&p.catalogue).as_ref().filter(|c| c.url == url)
    {
        return Ok(CatalogueResult {
            url,
            host,
            catalogue: Some(c.clone()),
            error: None,
        });
    }
    match catalogue::fetch(&p.http, &url, &p.now()) {
        Ok(c) => {
            lock(&p.fetched).clear();
            *lock(&p.catalogue) = Some(c.clone());
            Ok(CatalogueResult {
                url,
                host,
                catalogue: Some(c),
                error: None,
            })
        }
        Err(m) => {
            let held = lock(&p.catalogue)
                .as_ref()
                .filter(|c| c.url == url)
                .cloned();
            Ok(CatalogueResult {
                url,
                host,
                catalogue: held,
                error: Some(m),
            })
        }
    }
}
