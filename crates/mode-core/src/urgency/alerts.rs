//! Alerts (07 §8–§10): the alert events (§8.1), rule matching over
//! `alert_rules` (§8.2), the suppression ladder (§8.4: de-duplicated, stale,
//! cooldown, manual pause, quiet hours, Focus / DND, presentation, screen
//! lock, muted output, OS permission, master sound switch), quiet hours and
//! Focus detection (§8.5), the `alert_log` identity (§7.3), and the OS
//! notification payload builder (§8.6, §9 templates) the app crate posts.
//!
//! The overlay channel needs no rule (it follows the data); rules add the
//! sound and the OS notification and are the only way to get them (I8).
//! Withheld alerts are never queued (§8.4).

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{Duration, OffsetDateTime};

use super::{dedup, why};
use crate::db::seeds::{ALERT_RULES, AlertRuleSeed};
use crate::db::{Result, format_timestamp, parse_timestamp};
use crate::notes::fields::LocalTz;
use crate::repo::{self, AlertRule};

// ------------------------------------------------------------------ events

/// `session.waiting` (07 §8.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWaiting {
    pub session_id: String,
    pub claude_session_id: String,
    pub display_name: String,
    pub repo_root: Option<String>,
    /// `permission` | `question` | `idle_prompt` | `elicitation` | `agent_needs_input`.
    pub waiting_kind: String,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    /// The trimmed tool input or the question text.
    pub summary: Option<String>,
    pub state_since: String,
}

/// `session.failed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFailed {
    pub session_id: String,
    pub claude_session_id: String,
    pub display_name: String,
    pub repo_root: Option<String>,
    pub reason: Option<String>,
    pub ended_at: String,
}

/// `session.finished`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFinished {
    pub session_id: String,
    pub claude_session_id: String,
    pub display_name: String,
    pub repo_root: Option<String>,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub files_changed: i64,
    pub duration_seconds: i64,
    pub ended_at: String,
    pub last_assistant_preview: Option<String>,
}

/// `message.received`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReceived {
    pub message_id: String,
    pub thread_id: Option<String>,
    pub dedup_key: String,
    pub account_id: String,
    pub provider: String,
    pub kind: String,
    pub sender_contact_id: Option<String>,
    pub sender_name: String,
    pub channel_name: Option<String>,
    pub subject: Option<String>,
    pub snippet: String,
    pub received_at: String,
    pub urgency_tier: String,
    pub urgency_why: Option<String>,
    pub resurfaced: bool,
}

/// `calendar.starting`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarStarting {
    pub event_id: String,
    pub title: String,
    pub starts_at: String,
    pub join_url: Option<String>,
    pub conference: Option<String>,
    pub location: Option<String>,
    pub account_id: String,
    pub calendar_id: String,
    pub external_id: String,
    pub ical_uid: Option<String>,
    pub attendee_count: i64,
    /// The rule threshold that produced it.
    pub minutes_before: i64,
}

/// One due note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueNote {
    pub id: String,
    pub title: String,
    pub urgent: bool,
    pub when_date: String,
}

/// `note.due` (one per day; `count` notes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDue {
    pub date: String,
    pub notes: Vec<DueNote>,
}

/// `usage.threshold` (tokens only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageThreshold {
    /// `claude_api` (the monthly AI token budget) or `claude_code` (weekly).
    pub source: String,
    pub percent: i64,
    pub used: i64,
    pub limit: i64,
    /// `YYYY-MM` (or the week start for Claude Code).
    pub period: String,
}

/// `automation.failed` (03 §7B.3, premade rule "Automation failed"): a run
/// of an automation with `notify_on_failure` failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationFailed {
    pub automation_id: String,
    pub automation_name: String,
    pub run_id: String,
    /// The failure line (`automation_runs.error`).
    pub error: String,
}

/// An alert event (07 §8.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum AlertEvent {
    #[serde(rename = "session.waiting")]
    SessionWaiting(SessionWaiting),
    #[serde(rename = "session.failed")]
    SessionFailed(SessionFailed),
    #[serde(rename = "session.finished")]
    SessionFinished(SessionFinished),
    #[serde(rename = "message.received")]
    MessageReceived(MessageReceived),
    #[serde(rename = "calendar.starting")]
    CalendarStarting(CalendarStarting),
    #[serde(rename = "note.due")]
    NoteDue(NoteDue),
    #[serde(rename = "usage.threshold")]
    UsageThreshold(UsageThreshold),
    #[serde(rename = "automation.failed")]
    AutomationFailed(AutomationFailed),
}

impl AlertEvent {
    pub fn name(&self) -> &'static str {
        match self {
            AlertEvent::SessionWaiting(_) => "session.waiting",
            AlertEvent::SessionFailed(_) => "session.failed",
            AlertEvent::SessionFinished(_) => "session.finished",
            AlertEvent::MessageReceived(_) => "message.received",
            AlertEvent::CalendarStarting(_) => "calendar.starting",
            AlertEvent::NoteDue(_) => "note.due",
            AlertEvent::UsageThreshold(_) => "usage.threshold",
            AlertEvent::AutomationFailed(_) => "automation.failed",
        }
    }

    /// The §7.1 item key.
    pub fn item_key(&self) -> String {
        match self {
            AlertEvent::SessionWaiting(e) => match (&e.waiting_kind[..], &e.tool_use_id) {
                ("permission", Some(t)) if !t.is_empty() => {
                    dedup::session_permission_key(&e.claude_session_id, t)
                }
                _ => dedup::session_wait_key(&e.claude_session_id, &e.state_since),
            },
            AlertEvent::SessionFailed(e) => {
                dedup::session_end_key(&e.claude_session_id, &e.ended_at)
            }
            AlertEvent::SessionFinished(e) => {
                dedup::session_end_key(&e.claude_session_id, &e.ended_at)
            }
            AlertEvent::MessageReceived(e) => e.dedup_key.clone(),
            AlertEvent::CalendarStarting(e) => dedup::calendar_key(
                e.ical_uid.as_deref(),
                &e.account_id,
                &e.calendar_id,
                &e.external_id,
            ),
            AlertEvent::NoteDue(e) => match e.notes.as_slice() {
                [one] => dedup::note_key(&one.id, &e.date),
                _ => dedup::note_key("due", &e.date),
            },
            AlertEvent::UsageThreshold(e) => {
                format!("{}:{}", dedup::usage_key(e.percent, &e.period), e.source)
            }
            AlertEvent::AutomationFailed(e) => format!("automation:{}", e.run_id),
        }
    }

    /// The cooldown scope (§8.2): thread, session, calendar event, note.
    pub fn cooldown_key(&self) -> String {
        match self {
            AlertEvent::SessionWaiting(e) => e.session_id.clone(),
            AlertEvent::SessionFailed(e) => e.session_id.clone(),
            AlertEvent::SessionFinished(e) => e.session_id.clone(),
            AlertEvent::MessageReceived(e) => {
                e.thread_id.clone().unwrap_or_else(|| e.message_id.clone())
            }
            AlertEvent::CalendarStarting(e) => e.event_id.clone(),
            AlertEvent::NoteDue(e) => match e.notes.as_slice() {
                [one] => one.id.clone(),
                _ => "notes".into(),
            },
            AlertEvent::UsageThreshold(e) => e.source.clone(),
            AlertEvent::AutomationFailed(e) => e.automation_id.clone(),
        }
    }
}

// ------------------------------------------------------------------ rules

/// `trigger_json.filters` (07 §8.2). Unknown keys are refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_kind: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes_before: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_join_url: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urgent_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_resurfaced: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
}

/// `trigger_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub event: String,
    #[serde(default)]
    pub filters: Filters,
}

pub const EVENTS: &[&str] = &[
    "session.waiting",
    "session.failed",
    "session.finished",
    "message.received",
    "calendar.starting",
    "note.due",
    "usage.threshold",
    "automation.failed",
];

/// Parse and check a trigger for the Settings editor.
pub fn parse_trigger(raw: &str) -> std::result::Result<Trigger, String> {
    let t: Trigger = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if !EVENTS.contains(&t.event.as_str()) {
        return Err(format!("unknown event {}", t.event));
    }
    if let Some(m) = t.filters.minutes_before
        && !(1..=120).contains(&m)
    {
        return Err("minutes_before is 1–120".into());
    }
    if let Some(at) = &t.filters.at
        && at != "start_of_day"
        && parse_hm(at).is_none()
    {
        return Err("at is start_of_day or HH:MM".into());
    }
    Ok(t)
}

/// The filters each event reads (09 §5.5, the keys [`matches`] looks at).
pub fn event_filters(event: &str) -> &'static [&'static str] {
    match event {
        "session.waiting" => &["waiting_kind", "repo_root"],
        "session.failed" => &["repo_root"],
        "session.finished" => &["repo_root", "min_duration_seconds"],
        "message.received" => &[
            "tier",
            "account_id",
            "contact_id",
            "kind",
            "include_resurfaced",
        ],
        "calendar.starting" => &["minutes_before", "requires_join_url", "account_id"],
        "note.due" => &["at", "urgent_only"],
        "usage.threshold" => &["percent"],
        "automation.failed" => &["automation_id"],
        _ => &[],
    }
}

/// The values a list filter can match (07 §8.1 payloads).
const TIERS: &[&str] = &["vip", "direct", "ai-urgent", "ai-normal", "low"];
const MESSAGE_KINDS: &[&str] = &["email", "dm", "group_dm", "channel", "channel_thread"];
const WAITING_KINDS: &[&str] = &[
    "permission",
    "question",
    "idle_prompt",
    "elicitation",
    "agent_needs_input",
];

/// The Settings editor's check of a rule's filters against its event's
/// schema: only the event's keys (an off-event key would never be read),
/// list values the payload can carry, numbers in range.
pub fn check_filters(event: &str, f: &Filters) -> std::result::Result<(), String> {
    let allowed = event_filters(event);
    let keys = serde_json::to_value(f).map_err(|e| e.to_string())?;
    let stray: Vec<&str> = keys
        .as_object()
        .into_iter()
        .flat_map(|o| o.keys())
        .map(String::as_str)
        .filter(|k| !allowed.contains(k))
        .collect();
    if !stray.is_empty() {
        return Err(format!("{event} has no filter {}", stray.join(", ")));
    }
    for (key, list, known) in [
        ("tier", &f.tier, TIERS),
        ("kind", &f.kind, MESSAGE_KINDS),
        ("waiting_kind", &f.waiting_kind, WAITING_KINDS),
    ] {
        if let Some(bad) = list.iter().flatten().find(|v| !known.contains(&v.as_str())) {
            return Err(format!("{key} {bad:?} is not one of {}", known.join(", ")));
        }
    }
    if f.min_duration_seconds.is_some_and(|m| m < 0) {
        return Err("min_duration_seconds is 0 or more".into());
    }
    if f.percent.is_some_and(|p| !(1..=100).contains(&p)) {
        return Err("percent is 1–100".into());
    }
    Ok(())
}

/// Validate a whole rule (`respect_dnd = 0` only for session rules, filters
/// the event's, 07 §8.2).
pub fn validate_rule(rule: &AlertRule) -> std::result::Result<(), String> {
    let t = parse_trigger(&rule.trigger_json)?;
    check_filters(&t.event, &t.filters)?;
    let ch: Vec<String> =
        serde_json::from_str(&rule.channels_json).map_err(|_| "channels_json is not a list")?;
    if !ch.iter().any(|c| c == "overlay") {
        return Err("the overlay channel is always on".into());
    }
    if ch
        .iter()
        .any(|c| !matches!(c.as_str(), "overlay" | "os_notification" | "sound"))
    {
        return Err("channels are overlay, os_notification, sound".into());
    }
    if !rule.respect_dnd && !matches!(t.event.as_str(), "session.waiting" | "session.failed") {
        return Err("only session rules may ignore Focus / Do Not Disturb".into());
    }
    Ok(())
}

// ------------------------------------------------------------ premade

/// The seed a premade rule comes from (07 §8.2, ADR-0048): by id, else by
/// name (a restored or merged database keeps its own ids). Custom rules
/// have none.
pub fn premade_seed(rule: &AlertRule) -> Option<&'static AlertRuleSeed> {
    if rule.kind != "premade" {
        return None;
    }
    ALERT_RULES
        .iter()
        .find(|s| s.id == rule.id)
        .or_else(|| ALERT_RULES.iter().find(|s| s.name == rule.name))
}

/// **Reset**: the seed's definition and position written back (overlay
/// only, Focus and quiet hours respected, as the migration seeds it).
pub fn reset_to_seed(rule: &mut AlertRule, seed: &AlertRuleSeed) {
    rule.name = seed.name.into();
    rule.enabled = seed.enabled;
    rule.trigger_json = seed.trigger_json.into();
    rule.channels_json = r#"["overlay"]"#.into();
    rule.sound = seed.sound.map(str::to_string);
    rule.respect_dnd = true;
    rule.quiet_hours = true;
    rule.cooldown_seconds = seed.cooldown_seconds;
    rule.position = seed.position;
}

/// A trigger with its list filters sorted, so `["vip","direct"]` and
/// `["direct","vip"]` compare equal.
fn normalised(mut t: Trigger) -> Trigger {
    for list in [
        &mut t.filters.tier,
        &mut t.filters.kind,
        &mut t.filters.waiting_kind,
    ]
    .into_iter()
    .flatten()
    {
        list.sort();
        list.dedup();
    }
    t
}

fn channel_set(raw: &str) -> Option<std::collections::BTreeSet<String>> {
    serde_json::from_str::<Vec<String>>(raw)
        .ok()
        .map(|v| v.into_iter().collect())
}

/// `edited` on a rule row: a premade rule whose definition differs from
/// its seed in a field the Settings editor changes (name, enabled, event
/// and filters, channels, sound, cooldown, `respect_dnd`, `quiet_hours`).
/// Compared by value, not by `rev`: a Reset, or editing the values back by
/// hand, reads unedited again. The list order (`position`) is not part of
/// the definition. Custom rules are never edited.
pub fn edited(rule: &AlertRule) -> bool {
    let Some(seed) = premade_seed(rule) else {
        return false;
    };
    let trigger = |raw: &str| serde_json::from_str::<Trigger>(raw).ok().map(normalised);
    let same = rule.name == seed.name
        && rule.enabled == seed.enabled
        && trigger(&rule.trigger_json).is_some_and(|t| Some(t) == trigger(seed.trigger_json))
        && channel_set(&rule.channels_json) == channel_set(r#"["overlay"]"#)
        && rule.sound.as_deref() == seed.sound
        && rule.cooldown_seconds == seed.cooldown_seconds
        && rule.respect_dnd
        && rule.quiet_hours;
    !same
}

fn in_list(list: &Option<Vec<String>>, v: &str) -> bool {
    list.as_ref().is_none_or(|l| l.iter().any(|x| x == v))
}

fn repo_prefix(f: &Filters, repo: &Option<String>) -> bool {
    match &f.repo_root {
        None => true,
        Some(p) => repo.as_deref().is_some_and(|r| r.starts_with(p.as_str())),
    }
}

/// Whether `f` selects `e` (values matched exactly; `repo_root` by prefix).
pub fn matches(f: &Filters, e: &AlertEvent) -> bool {
    match e {
        AlertEvent::MessageReceived(m) => {
            in_list(&f.tier, &m.urgency_tier)
                && f.account_id.as_ref().is_none_or(|a| *a == m.account_id)
                && f.contact_id
                    .as_ref()
                    .is_none_or(|c| m.sender_contact_id.as_deref() == Some(c.as_str()))
                && in_list(&f.kind, &m.kind)
                && (!m.resurfaced || f.include_resurfaced == Some(true))
        }
        AlertEvent::SessionWaiting(s) => {
            in_list(&f.waiting_kind, &s.waiting_kind) && repo_prefix(f, &s.repo_root)
        }
        AlertEvent::SessionFailed(s) => repo_prefix(f, &s.repo_root),
        AlertEvent::SessionFinished(s) => {
            repo_prefix(f, &s.repo_root)
                && f.min_duration_seconds
                    .is_none_or(|m| s.duration_seconds >= m)
        }
        AlertEvent::CalendarStarting(c) => {
            f.minutes_before.unwrap_or(5) == c.minutes_before
                && (f.requires_join_url != Some(true) || c.join_url.is_some())
                && f.account_id.as_ref().is_none_or(|a| *a == c.account_id)
        }
        AlertEvent::NoteDue(n) => f.urgent_only != Some(true) || n.notes.iter().any(|x| x.urgent),
        AlertEvent::UsageThreshold(u) => f.percent.is_none_or(|p| p == u.percent),
        AlertEvent::AutomationFailed(a) => f
            .automation_id
            .as_ref()
            .is_none_or(|id| *id == a.automation_id),
    }
}

/// Enabled, live rules for `event`, by position.
pub fn rules_for(conn: &Connection, event: &str) -> Result<Vec<(AlertRule, Trigger)>> {
    let rules = repo::query::<AlertRule>(
        conn,
        "enabled = 1 AND deleted_at IS NULL ORDER BY position, name",
        &[],
    )?;
    Ok(rules
        .into_iter()
        .filter_map(|r| {
            let t = parse_trigger(&r.trigger_json).ok()?;
            (t.event == event).then_some((r, t))
        })
        .collect())
}

// ---------------------------------------------------------- quiet / focus

/// `notifications.quiet_hours` (07 §8.5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "qh_from")]
    pub from: String,
    #[serde(default = "qh_to")]
    pub to: String,
}

fn qh_from() -> String {
    "22:00".into()
}
fn qh_to() -> String {
    "08:00".into()
}

impl Default for QuietHours {
    fn default() -> Self {
        QuietHours {
            enabled: false,
            from: qh_from(),
            to: qh_to(),
        }
    }
}

pub fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

impl QuietHours {
    /// Active at a local wall-clock minute of the day. A window whose `to`
    /// is earlier than `from` crosses midnight; `from == to` is always quiet.
    pub fn active_at(&self, minute_of_day: u32) -> bool {
        if !self.enabled {
            return false;
        }
        let (Some(f), Some(t)) = (parse_hm(&self.from), parse_hm(&self.to)) else {
            return false;
        };
        if f == t {
            true
        } else if f < t {
            minute_of_day >= f && minute_of_day < t
        } else {
            minute_of_day >= f || minute_of_day < t
        }
    }

    pub fn active(&self, now: OffsetDateTime, tz: &dyn LocalTz) -> bool {
        let l = tz.to_local(now);
        self.active_at(l.hour() as u32 * 60 + l.minute() as u32)
    }
}

/// macOS Focus (07 §8.5.2): an active `storeAssertionRecords` entry in
/// `~/Library/DoNotDisturb/DB/Assertions.json` means a Focus is on.
/// `None` when the file is not in the expected shape (unknown).
pub fn macos_focus_from_assertions(json: &str) -> Option<bool> {
    let v: Value = serde_json::from_str(json).ok()?;
    let data = v.get("data")?.as_array()?;
    Some(data.iter().any(|d| {
        d.get("storeAssertionRecords")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    }))
}

/// Read the Focus state where Mode can (macOS assertions file); `None`
/// means unknown, which counts as "not in Focus" (§8.5.2). Windows' state
/// comes from the app crate (`SHQueryUserNotificationState`) through
/// [`Environment::focus`].
pub fn detect_focus(enabled: bool) -> Option<bool> {
    if !enabled {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        let text =
            std::fs::read_to_string(home.join("Library/DoNotDisturb/DB/Assertions.json")).ok()?;
        macos_focus_from_assertions(&text)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The OS notification permission (§8.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsPermission {
    Granted,
    Denied,
    /// Not asked yet / unknown: post and let the OS decide.
    #[default]
    Unknown,
}

/// Everything the suppression ladder reads besides the rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub quiet_hours: QuietHours,
    pub pause_until: Option<String>,
    /// Detected Focus / DND; `None` = unknown (treated as off).
    pub focus: Option<bool>,
    pub presentation: bool,
    pub screen_locked: bool,
    pub output_muted: bool,
    pub os_permission: OsPermission,
    pub sounds_enabled: bool,
    pub max_alert_age_minutes: i64,
    /// `notifications.os_preview = "minimal"`.
    pub preview_minimal: bool,
    /// `notifications.os_sound`.
    pub os_sound: bool,
}

impl Environment {
    /// The settings half; the OS half (focus, presentation, lock, muted,
    /// permission) is filled by the caller.
    pub fn load(conn: &Connection) -> Result<Environment> {
        use crate::settings::{get_as_in, get_bool_in, get_i64_in, get_string_in};
        Ok(Environment {
            quiet_hours: get_as_in(conn, "notifications.quiet_hours").unwrap_or_default(),
            pause_until: get_string_in(conn, "notifications.pause_until")?,
            sounds_enabled: get_bool_in(conn, "sounds.enabled")?.unwrap_or(false),
            max_alert_age_minutes: get_i64_in(conn, "notifications.max_alert_age_minutes")?
                .unwrap_or(30),
            preview_minimal: get_string_in(conn, "notifications.os_preview")?.as_deref()
                == Some("minimal"),
            os_sound: get_bool_in(conn, "notifications.os_sound")?.unwrap_or(false),
            ..Default::default()
        })
    }

    pub fn paused(&self, now: OffsetDateTime) -> bool {
        self.pause_until
            .as_deref()
            .and_then(parse_timestamp)
            .is_some_and(|t| t > now)
    }
}

/// `quiet.changed` (contract `QuietChanged`), merged: quiet hours and the
/// manual pause from the core, Focus from detection, presentation from the
/// app's overlay controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QuietChanged {
    pub quiet_hours: bool,
    pub focus: bool,
    pub presentation: bool,
    /// "Pause alerts for 1 h" (additive field; the mascot sleeps too).
    pub paused: bool,
}

pub fn quiet_state(env: &Environment, now: OffsetDateTime, tz: &dyn LocalTz) -> QuietChanged {
    QuietChanged {
        quiet_hours: env.quiet_hours.active(now, tz),
        focus: env.focus.unwrap_or(false),
        presentation: env.presentation,
        paused: env.paused(now),
    }
}

// ---------------------------------------------------------------- pipeline

/// One rule's verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleOutcome {
    pub rule_id: String,
    pub rule_name: String,
    /// Delivered channels (`overlay` always when not dropped).
    pub channels: Vec<String>,
    /// The first suppression row that applied (`dedup`, `stale`, `cooldown`,
    /// `paused`, `quiet_hours`, `focus`, `presentation`, `screen_locked`,
    /// `muted`, `os_permission`, `sounds_off`).
    pub suppressed_reason: Option<String>,
}

/// What the app does: play a cue, post a notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertOutcome {
    pub event: String,
    pub item_key: String,
    pub rules: Vec<RuleOutcome>,
    /// The first matching rule's cue, once (§8.2).
    pub sound: Option<String>,
    pub notification: Option<OsNotification>,
    /// `true` when nothing matched (overlay only).
    pub overlay_only: bool,
}

fn is_stale(e: &AlertEvent, env: &Environment, now: OffsetDateTime) -> bool {
    match e {
        AlertEvent::MessageReceived(m) => parse_timestamp(&m.received_at)
            .is_some_and(|r| now - r > Duration::minutes(env.max_alert_age_minutes.max(0))),
        AlertEvent::CalendarStarting(c) => parse_timestamp(&c.starts_at).is_some_and(|s| {
            now - (s - Duration::minutes(c.minutes_before)) > Duration::minutes(2)
        }),
        _ => false,
    }
}

fn already_logged(conn: &Connection, rule_id: &str, event: &str, key: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM alert_log WHERE rule_id = ?1 AND event = ?2 AND item_key = ?3",
            params![rule_id, event, key],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn in_cooldown(
    conn: &Connection,
    rule: &AlertRule,
    key: &str,
    now: OffsetDateTime,
) -> Result<bool> {
    if rule.cooldown_seconds <= 0 {
        return Ok(false);
    }
    let since = format_timestamp(now - Duration::seconds(rule.cooldown_seconds));
    Ok(conn
        .query_row(
            "SELECT 1 FROM alert_log WHERE rule_id = ?1 AND cooldown_key = ?2
               AND suppressed_reason IS NULL AND fired_at >= ?3 LIMIT 1",
            params![rule.id, key, since],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Run one event through the pipeline of §8.7. With `test = true` (the
/// Settings **Test** button) nothing is written and suppression is reported
/// instead of applied.
pub fn evaluate(
    conn: &Connection,
    event: &AlertEvent,
    env: &Environment,
    now: OffsetDateTime,
    tz: &dyn LocalTz,
    test: bool,
) -> Result<AlertOutcome> {
    let key = event.item_key();
    let ck = event.cooldown_key();
    let name = event.name();
    let stale = is_stale(event, env, now);
    let quiet = env.quiet_hours.active(now, tz);
    let paused = env.paused(now);
    let focus = env.focus.unwrap_or(false);
    let now_s = format_timestamp(now);
    let mut outcome = AlertOutcome {
        event: name.into(),
        item_key: key.clone(),
        rules: Vec::new(),
        sound: None,
        notification: None,
        overlay_only: true,
    };
    let mut post_os = false;
    for (rule, trigger) in rules_for(conn, name)? {
        if !matches(&trigger.filters, event) {
            continue;
        }
        outcome.overlay_only = false;
        let wanted: Vec<String> = serde_json::from_str(&rule.channels_json).unwrap_or_default();
        let mut ro = RuleOutcome {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            channels: vec!["overlay".into()],
            suppressed_reason: None,
        };
        // 1–3: dedup, stale, cooldown drop or withhold everything but the overlay.
        if already_logged(conn, &rule.id, name, &key)? {
            ro.channels.clear();
            ro.suppressed_reason = Some("dedup".into());
            outcome.rules.push(ro);
            continue;
        }
        let pre = if stale {
            Some("stale")
        } else if in_cooldown(conn, &rule, &ck, now)? {
            Some("cooldown")
        } else {
            None
        };
        let session_rule = matches!(name, "session.waiting" | "session.failed");
        let respect_dnd = rule.respect_dnd || !session_rule;
        let mut sound = wanted.iter().any(|c| c == "sound");
        let mut os = wanted.iter().any(|c| c == "os_notification");
        let mut reason: Option<&str> = pre;
        let withhold = |cond: bool,
                        r: &'static str,
                        s: bool,
                        o: bool,
                        reason: &mut Option<&str>,
                        sound: &mut bool,
                        os: &mut bool| {
            if cond && ((s && *sound) || (o && *os)) {
                if s {
                    *sound = false;
                }
                if o {
                    *os = false;
                }
                reason.get_or_insert(r);
            }
        };
        if pre.is_some() {
            sound = false;
            os = false;
        }
        withhold(
            paused,
            "paused",
            true,
            true,
            &mut reason,
            &mut sound,
            &mut os,
        );
        withhold(
            quiet && rule.quiet_hours,
            "quiet_hours",
            true,
            true,
            &mut reason,
            &mut sound,
            &mut os,
        );
        withhold(
            focus && respect_dnd,
            "focus",
            true,
            true,
            &mut reason,
            &mut sound,
            &mut os,
        );
        withhold(
            env.presentation,
            "presentation",
            true,
            true,
            &mut reason,
            &mut sound,
            &mut os,
        );
        withhold(
            env.screen_locked,
            "screen_locked",
            true,
            false,
            &mut reason,
            &mut sound,
            &mut os,
        );
        withhold(
            env.output_muted,
            "muted",
            true,
            false,
            &mut reason,
            &mut sound,
            &mut os,
        );
        withhold(
            env.os_permission == OsPermission::Denied,
            "os_permission",
            false,
            true,
            &mut reason,
            &mut sound,
            &mut os,
        );
        withhold(
            !env.sounds_enabled,
            "sounds_off",
            true,
            false,
            &mut reason,
            &mut sound,
            &mut os,
        );
        if sound {
            ro.channels.push("sound".into());
            if outcome.sound.is_none() {
                outcome.sound = rule.sound.clone();
            }
        }
        if os {
            ro.channels.push("os_notification".into());
            post_os = true;
        }
        ro.suppressed_reason = reason.map(str::to_string);
        if !test {
            let fired = pre.is_none().then(|| now_s.clone());
            conn.execute(
                "INSERT OR IGNORE INTO alert_log (id, rule_id, event, item_key, cooldown_key, channels_json,
                        suppressed_reason, fired_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    repo::new_id(),
                    rule.id,
                    name,
                    key,
                    ck,
                    serde_json::to_string(&ro.channels).unwrap_or_else(|_| "[]".into()),
                    ro.suppressed_reason,
                    fired,
                    now_s
                ],
            )?;
            if pre.is_none() {
                conn.execute(
                    "UPDATE alert_rules SET last_fired_at = ?2 WHERE id = ?1",
                    params![rule.id, now_s],
                )?;
            }
        }
        outcome.rules.push(ro);
    }
    if post_os {
        let tctx = TemplateCtx::load(conn, event)?;
        let mut n = notification(event, &tctx, env.preview_minimal);
        // The cue plays through the overlay; the OS sound only without one (§8.6).
        n.silent = outcome.sound.is_some() || !env.os_sound;
        outcome.notification = Some(n);
    }
    Ok(outcome)
}

// ------------------------------------------------------- notifications

/// Where a click goes (03 §2.5 deep links).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub view: String,
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

/// A notification action button (§8.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    /// `allow` | `deny` | `mark_read` | `snooze_tomorrow` | `join`.
    pub id: String,
    pub label: String,
}

/// The picture beside the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationImage {
    Avatar {
        url: String,
    },
    Initials {
        text: String,
    },
    Session {
        initial: String,
        color: Option<String>,
    },
    Glyph {
        name: String,
    },
}

/// The OS notification payload the app crate posts (§8.6, §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsNotification {
    /// The item key: replaces an earlier one, and withdraws it later.
    pub identifier: String,
    /// Stacks per conversation / session.
    pub thread_identifier: String,
    pub title: String,
    pub subtitle: String,
    /// Body lines (≤ 100 chars each).
    pub body: Vec<String>,
    pub image: Option<NotificationImage>,
    pub actions: Vec<NotificationAction>,
    /// `MODE_APPROVAL` for permission cards.
    pub category: Option<String>,
    pub route: Route,
    pub silent: bool,
}

/// Extra rows the templates read (thread title, account label, names).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateCtx {
    pub thread_title: Option<String>,
    /// The account's label, only when more than one account exists.
    pub account_label: Option<String>,
    pub sender_avatar_url: Option<String>,
    pub participant_names: Vec<String>,
    pub session_color: Option<String>,
}

impl TemplateCtx {
    pub fn load(conn: &Connection, e: &AlertEvent) -> Result<TemplateCtx> {
        let mut t = TemplateCtx::default();
        let accounts: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        let label = |id: &str| -> Result<Option<String>> {
            if accounts <= 1 {
                return Ok(None);
            }
            Ok(conn
                .query_row(
                    "SELECT display_name FROM accounts WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?)
        };
        match e {
            AlertEvent::MessageReceived(m) => {
                t.account_label = label(&m.account_id)?;
                if let Some(th) = &m.thread_id {
                    let row: Option<(String, String)> = conn
                        .query_row(
                            "SELECT title, participants_json FROM threads WHERE id = ?1",
                            [th],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if let Some((title, parts)) = row {
                        t.thread_title = Some(title);
                        t.participant_names = serde_json::from_str::<Vec<Value>>(&parts)
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|p| {
                                p.get("name").and_then(Value::as_str).map(str::to_string)
                            })
                            .collect();
                    }
                }
                t.sender_avatar_url = conn
                    .query_row(
                        "SELECT sender_avatar_url FROM messages WHERE id = ?1",
                        [&m.message_id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
            }
            AlertEvent::CalendarStarting(c) => t.account_label = label(&c.account_id)?,
            AlertEvent::SessionWaiting(SessionWaiting { session_id, .. })
            | AlertEvent::SessionFailed(SessionFailed { session_id, .. })
            | AlertEvent::SessionFinished(SessionFinished { session_id, .. }) => {
                t.session_color = conn
                    .query_row(
                        "SELECT avatar_color FROM sessions WHERE id = ?1",
                        [session_id],
                        |r| r.get(0),
                    )
                    .optional()?;
            }
            _ => {}
        }
        Ok(t)
    }
}

/// Title ≤ 50, subtitle ≤ 60, body lines ≤ 100 (§9.2 "Truncation").
pub const TITLE_MAX: usize = 50;
pub const SUBTITLE_MAX: usize = 60;
pub const LINE_MAX: usize = 100;

fn first_chars(s: &str, n: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    why::truncate_words(&flat, n)
}

fn repo_name(repo: &Option<String>) -> String {
    repo.as_deref()
        .map(|r| {
            r.trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(r)
                .to_string()
        })
        .unwrap_or_default()
}

fn duration_text(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn act(id: &str, label: &str) -> NotificationAction {
    NotificationAction {
        id: id.into(),
        label: label.into(),
    }
}

/// Build the §9.2 payload for an event (`minimal` = `os_preview = minimal`).
pub fn notification(e: &AlertEvent, t: &TemplateCtx, minimal: bool) -> OsNotification {
    let suffix = |s: String| match &t.account_label {
        Some(a) => format!("{s} · {a}"),
        None => s,
    };
    let (mut title, mut subtitle, mut body, image, actions, category, route, thread);
    match e {
        AlertEvent::MessageReceived(m) => {
            let urgent = matches!(m.urgency_tier.as_str(), "vip" | "direct" | "ai-urgent");
            title = if urgent {
                format!("{} (urgent)", m.sender_name)
            } else {
                m.sender_name.clone()
            };
            let why_line = m
                .urgency_why
                .as_deref()
                .filter(|w| !w.is_empty())
                .map(|w| format!("· {}", why::display(w)));
            let resurfaced = m.resurfaced.then(|| "· resurfaced".to_string());
            if m.provider == "slack" {
                subtitle = match m.kind.as_str() {
                    "dm" => "Slack · DM".to_string(),
                    "group_dm" => {
                        let names = t.participant_names.join(", ");
                        if names.is_empty() {
                            "Slack · group DM".into()
                        } else {
                            format!("Slack · group DM with {names}")
                        }
                    }
                    "channel_thread" => format!(
                        "Slack · {} · thread",
                        m.channel_name.clone().unwrap_or_default()
                    ),
                    _ => format!("Slack · {}", m.channel_name.clone().unwrap_or_default()),
                };
                body = vec![first_chars(&m.snippet, 80)];
            } else {
                subtitle = format!(
                    "Gmail · {}",
                    t.thread_title
                        .clone()
                        .or_else(|| m.subject.clone())
                        .unwrap_or_default()
                );
                body = vec![
                    m.subject
                        .clone()
                        .unwrap_or_else(|| first_chars(&m.snippet, 80)),
                ];
            }
            body.extend(why_line.or(resurfaced));
            subtitle = suffix(subtitle);
            image = Some(match &t.sender_avatar_url {
                Some(u) => NotificationImage::Avatar { url: u.clone() },
                None => NotificationImage::Initials {
                    text: initials(&m.sender_name),
                },
            });
            actions = vec![
                act("mark_read", "Mark read"),
                act("snooze_tomorrow", "Snooze"),
            ];
            category = Some("MODE_MESSAGE".to_string());
            route = Route {
                view: "inbox".into(),
                id: m.thread_id.clone(),
                focus: Some(m.message_id.clone()),
            };
            thread = m.thread_id.clone().unwrap_or_else(|| m.message_id.clone());
        }
        AlertEvent::SessionWaiting(s) => {
            let repo = repo_name(&s.repo_root);
            match s.waiting_kind.as_str() {
                "permission" => {
                    title = format!("{} needs you", s.display_name);
                    subtitle = format!("{repo} · waiting for you");
                    body = vec![format!(
                        "{}: {}",
                        s.tool_name.clone().unwrap_or_else(|| "Tool".into()),
                        s.summary.clone().unwrap_or_default()
                    )];
                    actions = vec![act("allow", "Allow"), act("deny", "Deny")];
                    category = Some("MODE_APPROVAL".to_string());
                }
                "question" => {
                    title = format!("{} asks", s.display_name);
                    subtitle = repo.clone();
                    body = vec![first_chars(s.summary.as_deref().unwrap_or(""), 120)];
                    actions = vec![];
                    category = None;
                }
                other => {
                    title = format!("{} is waiting", s.display_name);
                    subtitle = repo.clone();
                    body = vec![if other == "idle_prompt" {
                        "turn ended, waiting for a prompt".into()
                    } else {
                        "needs input in the terminal".into()
                    }];
                    actions = vec![];
                    category = None;
                }
            }
            image = Some(NotificationImage::Session {
                initial: initials(&s.display_name),
                color: t.session_color.clone(),
            });
            route = Route {
                view: "sessions".into(),
                id: Some(s.session_id.clone()),
                focus: Some("overview".into()),
            };
            thread = s.session_id.clone();
        }
        AlertEvent::SessionFailed(s) => {
            title = format!("{} failed", s.display_name);
            subtitle = repo_name(&s.repo_root);
            body = vec![
                s.reason
                    .clone()
                    .unwrap_or_else(|| "the session failed".into()),
            ];
            image = Some(NotificationImage::Session {
                initial: initials(&s.display_name),
                color: t.session_color.clone(),
            });
            actions = vec![];
            category = None;
            route = Route {
                view: "sessions".into(),
                id: Some(s.session_id.clone()),
                focus: Some("overview".into()),
            };
            thread = s.session_id.clone();
        }
        AlertEvent::SessionFinished(s) => {
            title = format!("{} finished", s.display_name);
            subtitle = format!(
                "{} · {}",
                repo_name(&s.repo_root),
                duration_text(s.duration_seconds)
            );
            body = vec![format!(
                "{} files, +{} −{}",
                s.files_changed, s.lines_added, s.lines_removed
            )];
            body.extend(
                s.last_assistant_preview
                    .as_deref()
                    .map(|p| first_chars(p, 100)),
            );
            image = Some(NotificationImage::Session {
                initial: initials(&s.display_name),
                color: t.session_color.clone(),
            });
            actions = vec![];
            category = None;
            route = Route {
                view: "sessions".into(),
                id: Some(s.session_id.clone()),
                focus: Some("overview".into()),
            };
            thread = s.session_id.clone();
        }
        AlertEvent::CalendarStarting(c) => {
            title = c.title.clone();
            subtitle = suffix(format!("in {} min", c.minutes_before));
            body = vec![
                c.location
                    .clone()
                    .or_else(|| c.conference.clone())
                    .unwrap_or_default(),
            ];
            if c.attendee_count > 0 {
                body.push(format!("{} attendees", c.attendee_count));
            }
            body.retain(|l| !l.is_empty());
            image = Some(NotificationImage::Glyph {
                name: c.conference.clone().unwrap_or_else(|| "calendar".into()),
            });
            actions = if c.join_url.is_some() {
                vec![act("join", "Join")]
            } else {
                vec![]
            };
            category = Some("MODE_EVENT".to_string());
            route = Route {
                view: "calendar".into(),
                id: Some(c.event_id.clone()),
                focus: None,
            };
            thread = c.event_id.clone();
        }
        AlertEvent::NoteDue(n) => {
            title = match n.notes.as_slice() {
                [one] => one.title.clone(),
                many => format!("{} notes due today", many.len()),
            };
            subtitle = "Notes".into();
            body = vec![
                n.notes
                    .iter()
                    .take(3)
                    .map(|x| {
                        if x.urgent {
                            format!("✱ {}", x.title)
                        } else {
                            x.title.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" · "),
            ];
            image = Some(NotificationImage::Glyph {
                name: "mascot_waiting".into(),
            });
            actions = vec![];
            category = None;
            route = Route {
                view: "notes".into(),
                id: Some("today".into()),
                focus: None,
            };
            thread = "notes".into();
        }
        AlertEvent::UsageThreshold(u) => {
            title = format!("Usage at {}%", u.percent);
            subtitle = if u.source == "claude_code" {
                "Mode · this week".into()
            } else {
                "Mode · this month".into()
            };
            body = vec![format!(
                "{} of {} tokens",
                tokens_text(u.used),
                tokens_text(u.limit)
            )];
            image = None;
            actions = vec![];
            category = None;
            route = Route {
                view: if u.source == "claude_code" {
                    "stats".into()
                } else {
                    "settings".into()
                },
                id: (u.source != "claude_code").then(|| "ai".into()),
                focus: None,
            };
            thread = "usage".into();
        }
        AlertEvent::AutomationFailed(a) => {
            title = format!("{} failed", a.automation_name);
            subtitle = "Automations".into();
            body = vec![a.error.clone()];
            image = Some(NotificationImage::Glyph {
                name: "automations".into(),
            });
            actions = vec![];
            category = None;
            route = Route {
                view: "automations".into(),
                id: Some(a.automation_id.clone()),
                focus: Some(a.run_id.clone()),
            };
            thread = a.automation_id.clone();
        }
    }
    if minimal {
        title = match e {
            AlertEvent::MessageReceived(_) => "New urgent message".into(),
            AlertEvent::SessionWaiting(_) => "Session needs you".into(),
            AlertEvent::SessionFinished(_) => "Session finished".into(),
            AlertEvent::SessionFailed(_) => "Session failed".into(),
            AlertEvent::CalendarStarting(_) => "Meeting starting soon".into(),
            AlertEvent::NoteDue(_) => "Notes due today".into(),
            AlertEvent::UsageThreshold(_) => "Usage threshold reached".into(),
            AlertEvent::AutomationFailed(_) => "Automation failed".into(),
        };
        subtitle = String::new();
        body = Vec::new();
    }
    OsNotification {
        identifier: e.item_key(),
        thread_identifier: thread,
        title: first_chars(&title, TITLE_MAX),
        subtitle: first_chars(&subtitle, SUBTITLE_MAX),
        body: body
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| first_chars(&l, LINE_MAX))
            .collect(),
        image: if minimal { None } else { image },
        actions,
        category,
        route,
        silent: true,
    }
}

/// "1.6M", "820k", "950" (the usage gauge's form; never a currency).
pub fn tokens_text(n: i64) -> String {
    if n >= 1_000_000 {
        let v = n as f64 / 1_000_000.0;
        let s = format!("{v:.1}");
        format!("{}M", s.trim_end_matches(".0"))
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

/// At most one OS notification per second across all rules; the excess in a
/// second is coalesced into one "n more urgent items" (§8.6 "Rate").
#[derive(Debug, Default)]
pub struct OsRateLimiter {
    last: Option<OffsetDateTime>,
    coalesced: u32,
}

impl OsRateLimiter {
    /// `true` = post now; `false` = coalesced.
    pub fn admit(&mut self, now: OffsetDateTime) -> bool {
        match self.last {
            Some(t) if now - t < Duration::seconds(1) => {
                self.coalesced += 1;
                false
            }
            _ => {
                self.last = Some(now);
                true
            }
        }
    }

    /// The summary of what was coalesced, once a second has passed.
    pub fn flush(&mut self, now: OffsetDateTime) -> Option<OsNotification> {
        if self.coalesced == 0 || self.last.is_some_and(|t| now - t < Duration::seconds(1)) {
            return None;
        }
        let n = std::mem::take(&mut self.coalesced);
        self.last = Some(now);
        Some(OsNotification {
            identifier: format!("mode:coalesced:{}", format_timestamp(now)),
            thread_identifier: "mode".into(),
            title: format!("{n} more urgent items"),
            subtitle: String::new(),
            body: vec![],
            image: None,
            actions: vec![],
            category: None,
            route: Route {
                view: "inbox".into(),
                id: None,
                focus: None,
            },
            silent: true,
        })
    }
}

/// The notification identifiers to withdraw when an item is resolved in
/// Mode (§8.6 "Withdrawal"): a message (its dedup key), a thread (its
/// messages' keys), a session (every key logged for it).
pub fn withdraw_keys(conn: &Connection, target_kind: &str, target_id: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |k: String| {
        if !out.contains(&k) {
            out.push(k)
        }
    };
    match target_kind {
        "message" => {
            if let Some(k) = conn
                .query_row(
                    "SELECT dedup_key FROM messages WHERE id = ?1",
                    [target_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            {
                push(k);
            }
        }
        "thread" => {
            let mut st =
                conn.prepare("SELECT DISTINCT dedup_key FROM messages WHERE thread_id = ?1")?;
            for k in st.query_map([target_id], |r| r.get::<_, String>(0))? {
                push(k?);
            }
        }
        "session" => {
            let csid: Option<String> = conn
                .query_row(
                    "SELECT claude_session_id FROM sessions WHERE id = ?1",
                    [target_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(c) = csid {
                let mut st = conn.prepare(
                    "SELECT DISTINCT item_key FROM alert_log WHERE item_key LIKE ?1 AND fired_at IS NOT NULL",
                )?;
                for k in st.query_map([format!("session:{c}:%")], |r| r.get::<_, String>(0))? {
                    push(k?);
                }
            }
        }
        "calendar_event" => {
            let row: Option<(Option<String>, String, String, String)> = conn
                .query_row(
                    "SELECT ical_uid, account_id, calendar_id, external_id FROM calendar_events WHERE id = ?1",
                    [target_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            if let Some((u, a, c, e)) = row {
                push(dedup::calendar_key(u.as_deref(), &a, &c, &e));
            }
        }
        _ => {}
    }
    Ok(out)
}

// ------------------------------------------------------------- emitters

/// `calendar.starting` events due now, one per enabled rule threshold
/// (`minutes_before`), skipping canceled, declined, all-day and snoozed
/// events and those missed by more than 2 minutes.
pub fn due_calendar_events(conn: &Connection, now: OffsetDateTime) -> Result<Vec<AlertEvent>> {
    let mut thresholds: Vec<i64> = rules_for(conn, "calendar.starting")?
        .iter()
        .map(|(_, t)| t.filters.minutes_before.unwrap_or(5))
        .collect();
    thresholds.sort_unstable();
    thresholds.dedup();
    let now_s = format_timestamp(now);
    let mut out = Vec::new();
    for m in thresholds {
        let from = format_timestamp(now - Duration::minutes(2) + Duration::minutes(m));
        let to = format_timestamp(now + Duration::minutes(m));
        let mut st = conn.prepare(
            "SELECT e.id, e.title, e.starts_at, e.join_url, e.conference, e.location, e.account_id,
                    e.calendar_id, e.external_id, e.ical_uid, e.attendees_json
               FROM calendar_events e
              WHERE e.status <> 'cancelled' AND e.my_response IS NOT 'declined' AND e.all_day = 0
                AND e.starts_at > ?1 AND e.starts_at <= ?2 AND e.starts_at > ?3
                AND NOT EXISTS (SELECT 1 FROM snoozes z WHERE z.target_kind = 'calendar_event'
                     AND z.target_id = e.id AND z.fired_at IS NULL AND z.cancelled_at IS NULL
                     AND z.deleted_at IS NULL AND z.until > ?3)",
        )?;
        let rows = st.query_map(params![from, to, now_s], |r| {
            let att: String = r.get(10)?;
            Ok(CalendarStarting {
                event_id: r.get(0)?,
                title: r.get(1)?,
                starts_at: r.get(2)?,
                join_url: r.get(3)?,
                conference: r.get(4)?,
                location: r.get(5)?,
                account_id: r.get(6)?,
                calendar_id: r.get(7)?,
                external_id: r.get(8)?,
                ical_uid: r.get(9)?,
                attendee_count: serde_json::from_str::<Vec<Value>>(&att)
                    .map(|v| v.len() as i64)
                    .unwrap_or(0),
                minutes_before: m,
            })
        })?;
        for r in rows {
            out.push(AlertEvent::CalendarStarting(r?));
        }
    }
    Ok(out)
}

/// `note.due` for today (07 §8.1): once the local time reaches the earliest
/// enabled rule's `at` (`start_of_day` = local 00:00, else `HH:MM`), one
/// event listing the notes with `when_date <= today`, an open status (not a
/// `closed` one of `notes.statuses`), not archived, not trashed and not
/// snoozed. The item key is per day (per note when only one is due), so a
/// caller may run this every minute: the pipeline de-duplicates a second
/// firing on the same day. Empty before the time, with no rule, or with
/// nothing due.
pub fn due_note_events(
    conn: &Connection,
    now: OffsetDateTime,
    tz: &dyn LocalTz,
) -> Result<Vec<AlertEvent>> {
    let earliest = rules_for(conn, "note.due")?
        .iter()
        .map(|(_, t)| match t.filters.at.as_deref() {
            None | Some("start_of_day") => 0,
            Some(hm) => parse_hm(hm).unwrap_or(0),
        })
        .min();
    let Some(at) = earliest else {
        return Ok(Vec::new());
    };
    let local = tz.to_local(now);
    if (local.hour() as u32 * 60 + local.minute() as u32) < at {
        return Ok(Vec::new());
    }
    let today = local.date();
    let date = format!(
        "{:04}-{:02}-{:02}",
        today.year(),
        u8::from(today.month()),
        today.day()
    );
    let mut closed = crate::settings::closed_statuses_in(conn).unwrap_or_default();
    if closed.is_empty() {
        closed.push("done".into());
    }
    let marks = vec!["?"; closed.len()].join(", ");
    let sql = format!(
        "SELECT n.id, n.title, n.urgent, n.when_date FROM notes n
          WHERE n.deleted_at IS NULL AND n.archived_at IS NULL
            AND n.when_date IS NOT NULL AND substr(n.when_date, 1, 10) <= ?
            AND n.status NOT IN ({marks})
            AND NOT EXISTS (SELECT 1 FROM snoozes z WHERE z.target_kind = 'note'
                 AND z.target_id = n.id AND z.fired_at IS NULL AND z.cancelled_at IS NULL
                 AND z.deleted_at IS NULL AND z.until > ?)
          ORDER BY n.urgent DESC, n.when_date, n.title"
    );
    let mut args: Vec<String> = vec![date.clone()];
    args.extend(closed);
    args.push(format_timestamp(now));
    let mut st = conn.prepare(&sql)?;
    let notes = st
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok(DueNote {
                id: r.get(0)?,
                title: r.get(1)?,
                urgent: r.get(2)?,
                when_date: r.get::<_, String>(3)?.chars().take(10).collect(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if notes.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![AlertEvent::NoteDue(NoteDue { date, notes })])
}

/// `usage.threshold` for the monthly AI token budget at each enabled rule's
/// `percent` (once per month per threshold by its item key).
pub fn usage_events(
    conn: &Connection,
    used: i64,
    limit: i64,
    month: &str,
) -> Result<Vec<AlertEvent>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    let pct = used * 100 / limit;
    Ok(rules_for(conn, "usage.threshold")?
        .iter()
        .filter_map(|(_, t)| t.filters.percent)
        .filter(|p| pct >= *p)
        .map(|p| {
            AlertEvent::UsageThreshold(UsageThreshold {
                source: "claude_api".into(),
                percent: p,
                used,
                limit,
                period: month.to_string(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours_windows() {
        let q = QuietHours {
            enabled: true,
            from: "22:00".into(),
            to: "08:00".into(),
        };
        assert!(q.active_at(23 * 60));
        assert!(q.active_at(7 * 60 + 59));
        assert!(!q.active_at(8 * 60));
        assert!(!q.active_at(12 * 60));
        let day = QuietHours {
            enabled: true,
            from: "12:00".into(),
            to: "13:00".into(),
        };
        assert!(day.active_at(12 * 60 + 30));
        assert!(!day.active_at(13 * 60));
        let always = QuietHours {
            enabled: true,
            from: "09:00".into(),
            to: "09:00".into(),
        };
        assert!(always.active_at(0));
        assert!(!QuietHours::default().active_at(23 * 60));
    }

    #[test]
    fn focus_assertions_parse() {
        assert_eq!(
            macos_focus_from_assertions(
                r#"{"data":[{"storeAssertionRecords":[{"assertionDetails":{}}]}]}"#
            ),
            Some(true)
        );
        assert_eq!(
            macos_focus_from_assertions(r#"{"data":[{"storeAssertionRecords":[]}]}"#),
            Some(false)
        );
        assert_eq!(macos_focus_from_assertions("nope"), None);
    }

    #[test]
    fn triggers_refuse_unknown_keys() {
        assert!(
            parse_trigger(r#"{"event":"message.received","filters":{"tier":["vip"]}}"#).is_ok()
        );
        assert!(
            parse_trigger(r#"{"event":"message.received","filters":{"color":"red"}}"#).is_err()
        );
        assert!(parse_trigger(r#"{"event":"nope.happened","filters":{}}"#).is_err());
        assert!(
            parse_trigger(r#"{"event":"calendar.starting","filters":{"minutes_before":500}}"#)
                .is_err()
        );
        assert!(parse_trigger(r#"{"event":"note.due","filters":{"at":"25:00"}}"#).is_err());
    }

    #[test]
    fn tokens_text_has_no_currency() {
        assert_eq!(tokens_text(1_600_000), "1.6M");
        assert_eq!(tokens_text(2_000_000), "2M");
        assert_eq!(tokens_text(820_000), "820k");
        assert_eq!(tokens_text(950), "950");
    }

    #[test]
    fn due_notes_wait_for_the_rule_time_and_skip_closed_archived_and_snoozed() {
        use crate::notes::ctx::TzMode;
        use crate::repo::{Note, Snooze};
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open(&dir.path().join("mode.db")).unwrap();
        let note = |id: &str, status: &str, when: Option<&str>, urgent: bool| Note {
            id: id.into(),
            path: format!("{id}.md"),
            title: format!("Note {id}"),
            status: status.into(),
            urgent,
            when_date: when.map(str::to_string),
            source_kind: "manual".into(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
            updated_at: "2026-10-01T00:00:00.000Z".into(),
            device_id: "d".into(),
            ..Default::default()
        };
        let rows = vec![
            note("due", "todo", Some("2026-10-21"), false),
            note("late", "doing", Some("2026-10-19"), true),
            note("future", "todo", Some("2026-10-22"), false),
            note("closed", "done", Some("2026-10-20"), false),
            note("undated", "todo", None, false),
            Note {
                archived_at: Some("2026-10-02T00:00:00.000Z".into()),
                ..note("archived", "todo", Some("2026-10-20"), false)
            },
            note("snoozed", "todo", Some("2026-10-20"), false),
        ];
        db.write(move |c| {
            for n in &rows {
                repo::insert(c, n)?;
            }
            repo::insert(
                c,
                &Snooze {
                    id: "z1".into(),
                    target_kind: "note".into(),
                    target_id: "snoozed".into(),
                    until: "2026-10-22T08:00:00.000Z".into(),
                    created_at: "2026-10-21T00:00:00.000Z".into(),
                    updated_at: "2026-10-21T00:00:00.000Z".into(),
                    device_id: "d".into(),
                    ..Default::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
        let at = |s: &str| parse_timestamp(s).unwrap();
        let run = |now: OffsetDateTime| {
            db.read(|c| {
                crate::urgency::with_tz(c, TzMode::Fixed(120), |z| due_note_events(c, now, z))
            })
            .unwrap()
        };
        // 2026-10-21 10:00 local (UTC+2): the premade "Due today" rule fires at start of day.
        let ev = run(at("2026-10-21T08:00:00.000Z"));
        assert_eq!(ev.len(), 1);
        let AlertEvent::NoteDue(n) = &ev[0] else {
            panic!("{ev:?}")
        };
        assert_eq!(n.date, "2026-10-21");
        let ids: Vec<&str> = n.notes.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(
            ids,
            ["late", "due"],
            "urgent first; closed, archived, snoozed, future and undated left out"
        );
        assert_eq!(ev[0].item_key(), dedup::note_key("due", "2026-10-21"));
        // A fixed time: nothing before it.
        db.write(|c| {
            c.execute(
                "UPDATE alert_rules SET trigger_json = '{\"event\":\"note.due\",\"filters\":{\"at\":\"09:30\"}}' WHERE trigger_json LIKE '%note.due%'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            run(at("2026-10-21T07:00:00.000Z")).is_empty(),
            "09:00 local"
        );
        assert_eq!(run(at("2026-10-21T07:31:00.000Z")).len(), 1, "09:31 local");
        db.write(|c| {
            c.execute(
                "UPDATE alert_rules SET enabled = 0 WHERE trigger_json LIKE '%note.due%'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            run(at("2026-10-21T12:00:00.000Z")).is_empty(),
            "no rule, no event"
        );
    }

    #[test]
    fn rate_limiter_coalesces_within_a_second() {
        let t0 = parse_timestamp("2026-10-22T10:00:00.000Z").unwrap();
        let mut r = OsRateLimiter::default();
        assert!(r.admit(t0));
        assert!(!r.admit(t0 + Duration::milliseconds(300)));
        assert!(!r.admit(t0 + Duration::milliseconds(600)));
        assert!(r.flush(t0 + Duration::milliseconds(700)).is_none());
        let s = r.flush(t0 + Duration::milliseconds(1500)).unwrap();
        assert_eq!(s.title, "2 more urgent items");
        assert!(r.admit(t0 + Duration::milliseconds(2600)));
    }
}
