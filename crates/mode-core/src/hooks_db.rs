//! The `hook_installs` rows (09 §5.5) behind the Claude Code card: one row
//! per settings file Mode edited, kept in step by the install commands and
//! re-verified by `inspect`.

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Result;
use crate::hooks_install::{self, Applied, InstallStatus, Status, Target};
use crate::repo::{self, HookInstall};

fn status_word(s: InstallStatus) -> &'static str {
    match s {
        InstallStatus::Installed => "installed",
        InstallStatus::Stale => "stale",
        InstallStatus::Missing => "missing",
        InstallStatus::Conflict => "conflict",
        InstallStatus::Removed | InstallStatus::NotInstalled => "removed",
    }
}

/// Whether Mode ever installed into this settings file.
pub fn previously_installed(conn: &Connection, settings_path: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM hook_installs WHERE settings_path = ? AND removed_at IS NULL",
            [settings_path],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

/// Record an install (or update) that was just applied.
pub fn record_install(
    conn: &Connection,
    target: &Target,
    applied: &Applied,
    events: &[String],
    now: &str,
) -> Result<()> {
    let settings_path = target.settings_path.to_string_lossy().into_owned();
    let existing = repo::query::<HookInstall>(conn, "settings_path = ?", &[&settings_path])?
        .into_iter()
        .next();
    let mut row = existing.unwrap_or_else(|| HookInstall {
        id: repo::new_id(),
        scope: "user".into(),
        settings_path: settings_path.clone(),
        installed_at: now.to_string(),
        created_at: now.to_string(),
        ..Default::default()
    });
    row.relay_path = target.relay_path.to_string_lossy().into_owned();
    row.relay_version = hooks_install::RELAY_VERSION.to_string();
    row.events_json = serde_json::to_string(events)?;
    row.block_hash = applied.block_hash.clone();
    if let Some(b) = &applied.backup_path {
        row.backup_path = Some(b.to_string_lossy().into_owned());
    }
    row.status = "installed".into();
    row.status_detail = None;
    row.installed_at = now.to_string();
    row.verified_at = Some(now.to_string());
    row.removed_at = None;
    row.updated_at = now.to_string();
    repo::upsert(conn, &row, &["settings_path"])
}

/// Fold a re-verification into the row (status, detail, `verified_at`).
pub fn record_status(conn: &Connection, status: &Status, now: &str) -> Result<bool> {
    let settings_path = status.path.to_string_lossy().into_owned();
    let removed_at: Option<&str> = matches!(
        status.status,
        InstallStatus::Removed | InstallStatus::NotInstalled
    )
    .then_some(now);
    Ok(conn.execute(
        "UPDATE hook_installs SET status = ?, status_detail = ?, verified_at = ?,
                removed_at = COALESCE(removed_at, ?), updated_at = ?
          WHERE settings_path = ?",
        params![
            status_word(status.status),
            status.detail,
            now,
            removed_at,
            now,
            settings_path
        ],
    )? > 0)
}

/// Every `hook_installs` row, newest change first (Diagnostics → Hooks).
pub fn rows(conn: &Connection) -> Result<Vec<HookInstall>> {
    repo::query::<HookInstall>(conn, "1 ORDER BY updated_at DESC", &[])
}

/// The row for one settings file, when Mode ever edited it.
pub fn row_for(conn: &Connection, settings_path: &str) -> Result<Option<HookInstall>> {
    Ok(
        repo::query::<HookInstall>(conn, "settings_path = ?", &[&settings_path])?
            .into_iter()
            .next(),
    )
}

/// Mark the row removed after an uninstall.
pub fn record_uninstall(conn: &Connection, settings_path: &str, now: &str) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE hook_installs SET status = 'removed', removed_at = ?, updated_at = ? WHERE settings_path = ?",
        params![now, now, settings_path],
    )? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn install_rows_are_upserted_by_settings_path() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open(&dir.path().join("mode.db")).unwrap();
        let target = Target::native(
            dir.path().join("settings.json"),
            dir.path().join("mode-hook"),
        );
        let applied = Applied {
            settings_path: target.settings_path.clone(),
            backup_path: Some(PathBuf::from("/b")),
            block_hash: "abc".into(),
            bytes_written: 10,
            written: true,
        };
        let t = target.clone();
        let a = applied.clone();
        db.write(move |c| {
            let sp = t.settings_path.to_string_lossy().into_owned();
            assert!(!previously_installed(c, &sp)?);
            record_install(
                c,
                &t,
                &a,
                &["SessionStart".into()],
                "2026-10-21T12:00:00.000Z",
            )?;
            assert!(previously_installed(c, &sp)?);
            let mut a2 = a.clone();
            a2.block_hash = "def".into();
            record_install(
                c,
                &t,
                &a2,
                &["SessionStart".into(), "Stop".into()],
                "2026-10-21T12:01:00.000Z",
            )?;
            let rows = repo::query::<HookInstall>(c, "1", &[])?;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].block_hash, "def");
            assert!(rows[0].events_json.contains("Stop"));
            assert!(record_uninstall(c, &sp, "2026-10-21T12:02:00.000Z")?);
            assert!(!previously_installed(c, &sp)?);
            assert_eq!(super::rows(c)?.len(), 1);
            let row = row_for(c, &sp)?.expect("the row");
            assert_eq!(row.status, "removed");
            assert_eq!(row.backup_path.as_deref(), Some("/b"));
            assert!(row_for(c, "/elsewhere")?.is_none());
            Ok(())
        })
        .unwrap();
    }
}
