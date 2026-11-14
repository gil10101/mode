//! The vault's line-ending policy, in one place (04 §3.2, §3.3, §3.4, §9.4).
//!
//! | Case | Ending written |
//! |---|---|
//! | A file Mode creates (new note, capture, template, automation "Create note" / "Ask Chat", chat `create_note`, a conflict copy, a deleted file saved again) | `\n` ([`NEW_FILE`]; 04 §3.2 shows the file "as Mode writes it") |
//! | A write into a file that exists (editor save, chat `edit_note`, a restored revision) | the file's own ending, detected from its first line ending ([`of_existing`]; 04 §3.3), `\r\n` included: "the file's detected ending is used for every line Mode writes" (04 §9.4) |
//! | A frontmatter-only write (field controls, chat `set_fields`, touch-up, tag rename, overlay actions) | the block in the file's ending; the body bytes are copied verbatim (04 §3.4 "Body") |
//!
//! Text that arrives from outside the file — the editor's buffer (CodeMirror
//! hands over `\n`), a capture pasted from a Windows clipboard, a template
//! saved by another editor, a body proposed by a model, an old revision — is
//! converted to the target ending with [`normalize`] before it is written, so
//! a write never leaves a file with mixed endings and a reader never sees
//! them in a file Mode wrote. Mixed endings a foreign editor left in a file
//! survive every frontmatter-only write; a body write through Mode makes the
//! body uniform in the file's detected ending. Nothing normalizes on read.
//!
//! The writers apply the policy at their entry points in [`super::write`]
//! (`save`, `create_note`, `create_file` through
//! [`super::frontmatter::new_file`], `restore_revision`, the conflict copy)
//! and in [`super::capture`] for the capture grammar's input.

use std::borrow::Cow;

/// A file's line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Eol {
    #[default]
    Lf,
    CrLf,
}

impl Eol {
    /// The ending of the file's first line ending (04 §3.3); `Lf` without one.
    pub fn detect(text: &str) -> Eol {
        match text.find('\n') {
            Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => Eol::CrLf,
            _ => Eol::Lf,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
        }
    }
}

/// The ending of every file Mode creates.
pub const NEW_FILE: Eol = Eol::Lf;

/// The ending a write into an existing file uses: its own (04 §3.3, §9.4).
pub fn of_existing(file_text: &str) -> Eol {
    Eol::detect(file_text)
}

/// `text` with every line ending (`\r\n` or `\n`) written as `eol`. A lone
/// `\r` is not a line ending in Markdown files Mode reads and is left alone.
/// Borrowed when nothing changes.
pub fn normalize(text: &str, eol: Eol) -> Cow<'_, str> {
    let uniform = match eol {
        Eol::Lf => !text.contains("\r\n"),
        Eol::CrLf => text
            .bytes()
            .enumerate()
            .all(|(i, b)| b != b'\n' || (i > 0 && text.as_bytes()[i - 1] == b'\r')),
    };
    if uniform {
        return Cow::Borrowed(text);
    }
    let lf = text.replace("\r\n", "\n");
    match eol {
        Eol::Lf => Cow::Owned(lf),
        Eol::CrLf => Cow::Owned(lf.replace('\n', "\r\n")),
    }
}

/// Whether every line ending in `text` is `eol` (a text without one is).
pub fn is_uniform(text: &str, eol: Eol) -> bool {
    matches!(normalize(text, eol), Cow::Borrowed(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_reads_the_first_line_ending() {
        assert_eq!(Eol::detect("a\r\nb\nc"), Eol::CrLf);
        assert_eq!(Eol::detect("a\nb\r\nc"), Eol::Lf);
        assert_eq!(Eol::detect("no ending"), Eol::Lf);
        assert_eq!(Eol::detect("\n"), Eol::Lf);
        assert_eq!(of_existing("\u{FEFF}---\r\nid: x\r\n"), Eol::CrLf);
        assert_eq!(NEW_FILE, Eol::Lf);
    }

    #[test]
    fn normalize_converts_every_line_ending() {
        assert_eq!(normalize("a\r\nb\nc\r\n", Eol::Lf), "a\nb\nc\n");
        assert_eq!(normalize("a\r\nb\nc\r\n", Eol::CrLf), "a\r\nb\r\nc\r\n");
        assert_eq!(normalize("a\nb\n", Eol::CrLf), "a\r\nb\r\n");
        assert_eq!(normalize("\n\n", Eol::CrLf), "\r\n\r\n");
        assert_eq!(normalize("a\rb\n", Eol::Lf), "a\rb\n", "a lone CR stays");
        assert!(matches!(normalize("a\nb\n", Eol::Lf), Cow::Borrowed(_)));
        assert!(matches!(
            normalize("a\r\nb\r\n", Eol::CrLf),
            Cow::Borrowed(_)
        ));
        assert!(matches!(normalize("", Eol::CrLf), Cow::Borrowed(_)));
        assert!(is_uniform("one line", Eol::CrLf));
        assert!(!is_uniform("a\r\nb\n", Eol::CrLf));
        assert!(!is_uniform("a\r\nb\n", Eol::Lf));
    }
}
