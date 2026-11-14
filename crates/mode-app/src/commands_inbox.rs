//! The Inbox and Calendar commands (03 §4, §6; 06 §7; 07 §2–§6;
//! `ui/src/state/contract-connectors.ts`): the list and the thread view,
//! triage with write-back (mark read, archive), snooze, the badge override,
//! bodies on demand, open in the source app, the keyword rules and Re-score
//! of Settings → Urgency, important people, the agenda, Join and Prepare notes,
//! the thread and event links (`event_links`, `item_link_add`,
//! `item_link_remove`), and the scorer counters for Diagnostics. Thin wrappers over
//! `mode_core::connectors::api` on the blocking pool; an `ApiError`
//! rejects as `{code, message}`.
//!
//! The overlay may also call `message_mark_read`, `thread_archive`,
//! `thread_snooze`, `open_in_source` and `event_join`. It has no opener
//! grant, so `open_in_source` and `event_join` take `open: true` to have
//! the core open the link (http, https, mailto and slack only).

use std::sync::Arc;

use mode_core::calendar::EventRow;
use mode_core::connectors::FetchedBody;
use mode_core::connectors::api::{
    self, AgendaArgs, ArchiveArgs, ContactArgs, IdArgs, InboxListArgs, ItemLinkArgs,
    KeywordRulesSetArgs, MarkReadArgs, NoteIdResult, OpenInSource, SetTierArgs, SnoozeArgs,
    SnoozeResult, ThreadGetArgs, UnsnoozeArgs, UrgencyDiagnostics,
};
use mode_core::connectors::service::PollReport;
use mode_core::connectors::store::TriageResult;
use mode_core::links::ItemLink;
use mode_core::urgency::Tier;
use mode_core::urgency::route::{InboxFilter, InboxListResult, ThreadDetail};
use mode_core::urgency::rules::KeywordRule;
use mode_core::urgency::snooze::Preset;
use mode_core::urgency::vips::{VipAdd, VipChange, VipRow};
use tauri::{AppHandle, State};

use crate::cmd_error::CmdResult;
use crate::commands_connectors::{open_external, with_connectors};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

// -------------------------------------------------------------------- Inbox

#[tauri::command(rename_all = "snake_case")]
pub async fn inbox_list(
    state: Core<'_>,
    filter: Option<InboxFilter>,
    cursor: Option<String>,
    limit: Option<i64>,
) -> CmdResult<InboxListResult> {
    let args = InboxListArgs {
        filter: filter.unwrap_or_default(),
        cursor,
        limit,
    };
    with_connectors(&state, move |c| api::inbox_list(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn thread_get(state: Core<'_>, thread_id: String) -> CmdResult<ThreadDetail> {
    with_connectors(&state, move |c| {
        api::thread_get(c, ThreadGetArgs { thread_id })
    })
    .await
}

/// Local first, queued write-back where the scope allows (06 §2.5); the
/// item's OS notification is withdrawn.
#[tauri::command(rename_all = "snake_case")]
pub async fn message_mark_read(
    state: Core<'_>,
    id: String,
    kind: Option<String>,
    read: Option<bool>,
) -> CmdResult<TriageResult> {
    let args = MarkReadArgs {
        id,
        kind,
        read: read.unwrap_or(true),
    };
    with_connectors(&state, move |c| api::message_mark_read(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn thread_archive(
    state: Core<'_>,
    id: String,
    kind: Option<String>,
    archived: Option<bool>,
) -> CmdResult<TriageResult> {
    let args = ArchiveArgs {
        id,
        kind,
        archived: archived.unwrap_or(true),
    };
    with_connectors(&state, move |c| api::thread_archive(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn thread_snooze(
    state: Core<'_>,
    id: String,
    kind: Option<String>,
    preset: Option<Preset>,
    until: Option<String>,
    wake_on_reply: Option<bool>,
) -> CmdResult<SnoozeResult> {
    let args = SnoozeArgs {
        id,
        kind,
        preset,
        until,
        wake_on_reply,
    };
    with_connectors(&state, move |c| api::thread_snooze(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn thread_unsnooze(
    state: Core<'_>,
    id: String,
    kind: Option<String>,
) -> CmdResult<usize> {
    with_connectors(&state, move |c| {
        api::thread_unsnooze(c, UnsnoozeArgs { id, kind })
    })
    .await
}

/// The source app's link for a message, thread or event; `open: true`
/// also opens it (the deep link, else the web fallback).
#[tauri::command(rename_all = "snake_case")]
pub async fn open_in_source(
    app: AppHandle,
    state: Core<'_>,
    id: String,
    open: Option<bool>,
) -> CmdResult<OpenInSource> {
    let link = with_connectors(&state, move |c| api::open_in_source(c, IdArgs { id })).await?;
    if open == Some(true) {
        let first = open_external(&app, &link.url);
        if first.is_err()
            && let Some(f) = &link.fallback_url
        {
            open_external(&app, f)?;
        } else {
            first?;
        }
    }
    Ok(link)
}

/// The badge menu override (07 §3.5).
#[tauri::command(rename_all = "snake_case")]
pub async fn message_set_tier(state: Core<'_>, id: String, tier: Tier) -> CmdResult<usize> {
    with_connectors(&state, move |c| {
        api::message_set_tier(c, SetTierArgs { id, tier })
    })
    .await
}

/// A body on demand (06 §2.1 `fetch_body`).
#[tauri::command(rename_all = "snake_case")]
pub async fn message_body(state: Core<'_>, id: String) -> CmdResult<FetchedBody> {
    with_connectors(&state, move |c| api::message_body(c, IdArgs { id })).await
}

/// Poll every source now (the toolbar refresh).
#[tauri::command(rename_all = "snake_case")]
pub async fn inbox_refresh(state: Core<'_>) -> CmdResult<PollReport> {
    with_connectors(&state, api::inbox_refresh).await
}

// ------------------------------------------------- Urgency settings

#[tauri::command(rename_all = "snake_case")]
pub async fn keyword_rules_list(state: Core<'_>) -> CmdResult<Vec<KeywordRule>> {
    with_connectors(&state, api::keyword_rules_list).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn keyword_rules_set(
    state: Core<'_>,
    rules: Vec<KeywordRule>,
) -> CmdResult<Vec<KeywordRule>> {
    with_connectors(&state, move |c| {
        api::keyword_rules_set(c, KeywordRulesSetArgs { rules })
    })
    .await
}

/// Re-score the last 24 h (07 §3.4.8).
#[tauri::command(rename_all = "snake_case")]
pub async fn urgency_rescore(state: Core<'_>) -> CmdResult<usize> {
    with_connectors(&state, api::urgency_rescore).await
}

/// Scorer counters and queue depths for Diagnostics (counts only, 07 §11.6).
#[tauri::command(rename_all = "snake_case")]
pub async fn urgency_diagnostics(state: Core<'_>) -> CmdResult<UrgencyDiagnostics> {
    with_connectors(&state, api::urgency_diagnostics).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vips_list(state: Core<'_>) -> CmdResult<Vec<VipRow>> {
    with_connectors(&state, api::vips_list).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn vip_add(
    state: Core<'_>,
    contact_id: Option<String>,
    message_id: Option<String>,
    email: Option<String>,
    slack_user: Option<String>,
    slack_team: Option<String>,
    display_name: Option<String>,
    vip_note: Option<String>,
) -> CmdResult<VipChange> {
    let args = VipAdd {
        contact_id,
        message_id,
        email,
        slack_user,
        slack_team,
        display_name,
        vip_note,
    };
    with_connectors(&state, move |c| api::vip_add(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn vip_remove(state: Core<'_>, contact_id: String) -> CmdResult<VipChange> {
    with_connectors(&state, move |c| {
        api::vip_remove(c, ContactArgs { contact_id })
    })
    .await
}

// ----------------------------------------------------------------- calendar

#[tauri::command(rename_all = "snake_case")]
pub async fn agenda_list(state: Core<'_>, from: String, to: String) -> CmdResult<Vec<EventRow>> {
    with_connectors(&state, move |c| {
        api::agenda_list(c, AgendaArgs { from, to })
    })
    .await
}

/// The event's join URL; `open: true` also opens it.
#[tauri::command(rename_all = "snake_case")]
pub async fn event_join(
    app: AppHandle,
    state: Core<'_>,
    id: String,
    open: Option<bool>,
) -> CmdResult<String> {
    let url = with_connectors(&state, move |c| api::event_join(c, IdArgs { id })).await?;
    if open == Some(true) {
        open_external(&app, &url)?;
    }
    Ok(url)
}

/// The event's prepare note (03 §6.7): created through the notes service
/// unless it exists.
#[tauri::command(rename_all = "snake_case")]
pub async fn event_prepare_notes(state: Core<'_>, id: String) -> CmdResult<NoteIdResult> {
    let notes = state.notes()?;
    with_connectors(&state, move |c| {
        api::event_prepare_notes(c, &notes, IdArgs { id })
    })
    .await
}

// -------------------------------------------------------------------- links

/// The event panel's linked sessions, notes and messages (03 §6.6).
#[tauri::command(rename_all = "snake_case")]
pub async fn event_links(state: Core<'_>, id: String) -> CmdResult<Vec<ItemLink>> {
    with_connectors(&state, move |c| api::event_links(c, IdArgs { id })).await
}

/// Link a thread or an event to a session, note or message by hand.
#[tauri::command(rename_all = "snake_case")]
pub async fn item_link_add(
    state: Core<'_>,
    kind: String,
    id: String,
    to_kind: String,
    to_id: String,
) -> CmdResult<Vec<ItemLink>> {
    let args = ItemLinkArgs {
        kind,
        id,
        to_kind,
        to_id,
    };
    with_connectors(&state, move |c| api::item_link_add(c, args)).await
}

/// Remove a link (or a suggestion, remembered as dismissed).
#[tauri::command(rename_all = "snake_case")]
pub async fn item_link_remove(
    state: Core<'_>,
    kind: String,
    id: String,
    to_kind: String,
    to_id: String,
) -> CmdResult<Vec<ItemLink>> {
    let args = ItemLinkArgs {
        kind,
        id,
        to_kind,
        to_id,
    };
    with_connectors(&state, move |c| api::item_link_remove(c, args)).await
}
