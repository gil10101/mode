//! `mode-wire`: the hook wire protocol shared by the relay (`mode-hook`) and the
//! core (`mode-core`). Serde types only, no I/O; see `docs/05-sessions.md` §3.4.
//!
//! One JSON object per line (NDJSON) in both directions. `v` is the protocol
//! version, `id` is a UUID v7 minted by the relay and echoed by the core.

#![deny(unsafe_code)]

pub mod endpoint;
pub mod id;
pub mod scan;
#[cfg(windows)]
pub mod sid;
pub mod time;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use endpoint::Endpoint;

/// Wire protocol version carried in every frame as `v`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest frame either side accepts (05 §3.3, §5.2: 8 MB, truncated beyond).
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Session ids of the installer's synthetic round trip (05 §3.6 S10). The
/// core acknowledges and records such a frame but never creates a session.
pub const ROUND_TRIP_SESSION_PREFIX: &str = "mode-round-trip-";

/// Hook event names that Mode registers (05 §3.1).
pub mod events {
    pub const PERMISSION_REQUEST: &str = "PermissionRequest";
    pub const PRE_TOOL_USE: &str = "PreToolUse";
    pub const POST_TOOL_USE: &str = "PostToolUse";
    pub const POST_TOOL_USE_FAILURE: &str = "PostToolUseFailure";
    pub const SESSION_START: &str = "SessionStart";
    pub const SESSION_END: &str = "SessionEnd";
    pub const USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
    pub const NOTIFICATION: &str = "Notification";
    pub const STOP: &str = "Stop";
    pub const STOP_FAILURE: &str = "StopFailure";
    pub const SUBAGENT_START: &str = "SubagentStart";
    pub const SUBAGENT_STOP: &str = "SubagentStop";
    pub const PRE_COMPACT: &str = "PreCompact";
    pub const POST_COMPACT: &str = "PostCompact";
    pub const CWD_CHANGED: &str = "CwdChanged";
    pub const PRE_MODEL_SWITCH: &str = "PreModelSwitch";
    pub const POST_MODEL_SWITCH: &str = "PostModelSwitch";
    pub const ELICITATION: &str = "Elicitation";

    /// The tool whose `PreToolUse` the relay waits on (the question card).
    pub const ASK_USER_QUESTION_TOOL: &str = "AskUserQuestion";
}

/// `effort` as Claude Code 2.1.305 sends it: `{"level": "medium"}` (SP-4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Effort {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One entry of `permission_suggestions` on a `PermissionRequest` (SP-4):
/// `{"type": "setMode", "mode": "acceptEdits", "destination": "session"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionSuggestion {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

/// The JSON Claude Code writes to a hook's stdin. The common fields and every
/// field SP-4 observed on CLI 2.1.305 are typed; everything else (`tool_input`,
/// `tool_response`, `prompt`, `stop_hook_active`, `duration_ms`, …) stays
/// verbatim in `extra`, which must be a JSON object. Serializing gives back
/// exactly the object that was parsed (key order aside).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookEvent {
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    pub hook_event_name: String,

    // --- fields observed in spike SP-4 (recorded payloads), all optional ---
    /// Per-session scratch directory (every event).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratchpad_dir: Option<String>,
    /// The prompt this event belongs to (every event after `UserPromptSubmit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PostToolUseFailure`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// `PreToolUse`, `PostToolUse`, `PostToolUseFailure` (not on `PermissionRequest` in 2.1.305).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// `PermissionRequest` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_suggestions: Option<Vec<PermissionSuggestion>>,
    /// `Notification`: `permission_prompt`, `idle_prompt`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<String>,
    /// `Notification` text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `SessionStart`: `startup`, `resume`, `clear`, `compact`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `SessionEnd` (`other`, `clear`, `logout`, …) and `PreCompact`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `SubagentStart` / `SubagentStop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// `Stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_assistant_message: Option<String>,

    /// Every field Claude Code sent that is not typed above.
    #[serde(flatten)]
    pub extra: Value,
}

/// Keys that live in a typed field of [`HookEvent`] rather than in `extra`.
pub const TYPED_KEYS: &[&str] = &[
    "session_id",
    "transcript_path",
    "cwd",
    "permission_mode",
    "hook_event_name",
    "scratchpad_dir",
    "prompt_id",
    "effort",
    "tool_name",
    "tool_use_id",
    "permission_suggestions",
    "notification_type",
    "message",
    "source",
    "reason",
    "agent_id",
    "agent_type",
    "last_assistant_message",
];

impl HookEvent {
    /// A payload with the common fields set and no extra fields.
    pub fn new(hook_event_name: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            transcript_path: String::new(),
            cwd: String::new(),
            permission_mode: None,
            hook_event_name: hook_event_name.into(),
            scratchpad_dir: None,
            prompt_id: None,
            effort: None,
            tool_name: None,
            tool_use_id: None,
            permission_suggestions: None,
            notification_type: None,
            message: None,
            source: None,
            reason: None,
            agent_id: None,
            agent_type: None,
            last_assistant_message: None,
            extra: Value::Object(Map::new()),
        }
    }

    /// An untyped field by name (`tool_input`, `tool_response`, `prompt`, …).
    /// Typed fields are not found here; use [`HookEvent::field`] for both.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.extra.get(key)
    }

    /// Any field by name, typed or not, as an owned JSON value.
    pub fn field(&self, key: &str) -> Option<Value> {
        if TYPED_KEYS.contains(&key) {
            serde_json::to_value(self)
                .ok()
                .and_then(|v| v.get(key).cloned())
        } else {
            self.get(key).cloned()
        }
    }

    /// Set a field by name, routing typed keys into their field (builders, tests).
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        let value = value.into();
        if TYPED_KEYS.contains(&key) {
            let mut all = serde_json::to_value(&*self).unwrap_or(Value::Null);
            if let Value::Object(map) = &mut all {
                map.insert(key.to_string(), value);
                if let Ok(back) = serde_json::from_value::<HookEvent>(all) {
                    *self = back;
                }
            }
        } else if let Value::Object(map) = &mut self.extra {
            map.insert(key.to_string(), value);
        }
    }

    pub fn tool_name(&self) -> Option<&str> {
        self.tool_name.as_deref()
    }

    pub fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    /// `tool_input` when present and an object.
    pub fn tool_input(&self) -> Option<&Map<String, Value>> {
        self.get("tool_input").and_then(Value::as_object)
    }

    /// Whether the relay must hold the connection open for a `decision`
    /// (05 §3.1): `PermissionRequest`, and `PreToolUse` for `AskUserQuestion`.
    pub fn wants_decision(&self) -> bool {
        wants_decision(&self.hook_event_name, self.tool_name())
    }

    /// A synthetic frame from the installer's round trip (05 §3.6).
    pub fn is_round_trip(&self) -> bool {
        self.session_id.starts_with(ROUND_TRIP_SESSION_PREFIX)
    }
}

/// Decision rule shared by the relay (which knows the event from `argv`)
/// and the core.
pub fn wants_decision(event: &str, tool_name: Option<&str>) -> bool {
    event == events::PERMISSION_REQUEST
        || (event == events::PRE_TOOL_USE && tool_name == Some(events::ASK_USER_QUESTION_TOOL))
}

/// relay → core, once per hook event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookFrame {
    pub v: u32,
    pub id: String,
    /// The event name from `argv[1]` (falls back to `payload.hook_event_name`).
    pub event: String,
    pub relay_version: String,
    pub wants_decision: bool,
    /// RFC 3339 UTC with milliseconds.
    pub sent_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ppid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    /// Allow-listed environment variables only (05 §9.2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// How long the relay will wait for a decision before it exits on its own
    /// (`timeout − 2 s`, 05 §3.3); the core's watchdog answers `no_opinion`
    /// just before this runs out. Only on decision frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_wait_ms: Option<u64>,
    pub payload: HookEvent,
}

/// core → relay for every `hook` frame; ends the exchange for non-decision events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckFrame {
    pub v: u32,
    pub id: String,
}

impl AckFrame {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
        }
    }
}

/// core → relay when a pending decision is resolved; `output` is printed verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionFrame {
    pub v: u32,
    pub id: String,
    pub output: Value,
}

impl DecisionFrame {
    pub fn new(id: impl Into<String>, output: Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            output,
        }
    }

    /// The decision frame for a `PermissionRequest`.
    pub fn permission(id: impl Into<String>, decision: &PermissionDecision) -> Self {
        Self::new(id, decision.output())
    }
}

/// core → relay when Claude Code's own prompt should be used instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoOpinionFrame {
    pub v: u32,
    pub id: String,
}

impl NoOpinionFrame {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
        }
    }
}

/// Why a relay gave up on a pending decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    Deadline,
    Signal,
}

/// relay → core, best effort, just before the relay exits without a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelFrame {
    pub v: u32,
    pub id: String,
    pub reason: CancelReason,
}

impl CancelFrame {
    pub fn new(id: impl Into<String>, reason: CancelReason) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            reason,
        }
    }
}

/// Every frame on the wire, discriminated by `kind`. The hook frame is boxed:
/// it is ten times the size of the others and lives once per connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Frame {
    Hook(Box<HookFrame>),
    Ack(AckFrame),
    Decision(DecisionFrame),
    NoOpinion(NoOpinionFrame),
    Cancel(CancelFrame),
}

impl Frame {
    pub fn hook(frame: HookFrame) -> Self {
        Frame::Hook(Box::new(frame))
    }

    /// The `id` every frame carries.
    pub fn id(&self) -> &str {
        match self {
            Frame::Hook(f) => &f.id,
            Frame::Ack(f) => &f.id,
            Frame::Decision(f) => &f.id,
            Frame::NoOpinion(f) => &f.id,
            Frame::Cancel(f) => &f.id,
        }
    }

    /// Serialize as one NDJSON line (with the trailing newline).
    pub fn to_line(&self) -> serde_json::Result<String> {
        let mut s = serde_json::to_string(self)?;
        s.push('\n');
        Ok(s)
    }

    /// Parse one NDJSON line (the trailing newline is optional).
    pub fn from_line(line: &str) -> serde_json::Result<Self> {
        serde_json::from_str(line.trim_end_matches(['\n', '\r']))
    }
}

/// `hookSpecificOutput.decision.behavior` for a `PermissionRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Behavior {
    Allow,
    Deny,
}

/// What Mode answers to a `PermissionRequest` (05 §3.4, "The `output` objects
/// Mode produces, exactly").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub behavior: Behavior,
    /// Shown to Claude Code as `message` on deny; ignored on allow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PermissionDecision {
    pub const DEFAULT_DENY_MESSAGE: &str = "Denied in Mode";

    pub fn allow() -> Self {
        Self {
            behavior: Behavior::Allow,
            reason: None,
        }
    }

    pub fn deny(reason: Option<String>) -> Self {
        Self {
            behavior: Behavior::Deny,
            reason,
        }
    }

    /// The exact JSON object the relay prints to stdout.
    pub fn output(&self) -> Value {
        let decision = match self.behavior {
            Behavior::Allow => serde_json::json!({ "behavior": "allow" }),
            Behavior::Deny => serde_json::json!({
                "behavior": "deny",
                "message": self.reason.clone().unwrap_or_else(|| Self::DEFAULT_DENY_MESSAGE.to_string()),
            }),
        };
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": events::PERMISSION_REQUEST,
                "decision": decision,
            }
        })
    }
}

/// The `output` that pre-answers an `AskUserQuestion` (05 §3.4, §7.3):
/// `permissionDecision: allow` with `updatedInput` = `tool_input` plus
/// `answers` mapping each question text to the chosen label(s).
pub fn question_output(tool_input: &Value, answers: &Map<String, Value>) -> Value {
    let mut updated = match tool_input {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    updated.insert("answers".into(), Value::Object(answers.clone()));
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": events::PRE_TOOL_USE,
            "permissionDecision": "allow",
            "updatedInput": Value::Object(updated),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const PERMISSION_REQUEST: &str =
        include_str!("../../../fixtures/hooks/permission_request.json");
    const SESSION_START: &str = include_str!("../../../fixtures/hooks/session_start.json");

    /// The payloads recorded from Claude Code 2.1.305 (SP-4).
    fn recorded_fixtures() -> Vec<(String, String)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hooks/2.1.305");
        let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .map(|p| {
                (
                    p.file_stem().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).unwrap(),
                )
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn golden_fixtures_parse_and_reserialise_equivalently() {
        let fixtures = recorded_fixtures();
        assert!(fixtures.len() >= 7, "expected the seven SP-4 payloads");
        for (name, text) in &fixtures {
            let ev: HookEvent =
                serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(ev.hook_event_name, *name, "file name is the event");
            assert_eq!(ev.session_id, "b4e66983-4e8e-5447-b97c-85be9cf5e843");
            assert_eq!(ev.cwd, "/home/ana/code/scratch");
            assert!(
                ev.scratchpad_dir
                    .as_deref()
                    .unwrap()
                    .ends_with("/scratchpad")
            );
            let back: Value = serde_json::to_value(&ev).unwrap();
            let original: Value = serde_json::from_str(text).unwrap();
            assert_eq!(back, original, "{name}: re-serialization differs");
            for key in TYPED_KEYS {
                assert!(
                    ev.get(key).is_none(),
                    "{name}: `{key}` must not stay in extra"
                );
            }
            // The frame carrying it round-trips too.
            let frame = Frame::hook(HookFrame {
                v: PROTOCOL_VERSION,
                id: id::uuid_v7(),
                event: name.clone(),
                relay_version: "0.1.0".into(),
                wants_decision: ev.wants_decision(),
                sent_at: time::now_rfc3339(),
                ppid: Some(1),
                tty: None,
                env: BTreeMap::new(),
                launch_id: None,
                truncated: false,
                decision_wait_ms: ev.wants_decision().then_some(118_000),
                payload: ev,
            });
            let line = frame.to_line().unwrap();
            assert_eq!(Frame::from_line(&line).unwrap(), frame, "{name}");
        }
    }

    #[test]
    fn golden_fixtures_expose_the_sp4_fields() {
        let by_name: BTreeMap<String, HookEvent> = recorded_fixtures()
            .into_iter()
            .map(|(n, t)| (n, serde_json::from_str(&t).unwrap()))
            .collect();

        let pr = &by_name["PermissionRequest"];
        assert_eq!(pr.permission_mode.as_deref(), Some("default"));
        assert_eq!(
            pr.prompt_id.as_deref(),
            Some("b3abfbb6-1c03-4549-a612-94df97e139f6")
        );
        assert_eq!(pr.effort.as_ref().unwrap().level.as_deref(), Some("medium"));
        assert_eq!(pr.tool_name(), Some("Write"));
        assert_eq!(
            pr.tool_use_id(),
            None,
            "2.1.305 sends no tool_use_id on PermissionRequest"
        );
        let s = &pr.permission_suggestions.as_ref().unwrap()[0];
        assert_eq!(s.kind, "setMode");
        assert_eq!(s.fields["mode"], "acceptEdits");
        assert_eq!(s.fields["destination"], "session");
        assert_eq!(
            pr.tool_input().unwrap()["file_path"],
            "/home/ana/code/scratch/hello.txt"
        );
        assert!(pr.wants_decision());

        let post = &by_name["PostToolUse"];
        assert_eq!(post.tool_use_id(), Some("toolu_01RSzoUP4eTUbB8dxpfpd96Q"));
        assert_eq!(post.get("duration_ms"), Some(&Value::from(10)));
        assert_eq!(post.get("tool_response").unwrap()["type"], "create");
        assert!(!post.wants_decision());

        let pre = &by_name["PreToolUse"];
        assert_eq!(pre.tool_use_id(), Some("toolu_01RSzoUP4eTUbB8dxpfpd96Q"));
        assert!(!pre.wants_decision(), "Write is not a question");

        assert_eq!(by_name["SessionEnd"].reason.as_deref(), Some("other"));
        assert_eq!(by_name["SessionStart"].source.as_deref(), Some("startup"));
        assert!(by_name["SessionStart"].prompt_id.is_none());
        let stop = &by_name["Stop"];
        assert!(
            stop.last_assistant_message
                .as_deref()
                .unwrap()
                .starts_with("I created")
        );
        assert_eq!(stop.get("stop_hook_active"), Some(&Value::Bool(false)));
        let prompt = by_name["UserPromptSubmit"]
            .get("prompt")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(
            prompt.starts_with("Create a file named hello.txt"),
            "{prompt}"
        );
    }

    #[test]
    fn hook_event_keeps_unknown_fields_in_extra_and_round_trips() {
        let ev: HookEvent = serde_json::from_str(PERMISSION_REQUEST).unwrap();
        assert_eq!(ev.session_id, "7f3a1c2e-4b5d-4e6f-8a9b-0c1d2e3f4a5b");
        assert_eq!(ev.cwd, "/Users/ana/code/mode");
        assert_eq!(ev.permission_mode.as_deref(), Some("default"));
        assert_eq!(ev.hook_event_name, "PermissionRequest");
        assert_eq!(ev.tool_name(), Some("Bash"));
        assert_eq!(ev.tool_use_id(), Some("toolu_01AbCdEfGhIjKlMnOpQrStUv"));
        assert_eq!(ev.get("tool_input").unwrap()["command"], "npm test");
        assert!(ev.wants_decision());
        assert!(!ev.is_round_trip());

        let back: Value = serde_json::to_value(&ev).unwrap();
        let original: Value = serde_json::from_str(PERMISSION_REQUEST).unwrap();
        assert_eq!(
            back, original,
            "serialization must be byte-for-byte equivalent"
        );
    }

    #[test]
    fn field_and_set_cover_typed_and_untyped_keys() {
        let mut ev = HookEvent::new("PreToolUse", "s1");
        ev.set("tool_name", "AskUserQuestion");
        ev.set("tool_input", serde_json::json!({"questions": []}));
        ev.set("notification_type", "idle_prompt");
        assert_eq!(ev.tool_name(), Some("AskUserQuestion"));
        assert!(ev.wants_decision());
        assert_eq!(ev.field("tool_name"), Some(Value::from("AskUserQuestion")));
        assert_eq!(
            ev.field("tool_input").unwrap()["questions"],
            serde_json::json!([])
        );
        assert_eq!(ev.notification_type.as_deref(), Some("idle_prompt"));
        assert!(
            ev.get("tool_name").is_none(),
            "typed keys never land in extra"
        );
        assert!(ev.field("nope").is_none());
    }

    #[test]
    fn session_start_does_not_want_a_decision() {
        let ev: HookEvent = serde_json::from_str(SESSION_START).unwrap();
        assert!(!ev.wants_decision());
        assert_eq!(ev.source.as_deref(), Some("startup"));
        assert_eq!(
            ev.get("model").and_then(Value::as_str),
            Some("claude-opus-5-5")
        );
        assert!(wants_decision("PreToolUse", Some("AskUserQuestion")));
        assert!(!wants_decision("PreToolUse", Some("Bash")));
    }

    #[test]
    fn empty_extra_serialises_as_plain_object() {
        let ev = HookEvent::new("Stop", "s1");
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            r#"{"session_id":"s1","transcript_path":"","cwd":"","hook_event_name":"Stop"}"#
        );
        let back: HookEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
        let synthetic = HookEvent::new("SessionStart", format!("{ROUND_TRIP_SESSION_PREFIX}abc"));
        assert!(synthetic.is_round_trip());
    }

    #[test]
    fn frames_round_trip_with_kind_tag() {
        let payload: HookEvent = serde_json::from_str(PERMISSION_REQUEST).unwrap();
        let hook = Frame::hook(HookFrame {
            v: PROTOCOL_VERSION,
            id: "0192aaaa-bbbb-7ccc-8ddd-eeeeffff0000".into(),
            event: "PermissionRequest".into(),
            relay_version: "0.1.0".into(),
            wants_decision: true,
            sent_at: "2026-10-18T14:02:11.004Z".into(),
            ppid: Some(48213),
            tty: Some("/dev/ttys004".into()),
            env: BTreeMap::from([("TERM_PROGRAM".to_string(), "iTerm.app".to_string())]),
            launch_id: None,
            truncated: false,
            decision_wait_ms: Some(118_000),
            payload,
        });
        let line = hook.to_line().unwrap();
        assert!(
            line.starts_with(r#"{"kind":"hook","v":1,"id":"0192aaaa"#),
            "{line}"
        );
        assert!(line.ends_with('\n'));
        assert!(
            !line.contains("\"truncated\""),
            "false truncated is omitted"
        );
        assert!(!line.contains("\"launch_id\""));
        assert!(line.contains(r#""decision_wait_ms":118000"#));
        assert_eq!(Frame::from_line(&line).unwrap(), hook);

        for frame in [
            Frame::Ack(AckFrame::new("a")),
            Frame::Decision(DecisionFrame::permission("b", &PermissionDecision::allow())),
            Frame::NoOpinion(NoOpinionFrame::new("c")),
            Frame::Cancel(CancelFrame::new("d", CancelReason::Deadline)),
        ] {
            let line = frame.to_line().unwrap();
            assert_eq!(Frame::from_line(&line).unwrap(), frame, "{line}");
        }

        let ack = Frame::from_line(r#"{"v":1,"kind":"ack","id":"0192…"}"#).unwrap();
        assert_eq!(
            ack,
            Frame::Ack(AckFrame {
                v: 1,
                id: "0192…".into()
            })
        );
        assert_eq!(ack.id(), "0192…");
        let cancel = Frame::Cancel(CancelFrame::new("d", CancelReason::Signal))
            .to_line()
            .unwrap();
        assert!(cancel.contains(r#""reason":"signal""#));
        // A frame from an older relay without `decision_wait_ms` still parses.
        let old = r#"{"kind":"hook","v":1,"id":"x","event":"Stop","relay_version":"0.0.1","wants_decision":false,"sent_at":"2026-10-18T14:02:11.004Z","payload":{"session_id":"s","hook_event_name":"Stop"}}"#;
        let Frame::Hook(h) = Frame::from_line(old).unwrap() else {
            panic!()
        };
        assert_eq!(h.decision_wait_ms, None);
    }

    #[test]
    fn permission_outputs_match_the_spec_exactly() {
        assert_eq!(
            PermissionDecision::allow().output(),
            serde_json::json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}})
        );
        assert_eq!(
            PermissionDecision::deny(None).output(),
            serde_json::json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied in Mode"}}})
        );
        assert_eq!(
            PermissionDecision::deny(Some("not on main".into())).output()["hookSpecificOutput"]["decision"]
                ["message"],
            "not on main"
        );
        let decision: PermissionDecision =
            serde_json::from_str(r#"{"behavior":"deny","reason":"x"}"#).unwrap();
        assert_eq!(decision.behavior, Behavior::Deny);
        assert_eq!(decision.reason.as_deref(), Some("x"));
    }

    #[test]
    fn question_output_adds_answers_to_the_original_input() {
        let input =
            serde_json::json!({"questions": [{"question": "Which?", "options": [{"label": "A"}]}]});
        let mut answers = Map::new();
        answers.insert("Which?".into(), Value::from("A"));
        let out = question_output(&input, &answers);
        assert_eq!(
            out,
            serde_json::json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"questions":[{"question":"Which?","options":[{"label":"A"}]}],"answers":{"Which?":"A"}}}})
        );
    }
}
