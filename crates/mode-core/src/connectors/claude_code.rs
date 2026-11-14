//! Claude Code as a connector (06 §9): not a network connector and not a
//! [`ConnectorAdapter`](super::ConnectorAdapter) — it implements the agent
//! adapter of 05. It shares only the Integrations card: the status line
//! from `hook_installs.status` (`installed`, `stale`, `missing`,
//! `conflict`, `removed`), whether the `claude` binary was found (the app
//! probes it), and the session counts. No auth, no secrets, nothing leaves
//! the machine.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::Result;

/// The Claude Code card (03 §7C.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeCodeCard {
    /// `connected` | `not_connected` | `needs_attention`.
    pub state: String,
    pub glyph: String,
    pub line: String,
    /// The latest install's status, or `none`.
    pub hooks: String,
    pub sessions_tracked: i64,
    pub sessions_today: i64,
}

/// Build the card; `binary_found` comes from the app's `claude` probe.
pub fn card(conn: &Connection, binary_found: bool, today_start: &str) -> Result<ClaudeCodeCard> {
    let hooks: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status, status_detail FROM hook_installs ORDER BY updated_at DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let tracked: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE parent_session_id IS NULL AND tracking <> 'untracked'",
        [],
        |r| r.get(0),
    )?;
    let today: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE parent_session_id IS NULL AND started_at >= ?1",
        [today_start],
        |r| r.get(0),
    )?;
    let status = hooks.as_ref().map(|h| h.0.as_str()).unwrap_or("none");
    let (state, glyph, line) = match status {
        "installed" => (
            "connected",
            "◉",
            format!(
                "hooks installed · {tracked} session{}",
                if tracked == 1 { "" } else { "s" }
            ),
        ),
        "stale" | "missing" | "conflict" => (
            "needs_attention",
            "✕",
            hooks
                .as_ref()
                .and_then(|h| h.1.clone())
                .unwrap_or_else(|| format!("hooks {status} — reinstall")),
        ),
        _ if !binary_found => ("not_connected", "○", "Claude Code not found".to_string()),
        _ => ("not_connected", "○", "hooks not installed".to_string()),
    };
    Ok(ClaudeCodeCard {
        state: state.into(),
        glyph: glyph.into(),
        line,
        hooks: status.into(),
        sessions_tracked: tracked,
        sessions_today: today,
    })
}
