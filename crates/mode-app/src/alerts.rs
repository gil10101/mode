//! The alert sink (07 §8.3, §8.6) and the alert events the app feeds the
//! connectors service (07 §8.1):
//! - [`plan`] maps an [`AlertCommand`] to the steps the app takes: the
//!   overlay stream `alert:fired {event, item_key, sound}` (the overlay plays
//!   the cue once), one OS notification per item through
//!   `tauri-plugin-notification` (numeric id derived from the item key, the
//!   thread / session as its group, the action buttons of its category, no
//!   OS sound unless the core asked for one), and the withdrawal of delivered
//!   notifications;
//! - [`Notifier`] runs those steps and answers clicks and actions: a click
//!   opens the item's route in the app window (`app:open`), **Allow** /
//!   **Deny** answer the session's card, **Mark read**, **Snooze** and
//!   **Join** run the matching command without opening the window;
//! - [`SessionWatch`] turns session state transitions seen on every
//!   `sessions.changed` into `session.waiting` / `session.failed` /
//!   `session.finished` events and withdraws a session's notifications when
//!   it leaves those states (answered, acknowledged, dismissed);
//! - [`note_due_task`] runs the `note.due` check every minute.
//!
//! Desktop limits of the notification plugin (2.7): grouping is ignored on
//! macOS and Windows, removing a delivered notification works on Linux only,
//! and actions are reported only while a handler is registered (it is, from
//! setup).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use mode_core::connectors::Connectors;
use mode_core::connectors::api;
use mode_core::connectors::service::AlertCommand;
use mode_core::db::{Db, parse_timestamp};
use mode_core::push::Event;
use mode_core::repo::{self, Session};
use mode_core::sessions::{Decision, Pending};
use mode_core::urgency::alerts::{
    self, AlertEvent, AlertOutcome, OsNotification, Route, SessionFailed, SessionFinished,
    SessionWaiting,
};
use mode_core::urgency::with_tz;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::app_window::{self, AppOpen};
use crate::state::CoreState;

/// The `note.due` check cadence (07 §8.1: start of day or a fixed time).
pub const NOTE_DUE_EVERY: Duration = Duration::from_secs(60);
/// Delivered notifications remembered for clicks and actions.
pub const DELIVERED_KEPT: usize = 500;
/// The `extra` key carrying the item key on a posted notification.
pub const EXTRA_IDENTIFIER: &str = "mode_identifier";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ------------------------------------------------------- mapping (pure)

/// `alert:fired` (a stream to the overlay): the rule's cue, played once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlertFired {
    pub event: String,
    pub item_key: String,
    pub sound: Option<String>,
}

/// What a click or an action on a posted notification needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    pub identifier: String,
    pub event: String,
    pub route: Route,
}

/// One OS notification as the plugin posts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    /// The plugin's numeric id, stable per item key (a newer alert for the
    /// same item reuses it).
    pub id: i32,
    /// The item key (§7.1); travels as `extra.mode_identifier`.
    pub identifier: String,
    /// `threadIdentifier` / toast group: the conversation or session.
    pub group: String,
    pub title: String,
    /// The subtitle line, then the body lines.
    pub body: String,
    /// The action type (`MODE_APPROVAL`, `MODE_MESSAGE`, `MODE_EVENT`).
    pub action_type: Option<String>,
    /// `(id, label)`: `allow` / `deny`, `mark_read` / `snooze_tomorrow`, `join`.
    pub actions: Vec<(String, String)>,
    /// `None`: silent (the overlay plays the rule's cue, §8.6 "Sound").
    pub sound: Option<&'static str>,
    pub delivered: Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkStep {
    Overlay(AlertFired),
    Post(Box<Post>),
    Remove {
        identifiers: Vec<String>,
        ids: Vec<i32>,
    },
}

/// The plugin's notification id for an item key: FNV-1a, positive, never 0.
pub fn notification_id(identifier: &str) -> i32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in identifier.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    match (h & 0x7fff_ffff) as i32 {
        0 => 1,
        n => n,
    }
}

fn post_for(out: &AlertOutcome, n: &OsNotification) -> Post {
    let body = std::iter::once(n.subtitle.as_str())
        .chain(n.body.iter().map(String::as_str))
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let actions: Vec<(String, String)> = n
        .actions
        .iter()
        .map(|a| (a.id.clone(), a.label.clone()))
        .collect();
    Post {
        id: notification_id(&n.identifier),
        identifier: n.identifier.clone(),
        group: n.thread_identifier.clone(),
        title: n.title.clone(),
        body,
        action_type: if actions.is_empty() {
            None
        } else {
            n.category.clone()
        },
        actions,
        sound: (!n.silent).then_some("default"),
        delivered: Delivered {
            identifier: n.identifier.clone(),
            event: out.event.clone(),
            route: n.route.clone(),
        },
    }
}

/// What the app does for one alert command.
pub fn plan(cmd: &AlertCommand) -> Vec<SinkStep> {
    match cmd {
        AlertCommand::Fired(out) => {
            let mut steps = vec![SinkStep::Overlay(AlertFired {
                event: out.event.clone(),
                item_key: out.item_key.clone(),
                sound: out.sound.clone(),
            })];
            if let Some(n) = &out.notification {
                steps.push(SinkStep::Post(Box::new(post_for(out, n))));
            }
            steps
        }
        AlertCommand::Withdraw { identifiers } => vec![SinkStep::Remove {
            identifiers: identifiers.clone(),
            ids: identifiers.iter().map(|i| notification_id(i)).collect(),
        }],
    }
}

/// What a click or an action button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionPlan {
    /// Open the item in the app window (`tap`, or an action without a target).
    Open(AppOpen),
    /// Answer the session's open permission card.
    Decide { session_id: String, allow: bool },
    /// `message_mark_read` on a message (`kind = message`) or a thread.
    MarkRead { id: String, kind: &'static str },
    /// `thread_snooze { preset: tomorrow }`.
    SnoozeTomorrow { id: String, kind: &'static str },
    /// `event_join`, then open the URL.
    Join { event_id: String },
}

/// The app window request for a notification's route (03 §2.5).
pub fn open_for(route: &Route) -> AppOpen {
    let view = if app_window::VIEWS.contains(&route.view.as_str()) {
        route.view.as_str()
    } else {
        "inbox"
    };
    AppOpen::new(view, route.id.clone(), None, None)
}

/// Map a reported action (`tap` for a click) on a delivered notification.
pub fn action_plan(action: &str, d: &Delivered) -> ActionPlan {
    let r = &d.route;
    // The inbox route carries the thread as `id` and the message as `focus`.
    let message_target = || match (&r.focus, &r.id) {
        (Some(m), _) => Some((m.clone(), "message")),
        (None, Some(t)) => Some((t.clone(), "thread")),
        _ => None,
    };
    let thread_target = || match (&r.id, &r.focus) {
        (Some(t), _) => Some((t.clone(), "thread")),
        (None, Some(m)) => Some((m.clone(), "message")),
        _ => None,
    };
    let planned = match action {
        "allow" | "deny" if r.view == "sessions" => r.id.clone().map(|s| ActionPlan::Decide {
            session_id: s,
            allow: action == "allow",
        }),
        "mark_read" if r.view == "inbox" => {
            message_target().map(|(id, kind)| ActionPlan::MarkRead { id, kind })
        }
        "snooze_tomorrow" if r.view == "inbox" => {
            thread_target().map(|(id, kind)| ActionPlan::SnoozeTomorrow { id, kind })
        }
        "join" if r.view == "calendar" => {
            r.id.clone().map(|event_id| ActionPlan::Join { event_id })
        }
        _ => None,
    };
    planned.unwrap_or_else(|| ActionPlan::Open(open_for(r)))
}

// ------------------------------------------------------------ the notifier

/// Runs the sink steps on the app and answers notification actions.
pub struct Notifier {
    app: AppHandle,
    delivered: Mutex<(HashMap<i32, Delivered>, VecDeque<i32>)>,
}

impl std::fmt::Debug for Notifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notifier").finish_non_exhaustive()
    }
}

impl Notifier {
    pub fn new(app: AppHandle) -> Arc<Notifier> {
        Arc::new(Notifier {
            app,
            delivered: Mutex::new((HashMap::new(), VecDeque::new())),
        })
    }

    /// The connectors service's alert sink.
    pub fn handle(&self, cmd: &AlertCommand) {
        for step in plan(cmd) {
            match step {
                SinkStep::Overlay(p) => {
                    if let Err(e) =
                        self.app
                            .emit_to(crate::OVERLAY_WINDOW, Event::AlertFired.wire_name(), &p)
                    {
                        tracing::warn!(target: "mode::alerts", error = %e, "emit alert.fired failed");
                    }
                }
                SinkStep::Post(p) => self.post(*p),
                SinkStep::Remove { identifiers, ids } => {
                    {
                        let mut d = lock(&self.delivered);
                        for id in &ids {
                            d.0.remove(id);
                        }
                    }
                    use tauri_plugin_notification::NotificationExt;
                    if let Err(e) = self.app.notification().remove_active(ids) {
                        tracing::debug!(target: "mode::alerts", error = %e, "remove notifications");
                    }
                    tracing::info!(target: "mode::alerts", n = identifiers.len(), "notifications withdrawn");
                }
            }
        }
    }

    fn remember(&self, id: i32, d: Delivered) {
        let mut g = lock(&self.delivered);
        if g.0.insert(id, d).is_none() {
            g.1.push_back(id);
        }
        while g.1.len() > DELIVERED_KEPT {
            if let Some(old) = g.1.pop_front() {
                g.0.remove(&old);
            }
        }
    }

    fn lookup(&self, id: Option<i32>, identifier: Option<&str>) -> Option<Delivered> {
        let g = lock(&self.delivered);
        id.and_then(|i| g.0.get(&i).cloned()).or_else(|| {
            let ident = identifier?;
            g.0.get(&notification_id(ident)).cloned()
        })
    }

    /// Post one notification. Title and body never reach the log.
    fn post(&self, p: Post) {
        use tauri_plugin_notification::{Action, ActionType, NotificationExt};
        let n = self.app.notification();
        if let Some(t) = &p.action_type {
            let actions = p
                .actions
                .iter()
                .map(|(id, label)| Action::builder(id, label).build())
                .collect();
            if let Err(e) =
                n.register_action_types(vec![ActionType::builder(t).actions(actions).build()])
            {
                tracing::debug!(target: "mode::alerts", error = %e, "action types");
            }
        }
        let mut b = n
            .builder()
            .id(p.id)
            .title(&p.title)
            .body(&p.body)
            .group(&p.group)
            .extra(EXTRA_IDENTIFIER, &p.identifier);
        if let Some(t) = &p.action_type {
            b = b.action_type_id(t);
        }
        if let Some(s) = p.sound {
            b = b.sound(s);
        }
        let event = p.delivered.event.clone();
        self.remember(p.id, p.delivered);
        match b.show() {
            Ok(()) => tracing::info!(target: "mode::alerts", event = %event, "notification posted"),
            Err(e) => {
                tracing::warn!(target: "mode::alerts", event = %event, error = %e, "notification failed")
            }
        }
    }

    /// Answer clicks and action buttons for the life of the app.
    pub fn listen_actions(self: &Arc<Self>, core: Arc<CoreState>) {
        use tauri_plugin_notification::NotificationExt;
        let me = Arc::clone(self);
        let r = self.app.notification().on_action(move |performed| {
            let n = performed.notification();
            let ident = n.and_then(|n| n.extra().get(EXTRA_IDENTIFIER).and_then(|v| v.as_str()));
            let Some(d) = me.lookup(n.map(|n| n.id()), ident) else {
                tracing::debug!(target: "mode::alerts", action = performed.action_id(), "action on an unknown notification");
                return;
            };
            let plan = action_plan(performed.action_id(), &d);
            tracing::info!(target: "mode::alerts", action = performed.action_id(), event = %d.event, "notification action");
            let (app, core) = (me.app.clone(), Arc::clone(&core));
            tauri::async_runtime::spawn(run_action(app, core, plan));
        });
        if let Err(e) = r {
            tracing::warn!(target: "mode::alerts", error = %e, "notification actions unavailable");
        }
    }
}

/// Carry out a click or an action.
pub async fn run_action(app: AppHandle, core: Arc<CoreState>, plan: ActionPlan) {
    let result: Result<(), String> = match plan {
        ActionPlan::Open(req) => {
            app_window::open(&app, req);
            Ok(())
        }
        ActionPlan::Decide { session_id, allow } => {
            let engine = Arc::clone(&core.engine);
            let decision = Decision::Permission {
                behavior: if allow {
                    mode_wire::Behavior::Allow
                } else {
                    mode_wire::Behavior::Deny
                },
                always: false,
                message: None,
            };
            let r =
                tauri::async_runtime::spawn_blocking(move || engine.decide(&session_id, decision))
                    .await;
            core.publish([Event::Sessions, Event::Pill]);
            match r {
                Ok(Ok(outcome)) => {
                    tracing::info!(target: "mode::alerts", outcome = ?outcome, "decided from a notification");
                    Ok(())
                }
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(e.to_string()),
            }
        }
        ActionPlan::MarkRead { id, kind } => {
            with(&core, move |c| {
                api::message_mark_read(
                    c,
                    api::MarkReadArgs {
                        id,
                        kind: Some(kind.into()),
                        read: true,
                    },
                )
                .map(drop)
            })
            .await
        }
        ActionPlan::SnoozeTomorrow { id, kind } => {
            with(&core, move |c| {
                api::thread_snooze(
                    c,
                    api::SnoozeArgs {
                        id,
                        kind: Some(kind.into()),
                        preset: Some(mode_core::urgency::snooze::Preset::Tomorrow),
                        until: None,
                        wake_on_reply: None,
                    },
                )
                .map(drop)
            })
            .await
        }
        ActionPlan::Join { event_id } => {
            let url = {
                let c = Arc::clone(&core);
                tauri::async_runtime::spawn_blocking(move || {
                    let conn = c.connectors().map_err(|e| e.message)?;
                    api::event_join(&conn, api::IdArgs { id: event_id }).map_err(|e| e.message)
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
            };
            url.and_then(|u| {
                crate::commands_connectors::open_external(&app, &u).map_err(|e| e.message)
            })
        }
    };
    if let Err(e) = result {
        tracing::warn!(target: "mode::alerts", error = %e, "notification action failed");
    }
}

async fn with<F>(core: &Arc<CoreState>, f: F) -> Result<(), String>
where
    F: FnOnce(&Connectors) -> api::ApiResult<()> + Send + 'static,
{
    let c = core.connectors().map_err(|e| e.message)?;
    tauri::async_runtime::spawn_blocking(move || f(&c))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.message)
}

// ------------------------------------------------------- session alerts

/// What a session's alert identity depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    state: String,
    state_since: String,
    /// The open card (its `tool_use_id`, else when it arrived).
    card: Option<String>,
}

impl Seen {
    fn of(r: &Session) -> Seen {
        Seen {
            state: r.state.clone(),
            state_since: r.state_since.clone(),
            card: r
                .pending_json
                .as_deref()
                .and_then(Pending::from_json)
                .map(|p| p.tool_use_id.unwrap_or(p.received_at)),
        }
    }
}

/// A session alert to raise, or a session whose notifications to withdraw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSignal {
    Alert(Box<AlertEvent>),
    Withdraw(String),
}

/// Session state transitions → alert events (07 §8.1 `session.waiting`,
/// `session.failed`, `session.finished`). [`SessionWatch::prime`] at launch
/// records what exists without alerting; every `sessions.changed` then
/// calls [`SessionWatch::observe`].
#[derive(Debug, Default)]
pub struct SessionWatch {
    seen: Mutex<Option<HashMap<String, Seen>>>,
}

fn alertable(state: &str) -> bool {
    matches!(state, "waiting" | "failed" | "unread")
}

/// The rows the watch raises alerts for: top-level, not dismissed, and not
/// of an agent whose notifications are off (03 §7E.5).
fn top_level(db: &Db) -> mode_core::db::Result<Vec<Session>> {
    db.read(|c| {
        repo::query::<Session>(
            c,
            "parent_session_id IS NULL AND dismissed_at IS NULL
               AND (profile_id IS NULL OR profile_id NOT IN (SELECT id FROM agent_profiles WHERE notify = 0))",
            &[],
        )
    })
}

fn waiting_event(r: &Session) -> Option<AlertEvent> {
    let p = r.pending_json.as_deref().and_then(Pending::from_json);
    let kind = r
        .waiting_kind
        .clone()
        .or_else(|| p.as_ref().map(|p| p.kind.as_str().to_string()))?;
    Some(AlertEvent::SessionWaiting(SessionWaiting {
        session_id: r.id.clone(),
        claude_session_id: r.claude_session_id.clone(),
        display_name: r.display_name.clone(),
        repo_root: r.repo_root.clone(),
        waiting_kind: kind,
        tool_name: p.as_ref().and_then(|p| p.tool_name.clone()),
        tool_use_id: p.as_ref().and_then(|p| p.tool_use_id.clone()),
        summary: p.as_ref().map(|p| p.summary.clone()),
        state_since: r.state_since.clone(),
    }))
}

fn failed_event(r: &Session) -> AlertEvent {
    AlertEvent::SessionFailed(SessionFailed {
        session_id: r.id.clone(),
        claude_session_id: r.claude_session_id.clone(),
        display_name: r.display_name.clone(),
        repo_root: r.repo_root.clone(),
        reason: r.state_reason.clone().or_else(|| r.end_reason.clone()),
        ended_at: r.ended_at.clone().unwrap_or_else(|| r.state_since.clone()),
    })
}

fn finished_event(r: &Session) -> AlertEvent {
    let ended = r.ended_at.clone().unwrap_or_else(|| r.state_since.clone());
    let duration = match (parse_timestamp(&r.started_at), parse_timestamp(&ended)) {
        (Some(s), Some(e)) => (e - s).whole_seconds().max(0),
        _ => 0,
    };
    AlertEvent::SessionFinished(SessionFinished {
        session_id: r.id.clone(),
        claude_session_id: r.claude_session_id.clone(),
        display_name: r.display_name.clone(),
        repo_root: r.repo_root.clone(),
        lines_added: r.lines_added,
        lines_removed: r.lines_removed,
        files_changed: r.files_changed,
        duration_seconds: duration,
        ended_at: ended,
        last_assistant_preview: r.last_assistant_preview.clone(),
    })
}

fn transition(before: Option<&Seen>, r: &Session) -> Vec<SessionSignal> {
    let mut out = Vec::new();
    if before.is_some_and(|b| alertable(&b.state)) {
        // Answered, resumed, acknowledged, or a new card: the old ones go.
        out.push(SessionSignal::Withdraw(r.id.clone()));
    }
    let was = before.map(|b| b.state.as_str());
    match r.state.as_str() {
        "waiting" => out.extend(waiting_event(r).map(|e| SessionSignal::Alert(Box::new(e)))),
        "failed" if was != Some("failed") => {
            out.push(SessionSignal::Alert(Box::new(failed_event(r))))
        }
        "unread" if !matches!(was, Some("unread" | "done")) => {
            out.push(SessionSignal::Alert(Box::new(finished_event(r))))
        }
        _ => {}
    }
    out
}

impl SessionWatch {
    /// Remember the sessions as they are (launch): no alert for them.
    pub fn prime(&self, db: &Db) {
        match top_level(db) {
            Ok(rows) => {
                *lock(&self.seen) = Some(rows.iter().map(|r| (r.id.clone(), Seen::of(r))).collect())
            }
            Err(e) => tracing::warn!(target: "mode::alerts", error = %e, "session watch prime"),
        }
    }

    /// The signals since the last call (the first call only primes).
    pub fn observe(&self, db: &Db) -> Vec<SessionSignal> {
        let rows = match top_level(db) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(target: "mode::alerts", error = %e, "session watch");
                return Vec::new();
            }
        };
        let mut guard = lock(&self.seen);
        let next: HashMap<String, Seen> =
            rows.iter().map(|r| (r.id.clone(), Seen::of(r))).collect();
        let Some(prev) = guard.as_ref() else {
            *guard = Some(next);
            return Vec::new();
        };
        let mut out = Vec::new();
        for r in &rows {
            let before = prev.get(&r.id);
            if before != next.get(&r.id) {
                out.extend(transition(before, r));
            }
        }
        for (id, s) in prev {
            if !next.contains_key(id) && alertable(&s.state) {
                out.push(SessionSignal::Withdraw(id.clone()));
            }
        }
        *guard = Some(next);
        out
    }
}

/// Hand session signals to the connectors service (blocking: the alert
/// pipeline writes `alert_log`).
pub fn dispatch_session_signals(c: &Connectors, signals: Vec<SessionSignal>) {
    for s in signals {
        match s {
            SessionSignal::Withdraw(id) => c.withdraw("session", &id),
            SessionSignal::Alert(e) => c.alert(*e),
        }
    }
}

/// After a `sessions.changed` payload: look for transitions and raise their
/// alerts off the publisher's path.
pub fn after_sessions_push(core: &Arc<CoreState>) {
    let Some(c) = core.connectors.get().cloned() else {
        return;
    };
    let signals = core.session_watch.observe(&core.db);
    if !signals.is_empty() {
        tauri::async_runtime::spawn_blocking(move || dispatch_session_signals(&c, signals));
    }
}

// --------------------------------------------------------------- note.due

/// One `note.due` check: the events due now, through the pipeline (a second
/// firing the same day is de-duplicated there). Returns how many events.
pub fn note_due_once(c: &Connectors) -> usize {
    let (now, tz) = (c.now(), c.tz());
    match c
        .db()
        .read(|conn| with_tz(conn, tz, |z| alerts::due_note_events(conn, now, z)))
    {
        Ok(ev) => {
            let n = ev.len();
            c.dispatch_alerts(ev);
            n
        }
        Err(e) => {
            tracing::warn!(target: "mode::alerts", error = %e, "note.due check failed");
            0
        }
    }
}

/// The `note.due` check at launch and every minute (07 §8.1).
pub async fn note_due_task(core: Arc<CoreState>) {
    loop {
        if let Some(c) = core.connectors.get().cloned() {
            let _ = tauri::async_runtime::spawn_blocking(move || note_due_once(&c)).await;
        }
        tokio::time::sleep(NOTE_DUE_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mode_core::urgency::alerts::{NotificationAction, NotificationImage};

    fn note(
        identifier: &str,
        route: Route,
        actions: &[(&str, &str)],
        silent: bool,
    ) -> OsNotification {
        OsNotification {
            identifier: identifier.into(),
            thread_identifier: "thread-1".into(),
            title: "Ana Ruiz".into(),
            subtitle: "Contract v3".into(),
            body: vec!["Can you sign off by Friday?".into()],
            image: Some(NotificationImage::Initials { text: "AR".into() }),
            actions: actions
                .iter()
                .map(|(id, label)| NotificationAction {
                    id: (*id).into(),
                    label: (*label).into(),
                })
                .collect(),
            category: (!actions.is_empty()).then(|| "MODE_MESSAGE".to_string()),
            route,
            silent,
        }
    }

    #[test]
    fn ids_are_stable_positive_and_distinct() {
        let a = notification_id("gmail:<a@x>");
        assert_eq!(a, notification_id("gmail:<a@x>"));
        assert!(a > 0);
        assert_ne!(a, notification_id("gmail:<b@x>"));
        assert!(notification_id("") > 0);
    }

    #[test]
    fn a_fired_alert_plays_on_the_overlay_and_posts_one_notification() {
        let route = Route {
            view: "inbox".into(),
            id: Some("t1".into()),
            focus: Some("m1".into()),
        };
        let out = AlertOutcome {
            event: "message.received".into(),
            item_key: "gmail:<a@x>".into(),
            rules: vec![],
            sound: Some("urgent".into()),
            notification: Some(note(
                "gmail:<a@x>",
                route.clone(),
                &[("mark_read", "Mark read"), ("snooze_tomorrow", "Snooze")],
                true,
            )),
            overlay_only: false,
        };
        let steps = plan(&AlertCommand::Fired(out));
        assert_eq!(
            steps[0],
            SinkStep::Overlay(AlertFired {
                event: "message.received".into(),
                item_key: "gmail:<a@x>".into(),
                sound: Some("urgent".into()),
            })
        );
        let SinkStep::Post(p) = &steps[1] else {
            panic!("{steps:?}")
        };
        assert_eq!(p.id, notification_id("gmail:<a@x>"));
        assert_eq!(p.group, "thread-1");
        assert_eq!(p.body, "Contract v3\nCan you sign off by Friday?");
        assert_eq!(p.action_type.as_deref(), Some("MODE_MESSAGE"));
        assert_eq!(p.actions.len(), 2);
        assert_eq!(p.sound, None, "silent: the overlay plays the cue");
        assert_eq!(p.delivered.route, route);
        let quiet = AlertOutcome {
            event: "session.finished".into(),
            item_key: "k".into(),
            rules: vec![],
            sound: None,
            notification: Some(note("k", route, &[], false)),
            overlay_only: false,
        };
        let steps = plan(&AlertCommand::Fired(quiet));
        let SinkStep::Post(p) = &steps[1] else {
            panic!()
        };
        assert_eq!((p.action_type.as_ref(), p.sound), (None, Some("default")));
        let steps = plan(&AlertCommand::Withdraw {
            identifiers: vec!["a".into(), "b".into()],
        });
        assert_eq!(
            steps,
            vec![SinkStep::Remove {
                identifiers: vec!["a".into(), "b".into()],
                ids: vec![notification_id("a"), notification_id("b")],
            }]
        );
    }

    #[test]
    fn clicks_open_the_route_and_actions_run_without_the_window() {
        let d = |view: &str, id: Option<&str>, focus: Option<&str>| Delivered {
            identifier: "k".into(),
            event: "e".into(),
            route: Route {
                view: view.into(),
                id: id.map(str::to_string),
                focus: focus.map(str::to_string),
            },
        };
        let s = d("sessions", Some("s1"), Some("overview"));
        assert_eq!(
            action_plan("allow", &s),
            ActionPlan::Decide {
                session_id: "s1".into(),
                allow: true
            }
        );
        assert_eq!(
            action_plan("deny", &s),
            ActionPlan::Decide {
                session_id: "s1".into(),
                allow: false
            }
        );
        let ActionPlan::Open(o) = action_plan("tap", &s) else {
            panic!()
        };
        assert_eq!(
            (o.view.as_str(), o.route.as_str()),
            ("sessions", "sessions/s1")
        );
        let m = d("inbox", Some("t1"), Some("m1"));
        assert_eq!(
            action_plan("mark_read", &m),
            ActionPlan::MarkRead {
                id: "m1".into(),
                kind: "message"
            }
        );
        assert_eq!(
            action_plan("snooze_tomorrow", &m),
            ActionPlan::SnoozeTomorrow {
                id: "t1".into(),
                kind: "thread"
            }
        );
        assert_eq!(
            action_plan("join", &d("calendar", Some("e1"), None)),
            ActionPlan::Join {
                event_id: "e1".into()
            }
        );
        let ActionPlan::Open(o) = action_plan("tap", &d("settings", Some("ai"), None)) else {
            panic!()
        };
        assert_eq!(o.route, "settings/ai");
        let ActionPlan::Open(o) = action_plan("allow", &d("stats", None, None)) else {
            panic!()
        };
        assert_eq!(
            o.view, "stats",
            "an action without a target opens the route"
        );
        let ActionPlan::Open(o) = action_plan("tap", &d("nowhere", None, None)) else {
            panic!()
        };
        assert_eq!(o.view, "inbox");
    }
}
