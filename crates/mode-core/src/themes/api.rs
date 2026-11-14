//! One plain function per theme command (03 §8.5, 10 §4.5), args and result
//! structs per `ui/src/state/contract-themes.ts`, errors as `{code,
//! message}` like `notes::api`:
//!
//! | Command | Args | Result |
//! |---|---|---|
//! | `themes_list` | — | `ThemeRow[]` (built-ins in gallery order, then user packs by name) |
//! | `theme_get` | `{id}` (id or slug) | `ThemeDetail {row, document, report}` |
//! | `theme_set_active` | `{id, mode?: auto\|light\|dark}` | `ResolvedTheme` |
//! | `theme_import` | `{path? \| text?, apply?, on_conflict?: replace\|keep_both}` | `ThemeImportResult {ok, saved, slug, name, appearance, issues[], conflict, swatches, row}` |
//! | `theme_export` | `{id, path?}` | `{path}` |
//! | `theme_delete` | `{id}` (user packs only) | `{deleted, active_reset}` |
//! | `theme_resolve` | `{id?, mode?}` | `ResolvedTheme` |
//! | `theme_duplicate` | `{id, name?}` | `ThemeRow` (a `custom` copy) |
//! | `theme_update` | `{id, document}` (token editor) | `ThemeImportResult` |
//!
//! Mutations request `theme.changed` (`{tokens, dark, …}`) through the
//! service's publisher.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::builtin::{self, BUILTINS};
use super::resolve::{self, AppearancePref, Inputs, OsAppearance, ResolvedTheme};
use super::validate::{self, Report};
use crate::db::{Db, DbError};
use crate::push::Event;
use crate::repo::{self, Theme};
use crate::settings;

pub use crate::notes::api::{ApiError, ApiResult};

/// Largest pack file read on import (six 64 KB sounds in base64 fit).
pub const MAX_PACK_BYTES: u64 = 2 * 1024 * 1024;

fn err(code: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.into(),
    }
}

type PublishFn = Arc<dyn Fn(Event) + Send + Sync>;

/// The themes service: the database, the OS appearance the app reports,
/// where exports go, and the publisher hook for `theme.changed`.
pub struct Themes {
    db: Arc<Db>,
    os: Mutex<OsAppearance>,
    publish: Mutex<Option<PublishFn>>,
    export_dir: PathBuf,
}

impl std::fmt::Debug for Themes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Themes")
            .field("export_dir", &self.export_dir)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Themes {
    /// Re-seed the built-in documents (10 §4.5) and build the service.
    /// `export_dir` is where `theme_export` writes when no path is given.
    pub fn new(db: Arc<Db>, export_dir: PathBuf) -> Result<Arc<Themes>, DbError> {
        if !db.is_read_only() {
            let device = db.device_id().to_string();
            db.write(move |c| {
                builtin::reseed(c, &device, &crate::db::now_rfc3339())?;
                Ok(())
            })?;
        }
        Ok(Arc::new(Themes {
            db,
            os: Mutex::new(OsAppearance::default()),
            publish: Mutex::new(None),
            export_dir,
        }))
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// Where `theme.changed` requests go (`Publisher::publish` in the app).
    pub fn set_publisher(&self, f: impl Fn(Event) + Send + Sync + 'static) {
        *lock(&self.publish) = Some(Arc::new(f));
    }

    pub fn changed(&self) {
        let f = lock(&self.publish).clone();
        if let Some(f) = f {
            f(Event::Theme);
        }
    }

    /// The app reports the OS appearance, accent and accessibility flags
    /// (launch, every OS notification); a change requests `theme.changed`.
    pub fn set_os(&self, os: OsAppearance) -> bool {
        let changed = {
            let mut cur = lock(&self.os);
            let changed = *cur != os;
            *cur = os;
            changed
        };
        if changed {
            self.changed();
        }
        changed
    }

    pub fn os(&self) -> OsAppearance {
        lock(&self.os).clone()
    }

    /// The `theme.changed` payload: `{tokens, dark, slug, name, active,
    /// appearance, notices, accent_adjusted}`.
    pub fn payload(&self) -> Option<Value> {
        theme_resolve(
            self,
            ResolveArgs {
                id: None,
                mode: None,
            },
        )
        .ok()
        .and_then(|r| serde_json::to_value(r).ok())
    }
}

// ------------------------------------------------------------------- rows

/// Six colors of one appearance for a gallery card (bg, surface, fg,
/// accent, ok, info): the core derives them so the webview never reads a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swatch {
    pub bg: String,
    pub surface: String,
    pub fg: String,
    pub accent: String,
    pub ok: String,
    pub info: String,
}

/// The swatches of the appearances a pack has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swatches {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Swatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<Swatch>,
}

/// A themes gallery card (03 §8.5; `ThemeRow` in the contract).
/// `builtin`, `source_path`, `updated_at` and `card_line` are beyond it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeRow {
    pub id: String,
    pub slug: String,
    pub name: String,
    /// `builtin` | `imported` | `custom`.
    pub kind: String,
    /// `light` | `dark` | `auto`.
    pub appearance: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// The active pack (`theme.active`).
    pub active: bool,
    pub swatches: Swatches,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub updated_at: String,
    /// "Gruvbox Light (light) · System Dark at night" for one-appearance packs.
    #[serde(default)]
    pub card_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeDetail {
    pub row: ThemeRow,
    /// The stored pack document (unknown keys kept).
    pub document: Value,
    /// Its validation (beyond `ThemeDetail` in the contract).
    #[serde(default)]
    pub report: Report,
}

/// The document of a row: built-ins from code (the current build's packs),
/// user packs from `tokens_json`.
fn document_of(t: &Theme) -> Value {
    if t.kind == "builtin"
        && let Some(b) = builtin::by_slug(&t.slug)
    {
        return b.document();
    }
    serde_json::from_str(&t.tokens_json).unwrap_or_else(|_| json!({}))
}

fn swatch(doc: &Value, which: &str) -> Option<Swatch> {
    let m = doc.pointer(&format!("/colors/{which}/m"))?;
    let g = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Some(Swatch {
        bg: g("bg"),
        surface: g("surface"),
        fg: g("fg"),
        accent: g("accent"),
        ok: g("ok"),
        info: g("info"),
    })
}

/// The gallery swatches of a pack document.
pub fn swatches(doc: &Value) -> Swatches {
    Swatches {
        light: swatch(doc, "light"),
        dark: swatch(doc, "dark"),
    }
}

fn active_slug(conn: &Connection) -> crate::db::Result<String> {
    Ok(settings::get_string_in(conn, "theme.active")?.unwrap_or_else(|| "system".into()))
}

fn row_of(t: &Theme, active: &str) -> ThemeRow {
    let doc = document_of(t);
    let card_line = match t.appearance.as_str() {
        "light" => Some(format!("{} (light) · System Dark at night", t.name)),
        "dark" => Some(format!("{} (dark) · System Light by day", t.name)),
        _ => None,
    };
    ThemeRow {
        id: t.id.clone(),
        slug: t.slug.clone(),
        name: t.name.clone(),
        kind: t.kind.clone(),
        appearance: t.appearance.clone(),
        version: Some(t.version.clone()).filter(|v| !v.is_empty()),
        author: t.author.clone(),
        active: t.slug == active,
        swatches: swatches(&doc),
        builtin: t.kind == "builtin",
        source_path: t.source_path.clone(),
        updated_at: t.updated_at.clone(),
        card_line,
    }
}

fn live_themes(conn: &Connection) -> crate::db::Result<Vec<Theme>> {
    repo::query::<Theme>(conn, "deleted_at IS NULL ORDER BY name", &[])
}

/// A live pack by id or slug.
fn find(conn: &Connection, id: &str) -> crate::db::Result<Option<Theme>> {
    Ok(repo::query::<Theme>(
        conn,
        "deleted_at IS NULL AND (id = ?1 OR slug = ?1) LIMIT 1",
        &[&id],
    )?
    .into_iter()
    .next())
}

fn not_found(id: &str) -> ApiError {
    err("not_found", format!("No theme pack {id}."))
}

/// `themes_list`: built-ins in the gallery order of 03 §8.5, then imported
/// and custom packs by name.
pub fn themes_list(t: &Themes) -> ApiResult<Vec<ThemeRow>> {
    Ok(t.db.read(|c| {
        let active = active_slug(c)?;
        let all = live_themes(c)?;
        let mut out: Vec<ThemeRow> = BUILTINS
            .iter()
            .filter_map(|b| all.iter().find(|x| x.slug == b.slug && x.kind == "builtin"))
            .map(|x| row_of(x, &active))
            .collect();
        out.extend(
            all.iter()
                .filter(|x| x.kind != "builtin")
                .map(|x| row_of(x, &active)),
        );
        Ok(out)
    })?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeIdArgs {
    /// A theme id or slug.
    pub id: String,
}

/// `theme_get {id}`.
pub fn theme_get(t: &Themes, args: ThemeIdArgs) -> ApiResult<ThemeDetail> {
    let id = args.id.clone();
    let (row, active) = t.db.read(move |c| Ok((find(c, &id)?, active_slug(c)?)))?;
    let row = row.ok_or_else(|| not_found(&args.id))?;
    let document = document_of(&row);
    Ok(ThemeDetail {
        report: validate::validate(&document),
        row: row_of(&row, &active),
        document,
    })
}

// ----------------------------------------------------------- resolution

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveArgs {
    /// A pack to preview; `None` = the active pack.
    #[serde(default)]
    pub id: Option<String>,
    /// `auto` | `light` | `dark`; `None` = `theme.appearance`.
    #[serde(default)]
    pub mode: Option<String>,
}

fn parse_mode(m: Option<&str>) -> ApiResult<Option<AppearancePref>> {
    match m {
        None => Ok(None),
        Some(s) => AppearancePref::parse(s)
            .map(Some)
            .ok_or_else(|| err("invalid", format!("{s:?} is not auto, light or dark"))),
    }
}

/// `theme_resolve {id?, mode?}` → the token map (10 §3.1).
pub fn theme_resolve(t: &Themes, args: ResolveArgs) -> ApiResult<ResolvedTheme> {
    let mode = parse_mode(args.mode.as_deref())?;
    let os = t.os();
    let id = args.id.clone();
    let (pack, term, pref, scale) = t.db.read(move |c| {
        let slug = match &id {
            Some(i) => i.clone(),
            None => active_slug(c)?,
        };
        let pack = find(c, &slug)?.map(|r| document_of(&r));
        let term_slug =
            settings::get_string_in(c, "theme.terminal_pack")?.filter(|s| s != "follow");
        let term = match term_slug {
            Some(s) => find(c, &s)?.map(|r| document_of(&r)),
            None => None,
        };
        let pref = settings::get_string_in(c, "theme.appearance")?
            .and_then(|s| AppearancePref::parse(&s))
            .unwrap_or_default();
        let scale = settings::get_as_in::<f64>(c, "theme.font_scale").ok();
        Ok((pack, term, pref, scale))
    })?;
    if args.id.is_some() && pack.is_none() {
        return Err(not_found(args.id.as_deref().unwrap_or("")));
    }
    let pack = pack.unwrap_or_else(builtin::system_document);
    Ok(resolve::resolve(&Inputs {
        pack: &pack,
        terminal_pack: term.as_ref(),
        appearance: mode.unwrap_or(pref),
        os: &os,
        font_scale: scale,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetActiveArgs {
    pub id: String,
    #[serde(default)]
    pub mode: Option<String>,
}

/// `theme_set_active {id, mode?}`: **Use** on a card (10 §4.5); `mode` also
/// sets `theme.appearance`. Both surfaces repaint on `theme.changed`.
pub fn theme_set_active(t: &Themes, args: SetActiveArgs) -> ApiResult<ResolvedTheme> {
    let mode = parse_mode(args.mode.as_deref())?;
    let device = t.db.device_id().to_string();
    let id = args.id.clone();
    let found = t.db.write(move |c| {
        let Some(row) = find(c, &id)? else {
            return Ok(false);
        };
        settings::set_in(c, &device, "theme.active", &json!(row.slug))?;
        if let Some(m) = mode {
            settings::set_in(c, &device, "theme.appearance", &json!(m.as_str()))?;
        }
        Ok(true)
    })?;
    if !found {
        return Err(not_found(&args.id));
    }
    t.changed();
    theme_resolve(t, ResolveArgs::default())
}

// ----------------------------------------------------------------- import

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    /// Same row, `rev + 1`.
    Replace,
    /// A `-2` slug.
    KeepBoth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportArgs {
    /// A `.json` file (picker, drop, `mode://themes/import?path=`).
    #[serde(default)]
    pub path: Option<String>,
    /// The document itself (paste).
    #[serde(default)]
    pub text: Option<String>,
    /// `true` writes the `themes` row; `false` or absent validates and
    /// previews (10 §4.5: preview first, then **Import**).
    #[serde(default)]
    pub apply: Option<bool>,
    /// The older spelling of `apply: false` (wins when `apply` is absent).
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// What to do when a user pack already has the slug.
    #[serde(default)]
    pub on_conflict: Option<OnConflict>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportOutcome {
    /// Validated, not saved (`apply` false).
    #[default]
    Preview,
    Imported,
    Replaced,
    /// Errors: nothing saved ("Could not import {file}: {n} errors").
    Invalid,
    /// A user pack has the slug: ask Replace / Keep both.
    Conflict,
}

/// Both appearances of a pack, resolved, for the preview card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub light: Option<BTreeMap<String, String>>,
    pub dark: Option<BTreeMap<String, String>>,
}

/// `theme_import` / `theme_update` (`ThemeImportResult` in the contract).
/// `outcome`, `message`, `existing` and the resolved `preview` maps are
/// beyond it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeImportResult {
    /// No errors.
    pub ok: bool,
    /// A `themes` row was written.
    pub saved: bool,
    /// The pack's slug (the stored one when Keep both renamed it).
    pub slug: Option<String>,
    pub name: Option<String>,
    /// `light` | `dark` | `auto`.
    pub appearance: Option<String>,
    /// Every finding (errors, warnings, fills) with its JSON path.
    pub issues: Vec<validate::Issue>,
    /// A user pack has the slug: offer Replace or Keep both.
    #[serde(default)]
    pub conflict: bool,
    /// The preview card's swatches.
    #[serde(default)]
    pub swatches: Option<Swatches>,
    /// The saved row (when `saved`).
    #[serde(default)]
    pub row: Option<ThemeRow>,
    #[serde(default)]
    pub outcome: ImportOutcome,
    /// "Could not import x.json: 3 errors" / "{pack} has {n} contrast warnings; …".
    #[serde(default)]
    pub message: Option<String>,
    /// The colliding row (conflict).
    #[serde(default)]
    pub existing: Option<ThemeRow>,
    /// Both appearances resolved, for a full preview.
    #[serde(default)]
    pub preview: Option<Preview>,
}

impl ThemeImportResult {
    fn new(outcome: ImportOutcome, report: &Report, doc: Option<&Value>) -> ThemeImportResult {
        let s = |k: &str| {
            doc.and_then(|d| d.get(k))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        ThemeImportResult {
            ok: report.valid,
            saved: matches!(outcome, ImportOutcome::Imported | ImportOutcome::Replaced),
            slug: s("slug"),
            name: s("name"),
            appearance: s("appearance"),
            issues: report
                .errors
                .iter()
                .chain(&report.warnings)
                .chain(&report.fills)
                .cloned()
                .collect(),
            conflict: outcome == ImportOutcome::Conflict,
            swatches: doc.filter(|_| report.valid).map(swatches),
            row: None,
            outcome,
            message: None,
            existing: None,
            preview: None,
        }
    }
}

fn preview_of(doc: &Value, os: &OsAppearance) -> Preview {
    let one = |which: &str, pref| {
        doc.pointer(&format!("/colors/{which}")).map(|_| {
            resolve::resolve(&Inputs {
                pack: doc,
                terminal_pack: None,
                appearance: pref,
                os,
                font_scale: None,
            })
            .tokens
        })
    };
    Preview {
        light: one("light", AppearancePref::Light),
        dark: one("dark", AppearancePref::Dark),
    }
}

fn unique_slug(conn: &Connection, base: &str) -> crate::db::Result<String> {
    for n in 2..1000 {
        let mut stem: String = base.chars().take(29).collect();
        stem = stem.trim_end_matches('-').to_string();
        let cand = format!("{stem}-{n}");
        let taken = conn
            .query_row(
                "SELECT 1 FROM themes WHERE slug = ?1 AND deleted_at IS NULL",
                [&cand],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !taken && !builtin::is_reserved(&cand) {
            return Ok(cand);
        }
    }
    Err(DbError::Invalid {
        what: "slug",
        detail: format!("no free slug for {base}"),
    })
}

fn contrast_message(name: &str, report: &Report) -> Option<String> {
    let n = report.contrast().count();
    (n > 0).then(|| format!("{name} has {n} contrast warnings; text may be hard to read."))
}

fn read_pack(args: &ImportArgs) -> ApiResult<(String, Option<String>)> {
    match (&args.text, &args.path) {
        (Some(t), _) => Ok((t.clone(), args.path.clone())),
        (None, Some(p)) => {
            let meta = std::fs::metadata(p).map_err(|e| err("io", format!("{p}: {e}")))?;
            if meta.len() > MAX_PACK_BYTES {
                return Err(err("invalid", format!("{p} is larger than 2 MB")));
            }
            let text = std::fs::read_to_string(p).map_err(|e| err("io", format!("{p}: {e}")))?;
            Ok((text, Some(p.clone())))
        }
        (None, None) => Err(err("invalid", "Give a path or the pack text.")),
    }
}

fn file_label(path: Option<&str>) -> String {
    path.and_then(|p| Path::new(p).file_name())
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the pack".into())
}

/// `theme_import`: validate (10 §4.3), preview (`apply` false or absent:
/// nothing is saved, `conflict` says whether a user pack has the slug),
/// then store a `kind = 'imported'` row with the file plus filled defaults
/// (10 §4.5). A colliding user pack without `on_conflict` is not touched.
pub fn theme_import(t: &Themes, args: ImportArgs) -> ApiResult<ThemeImportResult> {
    let (text, path) = read_pack(&args)?;
    let label = file_label(path.as_deref());
    let apply = args
        .apply
        .unwrap_or_else(|| args.dry_run.is_some_and(|d| !d));
    let mut doc: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            let mut report = Report::default();
            report.errors.push(validate::Issue {
                path: String::new(),
                message: format!("Not a JSON document: {e}."),
                severity: validate::Severity::Error,
                kind: validate::IssueKind::Structure,
                ratio: None,
                required: false,
            });
            let mut r = ThemeImportResult::new(ImportOutcome::Invalid, &report, None);
            r.message = Some(format!("Could not import {label}: 1 errors"));
            return Ok(r);
        }
    };
    let report = validate::validate(&doc);
    if !report.valid {
        let mut r = ThemeImportResult::new(ImportOutcome::Invalid, &report, Some(&doc));
        r.message = Some(format!(
            "Could not import {label}: {} errors",
            report.errors.len()
        ));
        return Ok(r);
    }
    validate::fill_defaults(&mut doc);
    let preview = Some(preview_of(&doc, &t.os()));
    let name = doc["name"].as_str().unwrap_or("").to_string();
    let message = contrast_message(&name, &report);
    if !apply {
        let slug = doc["slug"].as_str().unwrap_or("").to_string();
        let existing = t.db.read(move |c| {
            let active = active_slug(c)?;
            Ok(find(c, &slug)?
                .filter(|e| e.kind != "builtin")
                .map(|e| row_of(&e, &active)))
        })?;
        let mut r = ThemeImportResult::new(ImportOutcome::Preview, &report, Some(&doc));
        r.conflict = existing.is_some();
        r.existing = existing;
        r.preview = preview;
        r.message = message;
        return Ok(r);
    }
    let device = t.db.device_id().to_string();
    let on_conflict = args.on_conflict;
    let stored = t.db.write(move |c| {
        let slug = doc["slug"].as_str().unwrap_or("").to_string();
        let existing = find(c, &slug)?;
        let active = active_slug(c)?;
        let (row, outcome) = match (existing, on_conflict) {
            (Some(e), None) if e.kind != "builtin" => {
                return Ok((row_of(&e, &active), ImportOutcome::Conflict, doc));
            }
            (Some(mut e), Some(OnConflict::Replace)) if e.kind != "builtin" => {
                e.name = name.clone();
                e.appearance = doc["appearance"].as_str().unwrap_or("auto").into();
                e.version = doc["version"].as_str().unwrap_or("1").into();
                e.author = doc["author"].as_str().map(str::to_string);
                e.tokens_json = doc.to_string();
                e.source_path = path.clone();
                e.kind = "imported".into();
                repo::save(c, &device, &mut e)?;
                (e, ImportOutcome::Replaced)
            }
            (existing, _) => {
                let slug = if existing.is_some() || builtin::is_reserved(&slug) {
                    let s = unique_slug(c, &slug)?;
                    doc["slug"] = json!(s);
                    s
                } else {
                    slug
                };
                let mut row = Theme {
                    id: repo::new_id(),
                    slug,
                    name: name.clone(),
                    kind: "imported".into(),
                    appearance: doc["appearance"].as_str().unwrap_or("auto").into(),
                    version: doc["version"].as_str().unwrap_or("1").into(),
                    author: doc["author"].as_str().map(str::to_string),
                    tokens_json: doc.to_string(),
                    source_path: path.clone(),
                    ..Default::default()
                };
                repo::create(c, &device, &mut row)?;
                (row, ImportOutcome::Imported)
            }
        };
        Ok((row_of(&row, &active), outcome, doc))
    })?;
    let (row, outcome, doc) = stored;
    if outcome != ImportOutcome::Conflict && row.active {
        t.changed();
    }
    let mut r = ThemeImportResult::new(outcome, &report, Some(&doc));
    r.preview = preview;
    r.message = message;
    if outcome == ImportOutcome::Conflict {
        r.existing = Some(row);
    } else {
        r.slug = Some(row.slug.clone());
        r.row = Some(row);
    }
    Ok(r)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeUpdateArgs {
    pub id: String,
    pub document: Value,
}

/// `theme_update {id, document}`: the token editor's **Save** on an
/// imported or custom pack (`rev + 1`); the slug stays the row's.
pub fn theme_update(t: &Themes, args: ThemeUpdateArgs) -> ApiResult<ThemeImportResult> {
    let mut doc = args.document;
    let id = args.id.clone();
    let current = t.db.read(move |c| find(c, &id))?;
    let current = current.ok_or_else(|| not_found(&args.id))?;
    if current.kind == "builtin" {
        return Err(err(
            "invalid",
            "Built-in packs cannot be edited; duplicate it first.",
        ));
    }
    doc["slug"] = json!(current.slug);
    let report = validate::validate(&doc);
    if !report.valid {
        let mut r = ThemeImportResult::new(ImportOutcome::Invalid, &report, Some(&doc));
        r.message = Some(format!(
            "Could not save {}: {} errors",
            current.name,
            report.errors.len()
        ));
        return Ok(r);
    }
    validate::fill_defaults(&mut doc);
    let preview = Some(preview_of(&doc, &t.os()));
    let message = contrast_message(&current.name, &report);
    let device = t.db.device_id().to_string();
    let doc2 = doc.clone();
    let row = t.db.write(move |c| {
        let mut e = current;
        e.name = doc2["name"].as_str().unwrap_or(&e.name).to_string();
        e.appearance = doc2["appearance"].as_str().unwrap_or("auto").into();
        e.version = doc2["version"].as_str().unwrap_or("1").into();
        e.author = doc2["author"].as_str().map(str::to_string);
        e.tokens_json = doc2.to_string();
        repo::save(c, &device, &mut e)?;
        Ok(row_of(&e, &active_slug(c)?))
    })?;
    if row.active {
        t.changed();
    }
    let mut r = ThemeImportResult::new(ImportOutcome::Replaced, &report, Some(&doc));
    r.preview = preview;
    r.message = message;
    r.row = Some(row);
    Ok(r)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateArgs {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// `theme_duplicate {id, name?}`: a `custom` copy of any pack, built-ins
/// included (10 §4.5 **Duplicate**).
pub fn theme_duplicate(t: &Themes, args: DuplicateArgs) -> ApiResult<ThemeRow> {
    let device = t.db.device_id().to_string();
    let id = args.id.clone();
    let row = t.db.write(move |c| {
        let Some(src) = find(c, &id)? else {
            return Ok(None);
        };
        let mut doc = document_of(&src);
        let name = args
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("{} copy", src.name));
        let slug = unique_slug(c, &format!("{}-copy", src.slug).replace("--", "-"))?;
        doc["slug"] = json!(slug);
        doc["name"] = json!(name);
        let mut row = Theme {
            id: repo::new_id(),
            slug,
            name,
            kind: "custom".into(),
            appearance: src.appearance.clone(),
            version: src.version.clone(),
            author: src.author.clone(),
            tokens_json: doc.to_string(),
            source_path: None,
            ..Default::default()
        };
        repo::create(c, &device, &mut row)?;
        Ok(Some(row_of(&row, &active_slug(c)?)))
    })?;
    row.ok_or_else(|| not_found(&args.id))
}

// --------------------------------------------------------- export / delete

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportArgs {
    pub id: String,
    /// The file the save dialog chose; default `<export dir>/<slug>.json`.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportResult {
    pub path: String,
}

/// `theme_export {id, path?}`: **Export JSON** writes the stored document.
pub fn theme_export(t: &Themes, args: ExportArgs) -> ApiResult<ExportResult> {
    let id = args.id.clone();
    let row = t.db.read(move |c| find(c, &id))?;
    let row = row.ok_or_else(|| not_found(&args.id))?;
    let doc = document_of(&row);
    let path = match &args.path {
        Some(p) => PathBuf::from(p),
        None => t.export_dir.join(format!("{}.json", row.slug)),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| err("io", e.to_string()))?;
    }
    let mut text = serde_json::to_string_pretty(&doc)?;
    text.push('\n');
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| err("io", e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| err("io", e.to_string()))?;
    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted: bool,
    /// The pack was active: `theme.active` went back to `system`.
    pub active_reset: bool,
}

/// `theme_delete {id}`: tombstones an imported or custom pack (10 §4.5).
pub fn theme_delete(t: &Themes, args: ThemeIdArgs) -> ApiResult<DeleteResult> {
    let device = t.db.device_id().to_string();
    let id = args.id.clone();
    let out = t.db.write(move |c| {
        let Some(row) = find(c, &id)? else {
            return Ok(Err(not_found(&id)));
        };
        if row.kind == "builtin" {
            return Ok(Err(err("invalid", "Built-in packs cannot be removed.")));
        }
        let deleted = repo::soft_delete::<Theme>(c, &device, &row.id)?;
        let reset = deleted && active_slug(c)? == row.slug;
        if reset {
            settings::set_in(c, &device, "theme.active", &json!("system"))?;
        }
        Ok(Ok(DeleteResult {
            deleted,
            active_reset: reset,
        }))
    })??;
    if out.active_reset {
        t.changed();
    }
    Ok(out)
}
