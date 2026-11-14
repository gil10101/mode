//! The note body (04 §4): task lines and their four markers, glyphs at
//! the two levels, counts and progress, title derivation (§2.4), the
//! excerpt and word count caches (09 §5.1), and the task-line interactions
//! of §4.3. Every edit changes the marker byte(s) of one line and nothing
//! else, so untouched ranges stay byte-identical (§9.4).
//!
//! Task lines follow 04 §4.1:
//! `^(\s*)([-*+]|\d+[.)])[ \t]+\[( |/|!|x|X)\][ \t]+(.*)$`, outside fenced
//! code blocks (a fence renders its content literally in GFM).

use std::ops::Range;

/// The four task-line markers on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Marker {
    /// `[ ]`
    Todo,
    /// `[/]`
    Doing,
    /// `[!]` urgent, not yet started
    Urgent,
    /// `[x]` / `[X]`
    Done,
}

impl Marker {
    pub fn from_char(c: char) -> Option<Marker> {
        Some(match c {
            ' ' => Marker::Todo,
            '/' => Marker::Doing,
            '!' => Marker::Urgent,
            'x' | 'X' => Marker::Done,
            _ => return None,
        })
    }

    /// The character Mode writes (lowercase `x`).
    pub fn char(self) -> char {
        match self {
            Marker::Todo => ' ',
            Marker::Doing => '/',
            Marker::Urgent => '!',
            Marker::Done => 'x',
        }
    }

    /// The task-line glyph (04 §4.2).
    pub fn glyph(self) -> &'static str {
        match self {
            Marker::Todo => "\u{25CB}",
            Marker::Doing => "\u{25D1}",
            Marker::Urgent => "\u{2731}",
            Marker::Done => "\u{2713}",
        }
    }

    /// Click on the glyph: `[ ]` → `[/]` → `[x]` → `[ ]`; `[!]` → `[x]` (§4.3).
    pub fn cycled(self) -> Marker {
        match self {
            Marker::Todo => Marker::Doing,
            Marker::Doing => Marker::Done,
            Marker::Done => Marker::Todo,
            Marker::Urgent => Marker::Done,
        }
    }

    /// Space / ⌘Enter: any open marker → `[x]`; `[x]` → `[ ]` (§4.3).
    pub fn toggled(self) -> Marker {
        match self {
            Marker::Done => Marker::Todo,
            _ => Marker::Done,
        }
    }

    /// "Convert to note": the new note's `(status, urgent)` (§4.3).
    pub fn note_fields(self) -> (&'static str, bool) {
        match self {
            Marker::Todo => ("todo", false),
            Marker::Doing => ("doing", false),
            Marker::Urgent => ("todo", true),
            Marker::Done => ("done", false),
        }
    }
}

/// One task line of a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLine {
    /// 0-based line number within the body.
    pub line: usize,
    pub marker: Marker,
    /// Byte offset of the marker character within the body.
    pub marker_at: usize,
    /// Byte range of the text after the marker (line ending excluded).
    pub text: Range<usize>,
    pub indent: String,
}

/// `(start, end_without_eol)` of every line.
fn line_spans(body: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    let b = body.as_bytes();
    while pos < b.len() {
        let nl = body[pos..].find('\n').map(|i| pos + i);
        let (end, next) = match nl {
            Some(i) => (
                if i > pos && b[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                },
                i + 1,
            ),
            None => (b.len(), b.len()),
        };
        out.push((pos, end));
        pos = next;
    }
    out
}

/// Tracks fenced code blocks (``` / ~~~, three or more, ≤ 3 spaces indent).
#[derive(Default)]
struct Fences {
    open: Option<(char, usize)>,
}

impl Fences {
    /// Feed a line; true when the line is inside a fence or is a fence line.
    fn code(&mut self, line: &str) -> bool {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let t = &line[indent.min(line.len())..];
        let fence = if indent <= 3 {
            let c = t.chars().next();
            match c {
                Some(ch @ ('`' | '~')) => {
                    let n = t.chars().take_while(|x| *x == ch).count();
                    (n >= 3).then_some((ch, n))
                }
                _ => None,
            }
        } else {
            None
        };
        match (self.open, fence) {
            (None, Some(f)) => {
                self.open = Some(f);
                true
            }
            (Some((c, n)), Some((c2, n2)))
                if c == c2 && n2 >= n && t.trim_start_matches(c).trim().is_empty() =>
            {
                self.open = None;
                true
            }
            (Some(_), _) => true,
            (None, None) => false,
        }
    }
}

/// Match §4.1 on one line (without its ending): `(marker, marker offset, text start)`.
fn match_task(line: &str) -> Option<(Marker, usize, usize)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    // Bullet or ordered marker.
    if i < b.len() && matches!(b[i], b'-' | b'*' | b'+') {
        i += 1;
    } else {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || i - start > 9 || i >= b.len() || !matches!(b[i], b'.' | b')') {
            return None;
        }
        i += 1;
    }
    let ws = i;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i == ws || i + 3 > b.len() || b[i] != b'[' || b[i + 2] != b']' {
        return None;
    }
    let marker = Marker::from_char(b[i + 1] as char)?;
    let marker_at = i + 1;
    i += 3;
    let ws = i;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i == ws {
        return None;
    }
    Some((marker, marker_at, i))
}

/// Every task line of `body`, outside fenced code.
pub fn task_lines(body: &str) -> Vec<TaskLine> {
    let mut fences = Fences::default();
    let mut out = Vec::new();
    for (n, (s, e)) in line_spans(body).into_iter().enumerate() {
        let line = &body[s..e];
        if fences.code(line) {
            continue;
        }
        if let Some((marker, at, text)) = match_task(line) {
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            out.push(TaskLine {
                line: n,
                marker,
                marker_at: s + at,
                text: s + text..e,
                indent,
            });
        }
    }
    out
}

/// `(task_total, task_done)` (04 §4.4).
pub fn counts(body: &str) -> (i64, i64) {
    let lines = task_lines(body);
    let done = lines.iter().filter(|t| t.marker == Marker::Done).count();
    (lines.len() as i64, done as i64)
}

/// Replace the marker of task line `line` (0-based body line) with `f(marker)`.
/// `None` when that line is not a task line. Only the one marker byte changes.
pub fn edit_marker(body: &str, line: usize, f: impl FnOnce(Marker) -> Marker) -> Option<String> {
    let t = task_lines(body).into_iter().find(|t| t.line == line)?;
    let new = f(t.marker);
    let mut out = String::with_capacity(body.len());
    out.push_str(&body[..t.marker_at]);
    out.push(new.char());
    out.push_str(&body[t.marker_at + 1..]);
    Some(out)
}

/// Click on a task line's glyph (§4.3).
pub fn cycle_line(body: &str, line: usize) -> Option<String> {
    edit_marker(body, line, Marker::cycled)
}

/// Space / ⌘Enter on a task line (§4.3).
pub fn toggle_line(body: &str, line: usize) -> Option<String> {
    edit_marker(body, line, Marker::toggled)
}

/// Context menu: set a marker directly (§4.3).
pub fn set_line(body: &str, line: usize, marker: Marker) -> Option<String> {
    edit_marker(body, line, |_| marker)
}

/// The first ATX H1 (`# Title`) outside fenced code: up to three spaces of
/// indentation, `#` followed by a space, a tab or the line end; a closing
/// `#` run is dropped (CommonMark).
pub fn first_h1(body: &str) -> Option<String> {
    let mut fences = Fences::default();
    for (s, e) in line_spans(body) {
        let line = &body[s..e];
        if fences.code(line) {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 3 {
            continue;
        }
        let t = &line[indent..];
        let Some(rest) = t.strip_prefix('#') else {
            continue;
        };
        if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
            continue;
        }
        let mut title = rest.trim();
        // A closing sequence of `#` preceded by a space.
        let trimmed = title.trim_end_matches('#');
        if trimmed.len() < title.len() && (trimmed.is_empty() || trimmed.ends_with([' ', '\t'])) {
            title = trimmed.trim_end();
        }
        if title.is_empty() {
            continue;
        }
        return Some(title.to_string());
    }
    None
}

/// Strip inline Markdown from one line for the excerpt.
fn strip_inline(line: &str) -> String {
    let mut s = line.trim().to_string();
    // Block prefixes: headings, quotes, list bullets, task markers.
    loop {
        let before = s.clone();
        s = s.trim_start_matches('#').trim_start().to_string();
        s = s.trim_start_matches('>').trim_start().to_string();
        if let Some((_, _, text)) = match_task(&s) {
            s = s[text..].to_string();
        } else if let Some(rest) = s
            .strip_prefix("- ")
            .or_else(|| s.strip_prefix("* "))
            .or_else(|| s.strip_prefix("+ "))
        {
            s = rest.to_string();
        }
        if s == before {
            break;
        }
    }
    // Links and images: keep the text.
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c == '[' || (c == '!' && chars.get(i + 1) == Some(&'[')))
            && !(c == '[' && chars.get(i + 1) == Some(&'['))
        {
            let open = if c == '!' { i + 1 } else { i };
            if let Some(close) = chars[open..]
                .iter()
                .position(|x| *x == ']')
                .map(|p| p + open)
                && chars.get(close + 1) == Some(&'(')
                && let Some(end) = chars[close..]
                    .iter()
                    .position(|x| *x == ')')
                    .map(|p| p + close)
            {
                out.extend(&chars[open + 1..close]);
                i = end + 1;
                continue;
            }
        }
        if c == '[' && chars.get(i + 1) == Some(&'[') {
            // [[Wikilink|alias]] → alias or target.
            if let Some(end) = (i + 2..chars.len().saturating_sub(1))
                .find(|j| chars[*j] == ']' && chars[*j + 1] == ']')
            {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str(inner.rsplit('|').next().unwrap_or(&inner));
                i = end + 2;
                continue;
            }
        }
        if matches!(c, '*' | '_' | '`' | '~') {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first 200 characters of the body with Markdown stripped (09 §5.1);
/// the title H1, fenced code and HTML lines are skipped.
pub fn excerpt(body: &str) -> String {
    let mut fences = Fences::default();
    let mut skipped_title = false;
    let mut parts: Vec<String> = Vec::new();
    let mut len = 0;
    for (s, e) in line_spans(body) {
        let line = &body[s..e];
        if fences.code(line) {
            continue;
        }
        let t = line.trim();
        if t.is_empty()
            || t.starts_with('<')
            || t.chars().all(|c| matches!(c, '-' | '*' | '_' | '=' | ' '))
        {
            continue;
        }
        if !skipped_title && (t == "#" || t.starts_with("# ")) {
            skipped_title = true;
            continue;
        }
        let stripped = strip_inline(t);
        if stripped.is_empty() {
            continue;
        }
        len += stripped.chars().count() + 1;
        parts.push(stripped);
        if len > 200 {
            break;
        }
    }
    parts.join(" ").chars().take(200).collect()
}

/// Whitespace-separated words of the body.
pub fn word_count(body: &str) -> i64 {
    body.split_whitespace().count() as i64
}

/// The note-level glyph (04 §4.2): the status glyph; `✱` is overlaid by the
/// renderer whenever `urgent`.
pub fn note_glyph<'a>(status: &str, statuses: &'a [crate::settings::StatusDef]) -> &'a str {
    statuses
        .iter()
        .find(|s| s.slug == status)
        .and_then(|s| s.glyph.as_deref())
        .unwrap_or(super::fields::UNKNOWN_GLYPH)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "# Buy the things\n\nBefore the weekend.\n\n- [/] Milk, eggs\n- [ ] Call the plumber\n- [!] Pay the deposit\n- [x] Book the van\n  * [X] nested done\n1. [ ] numbered\n2) [x] paren\n- [-] not a task\n- [x]Milk\n- [ ]\n```\n- [ ] in code\n```\n\t+ [ ] tab indent\n";

    #[test]
    fn task_lines_follow_the_syntax() {
        let lines = task_lines(BODY);
        let texts: Vec<&str> = lines.iter().map(|t| &BODY[t.text.clone()]).collect();
        assert_eq!(
            texts,
            vec![
                "Milk, eggs",
                "Call the plumber",
                "Pay the deposit",
                "Book the van",
                "nested done",
                "numbered",
                "paren",
                "tab indent"
            ]
        );
        assert_eq!(counts(BODY), (8, 3));
        assert_eq!(lines[2].marker, Marker::Urgent);
        assert_eq!(lines[4].indent, "  ");
    }

    #[test]
    fn interactions_change_one_byte() {
        let b = "a\n- [ ] one\r\n- [!] two\n- [X] three";
        let c = cycle_line(b, 1).unwrap();
        assert_eq!(c, "a\n- [/] one\r\n- [!] two\n- [X] three");
        assert_eq!(
            cycle_line(&c, 1).unwrap(),
            "a\n- [x] one\r\n- [!] two\n- [X] three"
        );
        assert_eq!(
            cycle_line(b, 2).unwrap(),
            "a\n- [ ] one\r\n- [x] two\n- [X] three"
        );
        assert_eq!(
            toggle_line(b, 3).unwrap(),
            "a\n- [ ] one\r\n- [!] two\n- [ ] three"
        );
        assert_eq!(
            toggle_line(b, 1).unwrap(),
            "a\n- [x] one\r\n- [!] two\n- [X] three"
        );
        assert_eq!(
            set_line(b, 1, Marker::Urgent).unwrap(),
            "a\n- [!] one\r\n- [!] two\n- [X] three"
        );
        assert_eq!(cycle_line(b, 0), None);
        assert_eq!(Marker::Urgent.note_fields(), ("todo", true));
    }

    #[test]
    fn title_excerpt_words() {
        assert_eq!(first_h1(BODY).as_deref(), Some("Buy the things"));
        assert_eq!(
            first_h1("```\n# not\n```\n## h2\n   # Real # \n").as_deref(),
            Some("Real")
        );
        assert_eq!(first_h1("#hashtag\n"), None);
        assert_eq!(first_h1("    # indented code\n"), None);
        let ex = excerpt(
            "# T\n\nSee [the doc](https://x) and **bold** `code` [[Other|alias]].\n- [ ] task\n",
        );
        assert_eq!(ex, "See the doc and bold code alias. task");
        assert!(excerpt(&"word ".repeat(100)).chars().count() <= 200);
        assert_eq!(word_count("a b\n c"), 3);
    }
}
