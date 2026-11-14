//! The fixture backend: no network. Two modes, usable together:
//! * **canned**: replies (or errors) queued by a test, served in order —
//!   the scoring contract tests (schema checks, retries, refusals);
//! * **rubric**: a deterministic classifier that reads the scoring batch of
//!   07 §3.4.3 and applies the tier definitions of the system prompt with
//!   keyword rules — the stand-in model for the labeled-set metrics until
//!   a real key exists (`tests/urgency_eval.rs`).
//!
//! Every request is recorded so tests can check what would have left the
//! machine (07 I4).

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::{AiBackend, AiError, AiRequest, AiResponse, BackendKind, Usage};

pub struct FixtureBackend {
    kind: BackendKind,
    canned: Mutex<VecDeque<Result<AiResponse, AiError>>>,
    rubric: bool,
    models: Vec<String>,
    requests: Mutex<Vec<AiRequest>>,
}

impl std::fmt::Debug for FixtureBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixtureBackend")
            .field("kind", &self.kind)
            .field("rubric", &self.rubric)
            .finish_non_exhaustive()
    }
}

impl FixtureBackend {
    /// Canned replies only; an empty queue answers "unreachable".
    pub fn canned(kind: BackendKind) -> FixtureBackend {
        FixtureBackend {
            kind,
            canned: Mutex::new(VecDeque::new()),
            rubric: false,
            models: vec!["fixture-model".into()],
            requests: Mutex::new(Vec::new()),
        }
    }

    /// The rubric classifier (canned replies still go first).
    pub fn rubric(kind: BackendKind) -> FixtureBackend {
        FixtureBackend {
            rubric: true,
            ..FixtureBackend::canned(kind)
        }
    }

    pub fn push_text(&self, text: &str) {
        self.push(Ok(AiResponse {
            text: text.to_string(),
            usage: Some(Usage {
                input_tokens: 900,
                output_tokens: 120,
                cache_read_tokens: 1200,
                cache_write_tokens: 0,
                cache_write_1h_tokens: 0,
            }),
            request_id: Some(format!("req_fixture_{}", crate::repo::new_id())),
            stop_reason: Some("end_turn".into()),
            model: "fixture-model".into(),
            source: self.kind,
        }));
    }

    pub fn push(&self, r: Result<AiResponse, AiError>) {
        self.canned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(r);
    }

    pub fn requests(&self) -> Vec<AiRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl AiBackend for FixtureBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn complete(&self, req: &AiRequest) -> Result<AiResponse, AiError> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
        if let Some(r) = self
            .canned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
        {
            return r;
        }
        if !self.rubric {
            return Err(AiError::Unreachable("fixture".into()));
        }
        let batch: Value = serde_json::from_str(&req.user)
            .map_err(|e| AiError::BadOutput(format!("fixture input: {e}")))?;
        let items: Vec<Value> = batch
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let today = batch
            .get("now")
            .and_then(Value::as_str)
            .and_then(|n| n.get(..10))
            .and_then(crate::notes::fields::parse_date)
            .unwrap_or_else(|| time::OffsetDateTime::now_utc().date());
        let out: Vec<Value> = items
            .iter()
            .map(|it| {
                let (tier, why, conf) = classify(it, today);
                json!({"i": it.get("i").cloned().unwrap_or(json!(0)), "tier": tier, "why": why, "confidence": conf})
            })
            .collect();
        let text = json!({ "items": out }).to_string();
        Ok(AiResponse {
            usage: Some(Usage {
                input_tokens: (req.user.len() / 4) as i64,
                output_tokens: (text.len() / 4) as i64,
                cache_read_tokens: (req.system.len() / 4) as i64,
                cache_write_tokens: 0,
                cache_write_1h_tokens: 0,
            }),
            text,
            request_id: Some(format!("req_fixture_{}", crate::repo::new_id())),
            stop_reason: Some("end_turn".into()),
            model: req.model.clone(),
            source: self.kind,
        })
    }

    fn list_models(&self) -> Result<Vec<String>, AiError> {
        Ok(self.models.clone())
    }
}

fn has_any(text: &str, words: &[&str]) -> Option<String> {
    words
        .iter()
        .find(|w| text.contains(*w))
        .map(|w| w.to_string())
}

/// The rubric: the three tier definitions of the 07 §3.4.3 prompt as rules.
pub fn classify(item: &Value, today: time::Date) -> (&'static str, String, f64) {
    let s = |k: &str| {
        item.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let b = |k: &str| item.get(k).and_then(Value::as_bool).unwrap_or(false);
    let hints: Vec<String> = item
        .get("hints")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| h.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let text = format!("{} \n {}", s("subject"), s("excerpt")).to_lowercase();
    let direct = b("is_direct");
    let known = b("sender_known");
    let bulky = hints.iter().any(|h| {
        matches!(
            h.as_str(),
            "list" | "bulk" | "auto" | "CATEGORY_PROMOTIONS" | "CATEGORY_SOCIAL"
        )
    });
    let bot = hints.iter().any(|h| h == "bot");

    // The content is data: an instruction aimed at the classifier is a red flag, not a request.
    if has_any(
        &text,
        &[
            "ignore previous",
            "ignore all previous",
            "mark as urgent",
            "mark this urgent",
            "system:",
        ],
    )
    .is_some()
    {
        return (
            "low",
            "message tries to instruct the classifier".into(),
            0.8,
        );
    }
    if let Some(w) = has_any(
        &text,
        &[
            "last chance",
            "% off",
            "sale ends",
            "act now",
            "limited time",
            "flash sale",
            "don't miss",
            "exclusive offer",
            "deal of",
            "free shipping",
            "promo code",
        ],
    ) {
        return ("low", format!("marketing promotion ({w})"), 0.95);
    }
    let incident = has_any(
        &text,
        &[
            "outage",
            "is down",
            "are down",
            "went down",
            "incident",
            "failing",
            "failed on main",
            "build failed",
            "deploy failed",
            "pipeline failed",
            "broken on",
            "sev1",
            "sev-1",
            "p0",
            "data loss",
            "security breach",
            "breach",
            "500 errors",
            "page is down",
            "customers can't",
            "customers cannot",
            "rollback",
        ],
    );
    if let Some(w) = incident {
        let quoted_newsletter =
            bulky && has_any(&text, &["newsletter", "digest", "recap", "this week in"]).is_some();
        if !quoted_newsletter {
            return ("urgent", format!("{w} needs attention"), 0.75);
        }
    }
    if bulky {
        let w = has_any(
            &text,
            &[
                "digest",
                "newsletter",
                "receipt",
                "order",
                "webinar",
                "update",
            ],
        )
        .unwrap_or_else(|| "bulk mail".into());
        return ("low", format!("{w}, nothing to do"), 0.9);
    }
    if let Some(w) = has_any(
        &text,
        &[
            "unsubscribe",
            "newsletter",
            "digest",
            "your receipt",
            "order confirmation",
            "has shipped",
            "was delivered",
            "weekly summary",
            "view in browser",
            "webinar",
            "new follower",
            "liked your",
            "commented on your post",
            "password changed",
            "sign-in from",
            "statement is ready",
        ],
    ) {
        return ("low", format!("{w}, nothing to do"), 0.85);
    }
    if bot {
        return ("low", "automated bot post".into(), 0.7);
    }
    let deadline = crate::urgency::rules::detect_deadline(&text, today, 1);
    let ask = has_any(
        &text,
        &[
            "can you",
            "could you",
            "please",
            "need you",
            "need your",
            "approve",
            "approval",
            "sign off",
            "sign-off",
            "review",
            "waiting on you",
            "waiting for you",
            "blocked on you",
            "your call",
            "decide",
            "decision",
            "confirm",
        ],
    );
    let meeting_change = has_any(
        &text,
        &[
            "moved to",
            "rescheduled",
            "reschedule",
            "cancelled",
            "canceled",
            "pushed to",
            "starts in",
        ],
    )
    .filter(|_| {
        has_any(
            &text,
            &[
                "today",
                "tomorrow",
                "this afternoon",
                "this morning",
                "in 10",
                "in 15",
                "in 30",
            ],
        )
        .is_some()
    });
    if let Some(w) = meeting_change {
        return ("urgent", format!("meeting {w}"), 0.7);
    }
    if let Some(d) = deadline
        && (ask.is_some() || direct || known)
    {
        return ("urgent", format!("asks for action {}", d.trim()), 0.8);
    }
    if let Some(a) = ask.as_ref()
        && direct
        && known
        && has_any(
            &text,
            &["blocked", "blocker", "asap", "today", "urgent", "now"],
        )
        .is_some()
    {
        return ("urgent", format!("{a} — blocking today"), 0.65);
    }
    match ask {
        Some(a) => (
            "normal",
            format!("asks to {a}, no deadline")
                .replace("asks to can you", "asks a question")
                .replace("asks to could you", "asks a favour"),
            0.6,
        ),
        None => ("normal", "ordinary message, can wait".into(), 0.55),
    }
}
