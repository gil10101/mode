//! The export bundle of Settings → Advanced → Export & Import (03 §8.18):
//! `mode-export-<date>.json`, one JSON document with the sections picked
//! (`settings` = the user-scope settings of `mode_core::settings::Bundle`,
//! plus the `groupings`, `alert_rules`, `contacts` and user-made `themes`
//! rows). Never tokens (they live in the keyring), never the vault.
//!
//! Import is a merge by id and `rev` with a preview first: a row missing
//! here is `new`; a row whose content (stamps aside) is the same is
//! `unchanged`; a row that differs is `updated` when the file's `rev` is at
//! least ours, else `skipped` ("changed here since"); a row deleted here is
//! `skipped`. Written rows get `rev + 1` and a `change_log` entry through
//! the repository helpers. A bare settings bundle (`kind: mode-settings`)
//! imports too.

use std::collections::BTreeMap;

use mode_core::db::{DbError, Result};
use mode_core::repo::{self, AlertRule, Contact, Grouping, Syncable, Theme};
use mode_core::settings::{self, ImportOutcome, ImportReport};
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EXPORT_KIND: &str = "mode-export";
pub const EXPORT_FORMAT: u32 = 1;

/// The sections the export dialog offers (`ExportSection`).
pub const SECTIONS: &[&str] = &["settings", "groupings", "alert_rules", "contacts", "themes"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBundle {
    pub format: u32,
    pub kind: String,
    pub exported_at: String,
    pub app_version: String,
    pub sections: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<settings::Bundle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groupings: Option<Vec<Grouping>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert_rules: Option<Vec<AlertRule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contacts: Option<Vec<Contact>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub themes: Option<Vec<Theme>>,
}

fn invalid(detail: impl Into<String>) -> DbError {
    DbError::Invalid {
        what: "export bundle",
        detail: detail.into(),
    }
}

/// Check the section names (unknown ones refused, duplicates dropped).
pub fn check_sections(sections: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for s in sections {
        if !SECTIONS.contains(&s.as_str()) {
            return Err(invalid(format!("{s:?} is not an export section")));
        }
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    if out.is_empty() {
        return Err(invalid("pick at least one section"));
    }
    Ok(out)
}

/// Build the bundle from the database.
pub fn export_in(
    conn: &Connection,
    sections: &[String],
    include_device: bool,
) -> Result<ExportBundle> {
    let sections = check_sections(sections)?;
    let has = |s: &str| sections.iter().any(|x| x == s);
    Ok(ExportBundle {
        format: EXPORT_FORMAT,
        kind: EXPORT_KIND.into(),
        exported_at: mode_core::db::now_rfc3339(),
        app_version: mode_core::VERSION.into(),
        settings: if has("settings") {
            Some(settings::export_in(conn, include_device)?)
        } else {
            None
        },
        groupings: if has("groupings") {
            Some(repo::list_live::<Grouping>(conn, "position, name")?)
        } else {
            None
        },
        alert_rules: if has("alert_rules") {
            Some(repo::list_live::<AlertRule>(conn, "position, name")?)
        } else {
            None
        },
        contacts: if has("contacts") {
            Some(repo::list_live::<Contact>(conn, "display_name")?)
        } else {
            None
        },
        themes: if has("themes") {
            Some(
                repo::list_live::<Theme>(conn, "name")?
                    .into_iter()
                    .filter(|t| t.kind != "builtin")
                    .collect(),
            )
        } else {
            None
        },
        sections,
    })
}

/// Read a file's text as a bundle (an export, or a bare settings bundle).
pub fn parse(text: &str) -> Result<ExportBundle> {
    let v: Value = serde_json::from_str(text).map_err(|e| invalid(format!("not JSON: {e}")))?;
    match v.get("kind").and_then(Value::as_str) {
        Some(EXPORT_KIND) => {
            let b: ExportBundle = serde_json::from_value(v)
                .map_err(|e| invalid(format!("not a Mode export: {e}")))?;
            if b.format > EXPORT_FORMAT {
                return Err(invalid(format!(
                    "format {} needs a newer Mode (this build reads {EXPORT_FORMAT})",
                    b.format
                )));
            }
            Ok(b)
        }
        Some(settings::BUNDLE_KIND) => {
            let s: settings::Bundle = serde_json::from_value(v)
                .map_err(|e| invalid(format!("not a settings bundle: {e}")))?;
            Ok(ExportBundle {
                format: EXPORT_FORMAT,
                kind: EXPORT_KIND.into(),
                exported_at: s.exported_at.clone(),
                app_version: s.app_version.clone(),
                sections: vec!["settings".into()],
                settings: Some(s),
                groupings: None,
                alert_rules: None,
                contacts: None,
                themes: None,
            })
        }
        _ => Err(invalid("this file is not a Mode export")),
    }
}

/// Merge the bundle: the preview (`apply = false`) or the write. Report
/// keys are setting keys and `<section>/<name>` for rows.
pub fn import_in(
    conn: &Connection,
    device_id: &str,
    bundle: &ExportBundle,
    include_device: bool,
    apply: bool,
) -> Result<ImportReport> {
    let mut report = match &bundle.settings {
        Some(s) => settings::import_in(conn, device_id, s, include_device, apply)?,
        None => ImportReport {
            applied: apply,
            entries: BTreeMap::new(),
        },
    };
    report.applied = apply;
    let e = &mut report.entries;
    if let Some(rows) = &bundle.groupings {
        import_rows(
            conn,
            device_id,
            "groupings",
            rows,
            |g| g.name.clone(),
            apply,
            e,
        )?;
    }
    if let Some(rows) = &bundle.alert_rules {
        import_rows(
            conn,
            device_id,
            "alert_rules",
            rows,
            |r| r.name.clone(),
            apply,
            e,
        )?;
    }
    if let Some(rows) = &bundle.contacts {
        import_rows(
            conn,
            device_id,
            "contacts",
            rows,
            |c| c.display_name.clone(),
            apply,
            e,
        )?;
    }
    if let Some(rows) = &bundle.themes {
        let rows: Vec<Theme> = rows
            .iter()
            .filter(|t| t.kind != "builtin")
            .cloned()
            .collect();
        import_rows(
            conn,
            device_id,
            "themes",
            &rows,
            |t| t.name.clone(),
            apply,
            e,
        )?;
    }
    Ok(report)
}

/// Columns that are stamps or counters, not content.
const VOLATILE: &[&str] = &[
    "created_at",
    "updated_at",
    "rev",
    "device_id",
    "deleted_at",
    "last_fired_at",
    "message_count",
    "last_seen_at",
];

fn content<T: Serialize>(row: &T) -> Value {
    let mut v = serde_json::to_value(row).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        for k in VOLATILE {
            o.remove(*k);
        }
    }
    v
}

fn id_of<T: Serialize>(row: &T) -> String {
    serde_json::to_value(row)
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn import_rows<T>(
    conn: &Connection,
    device_id: &str,
    section: &str,
    rows: &[T],
    label: impl Fn(&T) -> String,
    apply: bool,
    entries: &mut BTreeMap<String, ImportOutcome>,
) -> Result<()>
where
    T: Syncable + Serialize + DeserializeOwned + Clone,
{
    for incoming in rows {
        let id = id_of(incoming);
        let mut key = format!("{section}/{}", label(incoming));
        if entries.contains_key(&key) {
            key = format!("{key} ({id})");
        }
        if id.is_empty() {
            entries.insert(key, ImportOutcome::Skipped("no id".into()));
            continue;
        }
        let existing = repo::get_by_id::<T>(conn, &id)?;
        let outcome = match &existing {
            None => ImportOutcome::New,
            Some(here) if here.deleted_at().is_some() => {
                ImportOutcome::Skipped("deleted on this device".into())
            }
            Some(here) if content(here) == content(incoming) => ImportOutcome::Unchanged,
            Some(here) if incoming.rev() >= here.rev() => ImportOutcome::Updated,
            Some(_) => ImportOutcome::Skipped("changed here since the export".into()),
        };
        let outcome = if apply {
            match (&outcome, existing) {
                (ImportOutcome::New, _) => {
                    let mut row = incoming.clone();
                    match repo::create(conn, device_id, &mut row) {
                        Ok(_) => outcome,
                        Err(e) => ImportOutcome::Skipped(format!("not written: {e}")),
                    }
                }
                (ImportOutcome::Updated, Some(here)) => {
                    // Keep our stamps, take the file's content; `save` bumps `rev`.
                    let mut merged = content(incoming);
                    let ours = serde_json::to_value(&here)?;
                    if let (Some(m), Some(o)) = (merged.as_object_mut(), ours.as_object()) {
                        for k in VOLATILE {
                            if let Some(v) = o.get(*k) {
                                m.insert((*k).to_string(), v.clone());
                            }
                        }
                    }
                    let mut row: T = serde_json::from_value(merged)?;
                    match repo::save(conn, device_id, &mut row) {
                        Ok(()) => outcome,
                        Err(e) => ImportOutcome::Skipped(format!("not written: {e}")),
                    }
                }
                _ => outcome,
            }
        } else {
            outcome
        };
        entries.insert(key, outcome);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, mode_core::db::Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = mode_core::db::open(&dir.path().join("mode.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn sections_are_checked() {
        assert!(check_sections(&["settings".into(), "nope".into()]).is_err());
        assert!(check_sections(&[]).is_err());
        assert_eq!(
            check_sections(&["themes".into(), "themes".into()]).unwrap(),
            ["themes"]
        );
    }

    #[test]
    fn a_bare_settings_bundle_imports_and_garbage_does_not() {
        let (_d, a) = db();
        let s = settings::export(&a, false).unwrap();
        let text = settings::bundle_to_json(&s).unwrap();
        let b = parse(&text).unwrap();
        assert_eq!(b.sections, ["settings"]);
        assert!(parse("{}").is_err());
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"kind":"mode-export","format":99,"exported_at":"t","app_version":"x","sections":[]}"#).is_err());
    }
}
