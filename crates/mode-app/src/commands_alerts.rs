//! The alert commands (07 §8; `ui/src/state/contract-connectors.ts`):
//! the recent `alert_log` rows (Diagnostics, Settings → Alerts), withdrawing
//! an item's OS notifications, the Settings **Test** button, the rule list
//! and its edits (`alert_rules_list`, `alert_rule_set`: enable, channels,
//! sound, cooldown, Focus / quiet hours, filters, Reset), custom rules from
//! the event picker (`alert_rule_add`, `alert_rule_delete`), and `quiet_set`:
//! the status item's "Pause alerts for 1 h" (`pause_minutes`, also from the
//! overlay), quiet hours, and OS facts a webview may report. Every quiet
//! write pushes `quiet.changed`. Thin wrappers over
//! `mode_core::connectors::api` on the blocking pool.
//!
//! `os_permission` is accepted from the webview but the app never sets it
//! itself: the notification plugin answers "granted" on every desktop OS
//! whatever the system settings say, so a denied permission is not
//! detectable yet. It stays `unknown` (post and let the OS decide), and 07
//! §8.4's `os_permission` suppression is reachable only through this
//! argument.

use std::sync::Arc;

use mode_core::connectors::api::{
    self, AlertDismissArgs, AlertLogRow, AlertRuleAddArgs, AlertRuleDeleted, AlertRuleRow,
    AlertRuleSetArgs, AlertTestArgs, AlertsRecentArgs, IdArgs, QuietSetArgs,
};
use mode_core::urgency::alerts::{AlertOutcome, Filters, OsPermission, QuietChanged, QuietHours};
use tauri::State;

use crate::cmd_error::CmdResult;
use crate::commands_connectors::with_connectors;
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

#[tauri::command(rename_all = "snake_case")]
pub async fn alerts_recent(state: Core<'_>, limit: Option<i64>) -> CmdResult<Vec<AlertLogRow>> {
    with_connectors(&state, move |c| {
        api::alerts_recent(c, AlertsRecentArgs { limit })
    })
    .await
}

/// Withdraw the OS notification(s) of an item (`item_key`, or
/// `target_kind` + `target_id`); returns the identifiers withdrawn.
#[tauri::command(rename_all = "snake_case")]
pub async fn alert_dismiss(
    state: Core<'_>,
    item_key: Option<String>,
    target_kind: Option<String>,
    target_id: Option<String>,
) -> CmdResult<Vec<String>> {
    let args = AlertDismissArgs {
        item_key,
        target_kind,
        target_id,
    };
    with_connectors(&state, move |c| api::alert_dismiss(c, args)).await
}

/// Settings → Alerts → Test (07 §8.2): a sample event through the pipeline,
/// delivered to the rule's channels; nothing is written.
#[tauri::command(rename_all = "snake_case")]
pub async fn alert_rule_test(state: Core<'_>, rule_id: String) -> CmdResult<AlertOutcome> {
    with_connectors(&state, move |c| {
        api::alert_rule_test(c, AlertTestArgs { rule_id })
    })
    .await
}

/// Settings → Alerts: the rules by position (the premade ones upserted by
/// name at start, 07 §8.2).
#[tauri::command(rename_all = "snake_case")]
pub async fn alert_rules_list(state: Core<'_>) -> CmdResult<Vec<AlertRuleRow>> {
    with_connectors(&state, api::alert_rules_list).await
}

/// Edit one rule; every field optional, `reset` restores a premade rule.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn alert_rule_set(
    state: Core<'_>,
    id: String,
    enabled: Option<bool>,
    name: Option<String>,
    channels: Option<Vec<String>>,
    sound: Option<String>,
    cooldown_seconds: Option<i64>,
    respect_dnd: Option<bool>,
    quiet_hours: Option<bool>,
    filters: Option<Filters>,
    position: Option<i64>,
    reset: Option<bool>,
) -> CmdResult<AlertRuleRow> {
    let args = AlertRuleSetArgs {
        id,
        enabled,
        name,
        channels,
        sound,
        cooldown_seconds,
        respect_dnd,
        quiet_hours,
        filters,
        position,
        reset,
    };
    with_connectors(&state, move |c| api::alert_rule_set(c, args)).await
}

/// A custom rule from the Alerts page's event picker (03 §8.11), enabled
/// and placed last; refusals are `invalid`.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn alert_rule_add(
    state: Core<'_>,
    name: String,
    event: String,
    filters: Filters,
    channels: Vec<String>,
    sound: Option<String>,
    respect_dnd: Option<bool>,
    quiet_hours: Option<bool>,
    cooldown_seconds: Option<i64>,
) -> CmdResult<AlertRuleRow> {
    let args = AlertRuleAddArgs {
        name,
        event,
        filters,
        channels,
        sound,
        respect_dnd,
        quiet_hours,
        cooldown_seconds,
    };
    with_connectors(&state, move |c| api::alert_rule_add(c, args)).await
}

/// Delete a custom rule (`{deleted}`); a premade one refuses with `premade`.
#[tauri::command(rename_all = "snake_case")]
pub async fn alert_rule_delete(state: Core<'_>, id: String) -> CmdResult<AlertRuleDeleted> {
    with_connectors(&state, move |c| api::alert_rule_delete(c, IdArgs { id })).await
}

/// The manual pause (`pause_minutes`, 0 resumes), quiet hours, and the OS
/// facts (presentation, Focus, lock, mute, permission); returns the merged
/// `quiet.changed` payload, which is also pushed.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn quiet_set(
    state: Core<'_>,
    pause_minutes: Option<i64>,
    quiet_hours: Option<QuietHours>,
    presentation: Option<bool>,
    focus: Option<bool>,
    screen_locked: Option<bool>,
    output_muted: Option<bool>,
    os_permission: Option<OsPermission>,
) -> CmdResult<QuietChanged> {
    let args = QuietSetArgs {
        pause_minutes,
        quiet_hours,
        presentation,
        focus: focus.map(Some),
        screen_locked,
        output_muted,
        os_permission,
    };
    with_connectors(&state, move |c| api::quiet_set(c, args)).await
}
