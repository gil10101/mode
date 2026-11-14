//! The wake phrase (XC-38, ADR-0061): an utterance that starts with "Hey
//! Mode", "Hi Mode", "OK Mode" or plain "Mode" is a request to Chat; the rest
//! of it is the request. Matched on the transcript, never by a hot-mic
//! detector: nothing listens unless the dictation key is held (08 §14).
//!
//! whisper.cpp mishears the phrase in measured ways (ADR-0064, 02 §5.7):
//! "Hi Mode" is most often "High mode", and "Mode" after a greeting can
//! come out as "Mo", "Moe", "Mood", "Maud" or "Mod". Those forms count
//! after a greeting ("High" only before "Mode" or "Mo"); alone at the start
//! only "Mode" does, so "Mod, …" or "Mo, …" stays dictation.

use std::sync::OnceLock;

use regex::Regex;

/// What an utterance is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Utterance {
    /// Words to type into the focused field (or capture as a note).
    Dictation(String),
    /// A request to Chat: the words after the wake phrase (may be empty).
    Command(String),
}

fn pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?is)^\s*(?:",
            r"(?:hey|hi|ok|okay|k|a\s?k)[\s,.\-]*(?:mode|mod|moe|mood|maud|mo)",
            r"|high[\s,.\-]*(?:mode|mo)",
            r"|mode",
            r")\b[\s,.:!]*(.*)$"
        ))
        .expect("the wake pattern compiles")
    })
}

/// Split a transcript into dictation or a Chat request.
pub fn classify(text: &str) -> Utterance {
    match pattern().captures(text) {
        Some(c) => Utterance::Command(
            c.get(1)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default(),
        ),
        None => Utterance::Dictation(text.trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(s: &str) -> String {
        match classify(s) {
            Utterance::Command(c) => c,
            other => panic!("{s:?} was {other:?}"),
        }
    }

    #[test]
    fn wake_phrases_become_requests() {
        assert_eq!(command("Hey Mode, summarize this."), "summarize this.");
        assert_eq!(
            command("hi mode reply to this email"),
            "reply to this email"
        );
        assert_eq!(
            command("Mode tell api-refactor to run the tests"),
            "tell api-refactor to run the tests"
        );
        assert_eq!(command("OK, Mode: what's next?"), "what's next?");
        assert_eq!(command("  Okay mode!  draft a reply"), "draft a reply");
        assert_eq!(command("mode"), "");
        assert_eq!(command("Hey Mode."), "");
    }

    /// How whisper.cpp (base.en) heard the wake phrase on the 2026-11-09
    /// measurement (eleven macOS voices, two rates; ADR-0064).
    #[test]
    fn misheard_wake_phrases_become_requests() {
        assert_eq!(command("High mode."), "");
        assert_eq!(
            command("High mode reply to this email."),
            "reply to this email."
        );
        assert_eq!(command("High mode, what's next?"), "what's next?");
        assert_eq!(
            command("High Mo reply to this email."),
            "reply to this email."
        );
        assert_eq!(command("Hey Mo, what's on my list?"), "what's on my list?");
        assert_eq!(
            command("Hey, Mo, show me my tool list."),
            "show me my tool list."
        );
        assert_eq!(command("Hey Moe, what's next"), "what's next");
        assert_eq!(
            command("Hey mood, what's on my list today?"),
            "what's on my list today?"
        );
        assert_eq!(
            command("Hey Maud, show me my tourlist."),
            "show me my tourlist."
        );
        assert_eq!(command("Hey Mod, what's next"), "what's next");
        assert_eq!(command("K-Mode."), "");
        assert_eq!(
            command("A K mode summarized this thread."),
            "summarized this thread."
        );
        assert_eq!(
            command("Ok mode summarize this thread."),
            "summarize this thread."
        );
        assert_eq!(command("Hey. Mode, what's next?"), "what's next?");
    }

    /// Misheard starts that are ordinary sentences stay dictation (ADR-0064).
    #[test]
    fn ordinary_misheard_starts_stay_dictation() {
        for s in [
            "Hey, what? Show me what to do for today!",
            "Hey, load. Show me my to do for today.",
            "Hey world.",
            "Hey, Moon.",
            "Hey, Rose, show me my two-four stay.",
            "Okay, well, summarize this thread!",
            "Okay, hold.",
            "Hooky mode summarize this thread.",
            "Team mode.",
            "Pay mode.",
            "Highmole.",
            "High-low.",
            "I MOLD!",
            "remote, reply to Priya.",
            "road, reply to Priya.",
            "Mod, reply to Priya.",
            "Mo, reply to Priya.",
            "Hey modern art is loud",
            "Hey more of that",
            "High mood today",
        ] {
            assert_eq!(
                classify(s),
                Utterance::Dictation(s.trim().to_string()),
                "{s:?}"
            );
        }
    }

    #[test]
    fn ordinary_sentences_stay_dictation() {
        for s in [
            "the mode of the data",
            "Model this as a queue",
            "Modest gains this week",
            "hey there, mode is on",
            "Call Priya about the retry policy",
            "",
        ] {
            assert_eq!(
                classify(s),
                Utterance::Dictation(s.trim().to_string()),
                "{s:?}"
            );
        }
    }
}
