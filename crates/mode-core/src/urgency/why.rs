//! Why lines (07 §4): one plain-text line per tier decision. Templates for
//! the deterministic tiers, `rule: <name>` for keyword rules, `set by you`
//! for overrides, `not scored (<reason>)` for the fallbacks, and the model's
//! own line cleaned and capped (≤ 120 stored, ≤ 80 displayed).

/// Stored cap (07 §3.4.4).
pub const STORED_MAX: usize = 120;
/// Display cap with an ellipsis (07 §3.4.4, §4).
pub const DISPLAY_MAX: usize = 80;

/// `Important person: {display_name}` + ` ({vip_note})` when set.
pub fn vip_contact(display_name: &str, vip_note: Option<&str>) -> String {
    match vip_note.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => clean(&format!("Important person: {display_name} ({n})")),
        None => clean(&format!("Important person: {display_name}")),
    }
}

/// `Important channel {channel_name}`.
pub fn vip_channel(channel_name: &str) -> String {
    clean(&format!("Important channel {channel_name}"))
}

/// `DM from {sender}` / `Group DM from {sender}`.
pub fn dm(sender_name: &str, group: bool) -> String {
    if group {
        clean(&format!("Group DM from {sender_name}"))
    } else {
        clean(&format!("DM from {sender_name}"))
    }
}

/// `@mention in {channel_name}`.
pub fn mention(channel_name: &str) -> String {
    clean(&format!("@mention in {channel_name}"))
}

/// `Sent only to you by {sender_name}`.
pub fn direct_email(sender_name: &str) -> String {
    clean(&format!("Sent only to you by {sender_name}"))
}

/// `rule: {name}`.
pub fn rule(name: &str) -> String {
    clean(&format!("rule: {name}"))
}

pub const SET_BY_YOU: &str = "set by you";
pub const NO_TEXT: &str = "no text to score";
pub const VIP_REMOVED: &str = "No longer important";
pub const INJECTION_FLAG: &str = "flagged: possible prompt injection";

/// The fallback reasons of 07 §3.4.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    Budget,
    LocalOnly,
    Unavailable,
    NoApiKey,
    Declined,
    Error,
}

impl Fallback {
    pub fn reason(self) -> &'static str {
        match self {
            Fallback::Budget => "budget",
            Fallback::LocalOnly => "local-only",
            Fallback::Unavailable => "unavailable",
            Fallback::NoApiKey => "no API key",
            Fallback::Declined => "declined",
            Fallback::Error => "error",
        }
    }

    /// `not scored ({reason})`.
    pub fn why(self) -> String {
        format!("not scored ({})", self.reason())
    }
}

/// Whether a stored why is a fallback (Re-score picks those up, 07 §3.4.8).
pub fn is_fallback(why: &str) -> bool {
    why.starts_with("not scored (") || why == NO_TEXT
}

/// Plain text: control characters and line breaks become spaces, runs of
/// whitespace collapse, capped at [`STORED_MAX`] characters.
pub fn clean(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(STORED_MAX).collect()
}

/// The display form: ≤ 80 characters, cut at a word boundary with "…".
pub fn display(why: &str) -> String {
    truncate_words(why, DISPLAY_MAX)
}

/// Cut at a word boundary to at most `max` characters (ellipsis included).
pub fn truncate_words(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let budget = max.saturating_sub(1);
    let cut: String = s.chars().take(budget).collect();
    let trimmed = match cut.rfind(char::is_whitespace) {
        Some(i) if i > budget / 2 => cut[..i].trim_end().to_string(),
        _ => cut.trim_end().to_string(),
    };
    format!("{trimmed}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_match_07_section_4() {
        assert_eq!(
            vip_contact("Ana Ruiz", Some("your manager")),
            "Important person: Ana Ruiz (your manager)"
        );
        assert_eq!(
            vip_contact("Ana Ruiz", Some("  ")),
            "Important person: Ana Ruiz"
        );
        assert_eq!(vip_channel("#incidents"), "Important channel #incidents");
        assert_eq!(dm("Tim Baker", false), "DM from Tim Baker");
        assert_eq!(dm("Tim Baker", true), "Group DM from Tim Baker");
        assert_eq!(mention("#release"), "@mention in #release");
        assert_eq!(
            direct_email("Léa Marchand"),
            "Sent only to you by Léa Marchand"
        );
        assert_eq!(rule("Mentions a deadline"), "rule: Mentions a deadline");
        assert_eq!(Fallback::Budget.why(), "not scored (budget)");
        assert_eq!(Fallback::NoApiKey.why(), "not scored (no API key)");
        assert!(is_fallback("not scored (local-only)"));
        assert!(!is_fallback("asks for sign-off"));
    }

    #[test]
    fn cleaning_and_display() {
        assert_eq!(clean("a\nb\t\u{7}c  d"), "a b c d");
        assert_eq!(clean(&"x".repeat(200)).chars().count(), STORED_MAX);
        let long = "asks for the signed contract and the invoice by Friday so legal can file it before the quarter closes";
        let d = display(long);
        assert!(d.chars().count() <= DISPLAY_MAX);
        assert!(d.ends_with('…'));
        assert_eq!(display("short"), "short");
    }
}
