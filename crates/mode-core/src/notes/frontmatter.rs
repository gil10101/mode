//! The note file format of 04 §3: frontmatter detection, a YAML-subset
//! reader for the top-level mapping, the per-key rules of §3.1 / §3.3, and
//! the writer of §3.4 that regenerates only the block between the
//! delimiters and copies everything from the closing delimiter to EOF
//! verbatim (§9.4).
//!
//! The reader is hand-written for the small schema (no YAML crate): it
//! splits the block into top-level entries, keeps every entry's original
//! text (unknown keys are re-emitted byte for byte), parses the values of
//! Mode's eight keys (plain / quoted scalars, flow and block sequences,
//! block scalars) and rejects what a YAML 1.2 parser would reject in a
//! mapping of this shape (stray top-level lines, tab indentation, duplicate
//! keys, unbalanced flow collections or quotes, `: ` inside a plain scalar).
//! A block it cannot read is reported as invalid and never rewritten.

use std::collections::BTreeSet;

use super::fields::{self, LocalTz, WhenRaw};

/// The file's line ending; the policy for which one a write uses is
/// [`super::eol`].
pub use super::eol::Eol;

pub const BOM: &str = "\u{FEFF}";

/// Mode's keys in the order Mode writes them (04 §3.4).
pub const MODE_KEYS: &[&str] = &[
    "id", "status", "urgent", "when", "tags", "created", "updated", "rev",
];

/// A parsed YAML value, as far as Mode needs to see it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Scalar(String),
    Seq(Vec<Value>),
    /// A mapping, an alias, a tagged value: valid YAML Mode does not interpret.
    Other,
}

/// One top-level `key: value` entry with its original text.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    /// The key line and its continuation lines, verbatim (line endings included).
    pub raw: String,
    pub value: Value,
}

/// The frontmatter block as found.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// No block: the body is the whole file (after a BOM).
    Absent,
    /// A readable block. `inner` is the byte range between the opening
    /// delimiter line and the closing one; `close` is where the closing
    /// delimiter line starts.
    Valid {
        inner_start: usize,
        close: usize,
        body_start: usize,
        entries: Vec<Entry>,
    },
    /// A block that is not valid YAML (04 §3.3): the whole file is the body.
    Invalid { reason: String },
}

/// A note file split into BOM, line ending, frontmatter and body.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub text: String,
    pub bom: bool,
    pub eol: Eol,
    pub block: Block,
}

impl Document {
    pub fn parse(text: &str) -> Document {
        let bom = text.starts_with(BOM);
        let eol = Eol::detect(text);
        let block = parse_block(text, if bom { BOM.len() } else { 0 });
        Document {
            text: text.to_string(),
            bom,
            eol,
            block,
        }
    }

    /// The body: after the closing delimiter line, or the whole file (BOM
    /// stripped) when there is no readable block.
    pub fn body(&self) -> &str {
        match &self.block {
            Block::Valid { body_start, .. } => &self.text[*body_start..],
            _ => &self.text[if self.bom { BOM.len() } else { 0 }..],
        }
    }

    pub fn entries(&self) -> &[Entry] {
        match &self.block {
            Block::Valid { entries, .. } => entries,
            _ => &[],
        }
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries().iter().find(|e| e.key == key)
    }

    pub fn has_block(&self) -> bool {
        matches!(self.block, Block::Valid { .. })
    }

    pub fn error(&self) -> Option<&str> {
        match &self.block {
            Block::Invalid { reason } => Some(reason),
            _ => None,
        }
    }

    /// Mode's fields per 04 §3.1 with the defaults of §3.3.
    pub fn fields(&self, tz: &dyn LocalTz) -> Fields {
        Fields::read(self, tz)
    }
}

/// Line iterator yielding `(start, end_without_eol, end_with_eol)`.
fn lines(text: &str, from: usize) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let bytes = text.as_bytes();
    let mut pos = from;
    std::iter::from_fn(move || {
        if pos >= bytes.len() {
            return None;
        }
        let start = pos;
        let nl = text[start..].find('\n').map(|i| start + i);
        let (end, next) = match nl {
            Some(i) => {
                let end = if i > start && bytes[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                (end, i + 1)
            }
            None => (bytes.len(), bytes.len()),
        };
        pos = next;
        Some((start, end, next))
    })
}

fn is_delim(line: &str, closing: bool) -> bool {
    let t = line.trim_end_matches([' ', '\t']);
    t == "---" || (closing && t == "...")
}

fn parse_block(text: &str, from: usize) -> Block {
    let mut it = lines(text, from);
    let Some((s, e, inner_start)) = it.next() else {
        return Block::Absent;
    };
    if !is_delim(&text[s..e], false) || inner_start == e {
        // No `---` first line, or `---` without a line ending (an empty file body).
        return Block::Absent;
    }
    let mut close = None;
    for (s, e, next) in it {
        if is_delim(&text[s..e], true) {
            close = Some((s, next));
            break;
        }
    }
    let Some((close, body_start)) = close else {
        // An unclosed `---` is a thematic break, not frontmatter.
        return Block::Absent;
    };
    match parse_entries(&text[inner_start..close]) {
        Ok(entries) => Block::Valid {
            inner_start,
            close,
            body_start,
            entries,
        },
        Err(reason) => Block::Invalid { reason },
    }
}

/// Byte index of the `:` that ends a top-level key on `line`, and the key.
fn split_key(line: &str) -> Option<(String, usize)> {
    let first = line.chars().next()?;
    if first == '"' || first == '\'' {
        let q = first;
        let mut i = 1;
        let b = line.as_bytes();
        let mut key = String::new();
        while i < b.len() {
            let c = line[i..].chars().next()?;
            if c == q {
                if q == '\'' && line[i + 1..].starts_with('\'') {
                    key.push('\'');
                    i += 2;
                    continue;
                }
                let after = &line[i + 1..];
                let after_trim = after.trim_start_matches([' ', '\t']);
                if let Some(rest) = after_trim.strip_prefix(':')
                    && (rest.is_empty() || rest.starts_with([' ', '\t']))
                {
                    let colon = line.len() - after_trim.len();
                    return Some((key, colon));
                }
                return None;
            }
            if q == '"' && c == '\\' {
                let n = line[i + 1..].chars().next()?;
                key.push(n);
                i += 1 + n.len_utf8();
                continue;
            }
            key.push(c);
            i += c.len_utf8();
        }
        return None;
    }
    if first.is_whitespace() || "-?:,[]{}#&*!|>'\"%@`".contains(first) {
        // `-` starts a sequence item; `? ` complex keys are not read.
        if first != '-' || line.starts_with("- ") || line == "-" {
            return None;
        }
    }
    let b = line.as_bytes();
    for (i, c) in b.iter().enumerate() {
        if *c == b':' && (i + 1 == b.len() || b[i + 1] == b' ' || b[i + 1] == b'\t') {
            let key = line[..i].trim_end().to_string();
            if key.is_empty() || key.contains(" #") {
                return None;
            }
            return Some((key, i));
        }
        if *c == b'#' && i > 0 && (b[i - 1] == b' ' || b[i - 1] == b'\t') {
            return None;
        }
    }
    None
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim_start_matches([' ', '\t']);
    t.is_empty() || t.starts_with('#')
}

/// Split the inside of a block into top-level entries and validate them.
pub fn parse_entries(inner: &str) -> Result<Vec<Entry>, String> {
    struct Pending {
        key: String,
        inline: String,
        start: usize,
        // End of the last non-blank line (with its line ending).
        end: usize,
        cont: Vec<String>,
        seq_at_col0: bool,
    }
    let mut out: Vec<Entry> = Vec::new();
    let mut cur: Option<Pending> = None;
    let mut keys: BTreeSet<String> = BTreeSet::new();

    let finish = |p: Pending, out: &mut Vec<Entry>| -> Result<(), String> {
        let value = parse_value(&p.inline, &p.cont, p.seq_at_col0)
            .map_err(|e| format!("{}: {e}", p.key))?;
        out.push(Entry {
            key: p.key,
            raw: inner[p.start..p.end].to_string(),
            value,
        });
        Ok(())
    };

    for (s, e, next) in lines(inner, 0) {
        let line = &inner[s..e];
        if line.starts_with('\t') {
            return Err("tab indentation".into());
        }
        if line.trim().is_empty() {
            if let Some(p) = cur.as_mut() {
                p.cont.push(String::new());
            }
            continue;
        }
        let indented = line.starts_with(' ');
        if !indented && line.starts_with('#') {
            // A comment at column 0 ends nothing but belongs to no entry.
            continue;
        }
        if indented {
            match cur.as_mut() {
                Some(p) => {
                    p.cont.push(line.to_string());
                    p.end = next;
                }
                None => return Err("indented line before the first key".into()),
            }
            continue;
        }
        if line == "-" || line.starts_with("- ") {
            match cur.as_mut() {
                Some(p) if p.inline.trim().is_empty() || is_comment_only(&p.inline) => {
                    p.seq_at_col0 = true;
                    p.cont.push(line.to_string());
                    p.end = next;
                    continue;
                }
                _ => return Err("a sequence item outside a key".into()),
            }
        }
        let Some((key, colon)) = split_key(line) else {
            return Err(format!("line {line:?} is not a key"));
        };
        if let Some(p) = cur.take() {
            finish(p, &mut out)?;
        }
        if !keys.insert(key.clone()) {
            return Err(format!("duplicate key {key:?}"));
        }
        cur = Some(Pending {
            key,
            inline: line[colon + 1..].to_string(),
            start: s,
            end: next,
            cont: Vec::new(),
            seq_at_col0: false,
        });
    }
    if let Some(p) = cur.take() {
        finish(p, &mut out)?;
    }
    Ok(out)
}

fn is_comment_only(s: &str) -> bool {
    s.trim_start().starts_with('#')
}

/// Remove a trailing ` # comment` outside quotes and brackets.
fn strip_comment(s: &str) -> &str {
    let b = s.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if q == b'"' && c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
            }
            None => {
                if (c == b'"' || c == b'\'')
                    && (i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'[' | b'{' | b','))
                {
                    quote = Some(c);
                } else if c == b'#' && (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t') {
                    return &s[..i];
                }
            }
        }
        i += 1;
    }
    s
}

fn parse_value(inline: &str, cont: &[String], seq_at_col0: bool) -> Result<Value, String> {
    // Trailing blank lines carry nothing.
    let mut cont: Vec<&str> = cont.iter().map(String::as_str).collect();
    while cont.last().is_some_and(|l| l.trim().is_empty()) {
        cont.pop();
    }
    let v = strip_comment(inline).trim();
    if v.is_empty() {
        let items: Vec<&str> = cont
            .iter()
            .copied()
            .filter(|l| !is_blank_or_comment(l))
            .collect();
        if items.is_empty() {
            return Ok(Value::Null);
        }
        let first = items[0].trim_start();
        if first == "-" || first.starts_with("- ") {
            return parse_block_seq(&cont, seq_at_col0);
        }
        if seq_at_col0 {
            return Err("mixed sequence and mapping".into());
        }
        if split_key(first).is_some() {
            // A nested mapping: valid, not interpreted (its lines must stay indented).
            return Ok(Value::Other);
        }
        // A multi-line plain scalar.
        let text = items.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        return plain_scalar(&text).map(Value::Scalar);
    }
    if seq_at_col0 {
        return Err("a sequence item after an inline value".into());
    }
    let first = v.as_bytes()[0];
    match first {
        b'[' | b'{' => {
            let mut all = String::from(inline.trim_start());
            for l in &cont {
                all.push('\n');
                all.push_str(l);
            }
            let (end, val) = parse_flow(&all, 0)?;
            let rest = strip_comment(&all[end..]).trim();
            if !rest.is_empty() {
                return Err(format!("text after a flow collection: {rest:?}"));
            }
            Ok(val)
        }
        b'"' | b'\'' => {
            let mut all = String::from(inline.trim_start());
            for l in &cont {
                all.push('\n');
                all.push_str(l.trim());
            }
            let (end, s) = parse_quoted(&all, 0)?;
            let rest = strip_comment(&all[end..]).trim();
            if !rest.is_empty() {
                return Err(format!("text after a quoted scalar: {rest:?}"));
            }
            Ok(Value::Scalar(s))
        }
        b'|' | b'>' => {
            let header = v;
            if !header[1..]
                .chars()
                .all(|c| c == '-' || c == '+' || c.is_ascii_digit())
            {
                return Err(format!("bad block scalar header {header:?}"));
            }
            let indent = cont
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.len() - l.trim_start_matches(' ').len())
                .min()
                .unwrap_or(0);
            let lines: Vec<String> = cont
                .iter()
                .map(|l| {
                    if l.len() >= indent {
                        l[indent..].to_string()
                    } else {
                        String::new()
                    }
                })
                .collect();
            let joined = if first == b'|' {
                lines.join("\n")
            } else {
                lines.join(" ")
            };
            Ok(Value::Scalar(joined.trim().to_string()))
        }
        b'&' | b'*' | b'!' => Ok(Value::Other),
        b'@' | b'`' => Err(format!("{:?} cannot start a plain scalar", first as char)),
        _ => {
            let mut text = v.to_string();
            for l in cont.iter().filter(|l| !is_blank_or_comment(l)) {
                text.push(' ');
                text.push_str(strip_comment(l).trim());
            }
            plain_scalar(&text).map(Value::Scalar)
        }
    }
}

fn plain_scalar(text: &str) -> Result<String, String> {
    if text.contains(": ") || text.ends_with(':') {
        return Err("': ' inside a plain scalar".into());
    }
    if text.starts_with("- ") {
        return Err("a sequence inside a scalar".into());
    }
    Ok(text.trim().to_string())
}

fn parse_block_seq(cont: &[&str], _col0: bool) -> Result<Value, String> {
    let mut items: Vec<Value> = Vec::new();
    let mut item_indent: Option<usize> = None;
    let mut pending: Option<String> = None;
    let flush = |pending: &mut Option<String>, items: &mut Vec<Value>| -> Result<(), String> {
        if let Some(text) = pending.take() {
            items.push(parse_item(&text)?);
        }
        Ok(())
    };
    for l in cont {
        if is_blank_or_comment(l) {
            continue;
        }
        let indent = l.len() - l.trim_start_matches(' ').len();
        let t = l.trim_start();
        let is_item = t == "-" || t.starts_with("- ");
        match item_indent {
            None if is_item => item_indent = Some(indent),
            None => return Err("expected a sequence item".into()),
            Some(ii) if is_item && indent == ii => {}
            Some(ii) if indent > ii => {
                // Continuation of the current item (a folded plain scalar or a nested value).
                match pending.as_mut() {
                    Some(p) => {
                        p.push('\n');
                        p.push_str(l);
                        continue;
                    }
                    None => return Err("continuation without an item".into()),
                }
            }
            Some(_) => return Err("bad sequence indentation".into()),
        }
        flush(&mut pending, &mut items)?;
        pending = Some(t.strip_prefix('-').unwrap_or("").to_string());
    }
    flush(&mut pending, &mut items)?;
    Ok(Value::Seq(items))
}

fn parse_item(text: &str) -> Result<Value, String> {
    let mut lines = text.split('\n');
    let first = lines.next().unwrap_or("");
    let cont: Vec<String> = lines.map(str::to_string).collect();
    let v = strip_comment(first).trim();
    if v.is_empty() && cont.iter().all(|l| l.trim().is_empty()) {
        return Ok(Value::Null);
    }
    if v.starts_with("- ") || v == "-" {
        return Ok(Value::Other);
    }
    if split_key(v).is_some() {
        return Ok(Value::Other);
    }
    parse_value(first, &cont, false)
}

/// Parse a quoted scalar starting at `at`; returns the index after the closing quote.
fn parse_quoted(s: &str, at: usize) -> Result<(usize, String), String> {
    let q = s.as_bytes()[at];
    let mut out = String::new();
    let mut i = at + 1;
    while i < s.len() {
        let c = s[i..].chars().next().ok_or("bad text")?;
        if c as u32 == u32::from(q) {
            if q == b'\'' && s[i + 1..].starts_with('\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            return Ok((i + 1, out));
        }
        if q == b'"' && c == '\\' {
            let n = s[i + 1..].chars().next().ok_or("dangling escape")?;
            i += 1 + n.len_utf8();
            match n {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                ' ' => out.push(' '),
                'x' | 'u' | 'U' => {
                    let len = match n {
                        'x' => 2,
                        'u' => 4,
                        _ => 8,
                    };
                    let hex = s.get(i..i + len).ok_or("short escape")?;
                    let code = u32::from_str_radix(hex, 16).map_err(|_| "bad escape")?;
                    out.push(char::from_u32(code).ok_or("bad escape")?);
                    i += len;
                }
                '\n' => {}
                other => return Err(format!("unknown escape \\{other}")),
            }
            continue;
        }
        if c == '\n' {
            // Line folding inside a quoted scalar.
            out.push(' ');
        } else {
            out.push(c);
        }
        i += c.len_utf8();
    }
    Err("unclosed quote".into())
}

/// Parse a flow collection or scalar starting at `at` (after whitespace).
fn parse_flow(s: &str, at: usize) -> Result<(usize, Value), String> {
    let b = s.as_bytes();
    let skip_ws = |mut i: usize| {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    };
    let i = skip_ws(at);
    if i >= b.len() {
        return Err("unexpected end of flow collection".into());
    }
    match b[i] {
        b'[' | b'{' => {
            let close = if b[i] == b'[' { b']' } else { b'}' };
            let is_seq = b[i] == b'[';
            let mut items = Vec::new();
            let mut j = skip_ws(i + 1);
            if j < b.len() && b[j] == close {
                return Ok((
                    j + 1,
                    if is_seq {
                        Value::Seq(items)
                    } else {
                        Value::Other
                    },
                ));
            }
            loop {
                let (end, v) = parse_flow(s, j)?;
                if !is_seq {
                    // `key: value` pairs inside a flow mapping.
                    let k = skip_ws(end);
                    if k < b.len() && b[k] == b':' {
                        let (end2, _) = parse_flow(s, k + 1)?;
                        j = skip_ws(end2);
                    } else {
                        j = k;
                    }
                } else {
                    items.push(v);
                    j = skip_ws(end);
                }
                if j >= b.len() {
                    return Err("unclosed flow collection".into());
                }
                if b[j] == b',' {
                    j = skip_ws(j + 1);
                    if j < b.len() && b[j] == close {
                        return Ok((
                            j + 1,
                            if is_seq {
                                Value::Seq(items)
                            } else {
                                Value::Other
                            },
                        ));
                    }
                    continue;
                }
                if b[j] == close {
                    return Ok((
                        j + 1,
                        if is_seq {
                            Value::Seq(items)
                        } else {
                            Value::Other
                        },
                    ));
                }
                return Err(format!(
                    "unexpected {:?} in a flow collection",
                    b[j] as char
                ));
            }
        }
        b'"' | b'\'' => {
            let (end, v) = parse_quoted(s, i)?;
            Ok((end, Value::Scalar(v)))
        }
        b']' | b'}' | b',' => Err("empty flow item".into()),
        _ => {
            let mut j = i;
            while j < b.len() {
                let c = b[j];
                if matches!(c, b',' | b']' | b'}' | b'[' | b'{') {
                    break;
                }
                if c == b':'
                    && (j + 1 >= b.len() || matches!(b[j + 1], b' ' | b',' | b']' | b'}' | b'\n'))
                {
                    break;
                }
                if c == b'#' && j > 0 && matches!(b[j - 1], b' ' | b'\t') {
                    return Err("a comment inside a flow collection".into());
                }
                j += 1;
            }
            let text = s[i..j].split_whitespace().collect::<Vec<_>>().join(" ");
            Ok((
                j,
                if text.is_empty() {
                    Value::Null
                } else {
                    Value::Scalar(text)
                },
            ))
        }
    }
}

// ------------------------------------------------------------------ fields

/// Mode's fields as read from a file (04 §3.1), with per-key problems.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fields {
    pub id: Option<String>,
    /// `None` = key absent (default `todo`).
    pub status: Option<String>,
    pub urgent: Option<bool>,
    pub when: Option<WhenRaw>,
    /// The `when` text as found (`when_raw`).
    pub when_raw: Option<String>,
    pub tags: Vec<String>,
    /// Canonical instants (`2026-10-18T09:12:44.120Z`).
    pub created: Option<String>,
    pub updated: Option<String>,
    pub rev: Option<i64>,
    /// Keys whose value was present but invalid, with why (§3.3 "Valid YAML, invalid value").
    pub invalid: Vec<(String, String)>,
}

impl Fields {
    fn read(doc: &Document, tz: &dyn LocalTz) -> Fields {
        let mut f = Fields::default();
        let bad = f_invalid;
        let mut invalid: Vec<(String, String)> = Vec::new();
        for e in doc.entries() {
            let scalar = match &e.value {
                Value::Scalar(s) => Some(s.as_str()),
                _ => None,
            };
            match e.key.as_str() {
                "id" => match scalar.map(str::trim).filter(|s| !s.is_empty()) {
                    Some(s) => f.id = Some(s.to_string()),
                    None => invalid.push(bad("id", "not a string".into())),
                },
                "status" => match &e.value {
                    Value::Null => {}
                    Value::Scalar(s) if s.trim().is_empty() => {}
                    Value::Scalar(s) => f.status = Some(fields::fold_status(s)),
                    _ => invalid.push(bad("status", "not a string".into())),
                },
                "urgent" => match &e.value {
                    Value::Null => {}
                    Value::Scalar(s) => match fields::parse_bool(s) {
                        Some(b) => f.urgent = Some(b),
                        None => invalid.push(bad("urgent", format!("{s:?} is not a boolean"))),
                    },
                    _ => invalid.push(bad("urgent", "not a boolean".into())),
                },
                "when" => match &e.value {
                    Value::Null => {}
                    Value::Scalar(s) if s.trim().is_empty() || s.trim() == "none" => {}
                    Value::Scalar(s) => match WhenRaw::parse(s) {
                        Some(w) => {
                            f.when = Some(w);
                            f.when_raw = Some(s.trim().to_string());
                        }
                        None => invalid.push(bad("when", format!("{s:?} is not a date"))),
                    },
                    _ => invalid.push(bad("when", "not a date".into())),
                },
                "tags" => {
                    let items: Vec<String> = match &e.value {
                        Value::Null => Vec::new(),
                        Value::Scalar(s) => fields::split_tag_string(s),
                        Value::Seq(items) => {
                            let mut v = Vec::new();
                            for it in items {
                                match it {
                                    Value::Scalar(s) => v.push(s.clone()),
                                    Value::Null => {}
                                    _ => {
                                        invalid.push(bad("tags", "an item is not a string".into()))
                                    }
                                }
                            }
                            v
                        }
                        Value::Other => {
                            invalid.push(bad("tags", "not a list".into()));
                            Vec::new()
                        }
                    };
                    let (tags, dropped) = fields::normalize_tags(items);
                    for d in dropped {
                        invalid.push(bad("tags", format!("{d:?} is not a valid tag")));
                    }
                    f.tags = tags;
                }
                "created" | "updated" => {
                    let parsed = scalar.and_then(|s| fields::canonical_instant(s, tz));
                    match (parsed, &e.value) {
                        (Some(t), _) => {
                            if e.key == "created" {
                                f.created = Some(t)
                            } else {
                                f.updated = Some(t)
                            }
                        }
                        (None, Value::Null) => {}
                        (None, _) => invalid.push(bad(&e.key, "not a timestamp".into())),
                    }
                }
                "rev" => match scalar.and_then(|s| s.trim().parse::<i64>().ok()) {
                    Some(n) if n >= 1 => f.rev = Some(n),
                    _ if matches!(e.value, Value::Null) => {}
                    _ => invalid.push(bad("rev", "not an integer ≥ 1".into())),
                },
                _ => {}
            }
        }
        f.invalid = invalid;
        f
    }

    pub fn is_invalid(&self, key: &str) -> bool {
        self.invalid.iter().any(|(k, _)| k == key)
    }
}

fn f_invalid(key: &str, why: String) -> (String, String) {
    (key.to_string(), why)
}

// ------------------------------------------------------------------ writer

/// The values Mode writes (04 §3.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Canonical {
    pub id: String,
    pub status: String,
    /// `status` is always written by user-visible writes; a touch-up of a
    /// file without a `status` key leaves it out while it is the default
    /// (04 §3.3: a foreign file gains four lines, not eight).
    pub write_status: bool,
    pub urgent: bool,
    /// The resolved local date.
    pub when: Option<String>,
    /// Display names.
    pub tags: Vec<String>,
    pub created: String,
    pub updated: String,
    pub rev: i64,
}

/// How a rewrite treats Mode keys whose current value is invalid: kept
/// verbatim in their slot unless the write sets that key (04 §3.3).
#[derive(Debug, Clone, Default)]
pub struct WriteOpts {
    /// Mode keys this write sets explicitly.
    pub edited: BTreeSet<&'static str>,
}

impl WriteOpts {
    pub fn editing(keys: &[&'static str]) -> WriteOpts {
        WriteOpts {
            edited: keys.iter().copied().collect(),
        }
    }
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

const YAML_WORDS: &[&str] = &[
    "true", "false", "yes", "no", "on", "off", "null", "~", "y", "n",
];

/// A UUID, slug, date, timestamp or integer is written plain (04 §3.4).
fn plain_ok(s: &str) -> bool {
    if s.is_empty() || YAML_WORDS.contains(&s.to_ascii_lowercase().as_str()) {
        return false;
    }
    if is_uuid(s) || fields::parse_date(s).is_some() || s.parse::<i64>().is_ok() {
        return true;
    }
    if crate::db::parse_timestamp(s).is_some() && s.ends_with('Z') {
        return true;
    }
    let b = s.as_bytes();
    (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// Double-quote with YAML escapes.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn scalar_text(s: &str) -> String {
    if plain_ok(s) { s.to_string() } else { quote(s) }
}

/// A tag inside a flow sequence: plain unless YAML-significant (04 §3.4).
fn tag_text(t: &str) -> String {
    let significant = t.is_empty()
        || t.starts_with([
            '-', '?', ':', '!', '&', '*', '|', '>', '\'', '%', '@', '`', ' ',
        ])
        || t.ends_with(' ')
        || t.contains([':', '{', '}', '\'', '\\'])
        || t.chars().any(char::is_control)
        || YAML_WORDS.contains(&t.to_ascii_lowercase().as_str())
        || t.parse::<f64>().is_ok();
    if significant { quote(t) } else { t.to_string() }
}

/// The block's lines in Mode's shape: Mode keys in fixed order, then every
/// unknown entry verbatim in its original order (04 §3.4).
pub fn render_block(c: &Canonical, doc: Option<&Document>, eol: Eol, opts: &WriteOpts) -> String {
    let nl = eol.as_str();
    let mut out = String::new();
    // Keep the author's text for a Mode key only while its value is invalid
    // and this write does not set it (04 §3.3).
    let keep_raw = |key: &'static str| -> Option<&Entry> {
        if opts.edited.contains(key) {
            return None;
        }
        let e = doc?.entry(key)?;
        if value_valid(key, &e.value) {
            None
        } else {
            Some(e)
        }
    };
    let raw = |e: &Entry, out: &mut String| out.push_str(&ensure_eol(&e.raw, nl));

    out.push_str(&format!("id: {}{nl}", scalar_text(&c.id)));
    if let Some(e) = keep_raw("status") {
        raw(e, &mut out);
    } else if c.write_status || c.status != "todo" {
        out.push_str(&format!("status: {}{nl}", scalar_text(&c.status)));
    }
    if let Some(e) = keep_raw("urgent") {
        raw(e, &mut out);
    } else if c.urgent {
        out.push_str(&format!("urgent: true{nl}"));
    }
    if let Some(e) = keep_raw("when") {
        raw(e, &mut out);
    } else if let Some(w) = &c.when {
        out.push_str(&format!("when: {w}{nl}"));
    }
    if let Some(e) = keep_raw("tags") {
        raw(e, &mut out);
    } else if !c.tags.is_empty() {
        let items: Vec<String> = c.tags.iter().map(|t| tag_text(t)).collect();
        out.push_str(&format!("tags: [{}]{nl}", items.join(", ")));
    }
    out.push_str(&format!("created: {}{nl}", c.created));
    out.push_str(&format!("updated: {}{nl}", c.updated));
    out.push_str(&format!("rev: {}{nl}", c.rev));
    if let Some(doc) = doc {
        for e in doc.entries() {
            if !MODE_KEYS.contains(&e.key.as_str()) {
                raw(e, &mut out);
            }
        }
    }
    out
}

/// Whether a Mode key's value is one §3.1 accepts (absent counts as valid).
pub fn value_valid(key: &str, v: &Value) -> bool {
    match key {
        "status" => matches!(v, Value::Scalar(_) | Value::Null),
        "urgent" => match v {
            Value::Scalar(s) => fields::parse_bool(s).is_some(),
            Value::Null => true,
            _ => false,
        },
        "when" => match v {
            Value::Scalar(s) => {
                s.trim().is_empty() || s.trim() == "none" || WhenRaw::parse(s).is_some()
            }
            Value::Null => true,
            _ => false,
        },
        "tags" => match v {
            Value::Seq(items) => items.iter().all(|i| match i {
                Value::Scalar(s) => fields::tag_display(s).is_some(),
                Value::Null => true,
                _ => false,
            }),
            Value::Scalar(s) => fields::split_tag_string(s)
                .iter()
                .all(|t| t.trim().is_empty() || fields::tag_display(t).is_some()),
            Value::Null => true,
            Value::Other => false,
        },
        _ => true,
    }
}

fn ensure_eol(raw: &str, nl: &str) -> String {
    if raw.ends_with('\n') {
        raw.to_string()
    } else {
        format!("{raw}{nl}")
    }
}

/// Rewrite `doc` with Mode's values: only the block between the delimiters
/// is regenerated; the BOM, the opening delimiter line and everything from
/// the closing delimiter to EOF are copied verbatim. A file without a block
/// gains one at the top, followed directly by its original bytes. Returns
/// `None` for a block that is not valid YAML (never rewritten, §3.3).
pub fn rewrite(doc: &Document, c: &Canonical, opts: &WriteOpts) -> Option<String> {
    match &doc.block {
        Block::Invalid { .. } => None,
        Block::Valid {
            inner_start, close, ..
        } => {
            let mut out = String::with_capacity(doc.text.len() + 64);
            out.push_str(&doc.text[..*inner_start]);
            out.push_str(&render_block(c, Some(doc), doc.eol, opts));
            out.push_str(&doc.text[*close..]);
            Some(out)
        }
        Block::Absent => {
            let nl = doc.eol.as_str();
            let start = if doc.bom { BOM.len() } else { 0 };
            let mut out = String::with_capacity(doc.text.len() + 200);
            if doc.bom {
                out.push_str(BOM);
            }
            out.push_str("---");
            out.push_str(nl);
            out.push_str(&render_block(c, None, doc.eol, opts));
            out.push_str("---");
            out.push_str(nl);
            out.push_str(&doc.text[start..]);
            Some(out)
        }
    }
}

/// Whether Mode leaves one blank line between the closing `---` and the
/// first body line of a file it creates (04 §3.4 "Blank line").
pub const BLANK_LINE_AFTER_FRONTMATTER: bool = true;

/// A new file as Mode creates it: frontmatter, one blank line, the body;
/// `\n` endings throughout, whatever the body arrived with
/// ([`super::eol::NEW_FILE`]); exactly one final newline (04 §3.4, §9.4).
pub fn new_file(c: &Canonical, body: &str) -> String {
    let eol = super::eol::NEW_FILE;
    let nl = eol.as_str();
    let mut out = format!("---{nl}");
    out.push_str(&render_block(c, None, eol, &WriteOpts::default()));
    out.push_str(&format!("---{nl}"));
    if BLANK_LINE_AFTER_FRONTMATTER {
        out.push_str(nl);
    }
    let body = super::eol::normalize(body, eol);
    let body = body.trim_end_matches(['\n', '\r']);
    out.push_str(body);
    out.push_str(nl);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::fields::FixedTz;

    const TZ: FixedTz = FixedTz(0);

    #[test]
    fn detects_blocks_bom_and_endings() {
        let d = Document::parse("# Title\n\nbody\n");
        assert_eq!(d.block, Block::Absent);
        assert_eq!(d.body(), "# Title\n\nbody\n");
        let d = Document::parse("\u{FEFF}---\r\nid: x\r\n---\r\n# T\r\n");
        assert!(d.bom);
        assert_eq!(d.eol, Eol::CrLf);
        assert!(d.has_block());
        assert_eq!(d.body(), "# T\r\n");
        assert_eq!(d.entry("id").unwrap().raw, "id: x\r\n");
        let d = Document::parse("---\nid: x\n...\nbody");
        assert_eq!(d.body(), "body");
        let d = Document::parse("---\nunclosed\n");
        assert_eq!(d.block, Block::Absent);
        let d = Document::parse("---\n---\nbody");
        assert!(d.has_block());
        assert!(d.entries().is_empty());
    }

    #[test]
    fn invalid_yaml_is_reported_not_rewritten() {
        for bad in [
            "---\nSome text\n---\n",
            "---\na: 1\na: 2\n---\n",
            "---\n\tid: x\n---\n",
            "---\ntags: [a, b\n---\n",
            "---\ntitle: \"open\n---\n",
            "---\nx: a: b\n---\n",
            "---\n- a\n- b\n---\n",
            "---\nk: v\n- a\n---\n",
        ] {
            let d = Document::parse(bad);
            assert!(
                d.error().is_some(),
                "{bad:?} must be invalid, got {:?}",
                d.block
            );
            assert_eq!(d.body(), bad, "the whole file is the body");
            assert!(rewrite(&d, &Canonical::default(), &WriteOpts::default()).is_none());
        }
    }

    #[test]
    fn values_of_every_shape() {
        let text = "---\nid: 0192f3c1-7c4e-7a0b-9d3e-5e1a2b3c4d5f\nstatus: Doing # c\nurgent: yes\nwhen: today\ntags:\n  - '#Gym'\n  - \"big  project\"\n  - x\ncreated: 2026-10-18\nupdated: '2026-10-18T20:05:01.009Z'\nrev: 7\naliases: [plumber, \"a, b\"]\nnested:\n  a: 1\n  b: [x]\nnote: |\n  line one\n  # not a comment\nplain: multi\n  line\n---\nbody\n";
        let d = Document::parse(text);
        assert!(d.error().is_none(), "{:?}", d.block);
        let f = d.fields(&TZ);
        assert_eq!(
            f.id.as_deref(),
            Some("0192f3c1-7c4e-7a0b-9d3e-5e1a2b3c4d5f")
        );
        assert_eq!(f.status.as_deref(), Some("doing"));
        assert_eq!(f.urgent, Some(true));
        assert_eq!(f.when, Some(WhenRaw::Today));
        assert_eq!(f.tags, vec!["Gym", "big project", "x"]);
        assert_eq!(f.created.as_deref(), Some("2026-10-18T00:00:00.000Z"));
        assert_eq!(f.updated.as_deref(), Some("2026-10-18T20:05:01.009Z"));
        assert_eq!(f.rev, Some(7));
        assert!(f.invalid.is_empty(), "{:?}", f.invalid);
        assert_eq!(d.entry("nested").unwrap().value, Value::Other);
        assert_eq!(
            d.entry("note").unwrap().value,
            Value::Scalar("line one\n# not a comment".into())
        );
        let col0 = Document::parse("---\ntags:\n- a\n- b # c\n---\n");
        assert_eq!(col0.fields(&TZ).tags, vec!["a", "b"]);
        assert_eq!(
            d.entry("plain").unwrap().value,
            Value::Scalar("multi line".into())
        );
        assert_eq!(
            d.entry("aliases").unwrap().value,
            Value::Seq(vec![
                Value::Scalar("plumber".into()),
                Value::Scalar("a, b".into())
            ])
        );
    }

    #[test]
    fn invalid_values_take_defaults_and_survive_rewrites() {
        let text = "---\nid: n1\nurgent: maybe\nwhen: next week\nrev: seven\nx: 1\n---\nB";
        let d = Document::parse(text);
        let f = d.fields(&TZ);
        assert_eq!(f.urgent, None);
        assert_eq!(f.when, None);
        assert_eq!(f.rev, None);
        assert_eq!(f.invalid.len(), 3);
        let c = Canonical {
            id: "n1".into(),
            status: "todo".into(),
            write_status: true,
            created: "2026-10-18T09:00:00.000Z".into(),
            updated: "2026-10-18T09:00:00.000Z".into(),
            rev: 2,
            ..Default::default()
        };
        let out = rewrite(&d, &c, &WriteOpts::editing(&["rev", "updated"])).unwrap();
        assert_eq!(
            out,
            "---\nid: n1\nstatus: todo\nurgent: maybe\nwhen: next week\ncreated: 2026-10-18T09:00:00.000Z\nupdated: 2026-10-18T09:00:00.000Z\nrev: 2\nx: 1\n---\nB"
        );
        let out = rewrite(&d, &c, &WriteOpts::editing(&["urgent", "when", "rev"])).unwrap();
        assert!(!out.contains("maybe") && !out.contains("next week"));
    }

    #[test]
    fn canonical_form_round_trips_byte_for_byte() {
        let text = "---\nid: 0192f3c1-7c4e-7a0b-9d3e-5e1a2b3c4d5f\nstatus: doing\nurgent: true\nwhen: 2026-10-19\ntags: [groceries, errands]\ncreated: 2026-10-18T09:12:44.120Z\nupdated: 2026-10-18T20:05:01.009Z\nrev: 7\naliases: [plumber]\n---\n# Buy the things\n\nBefore the weekend.\n";
        let d = Document::parse(text);
        let f = d.fields(&TZ);
        let c = Canonical {
            id: f.id.clone().unwrap(),
            status: f.status.clone().unwrap(),
            write_status: true,
            urgent: true,
            when: Some("2026-10-19".into()),
            tags: f.tags.clone(),
            created: f.created.clone().unwrap(),
            updated: f.updated.clone().unwrap(),
            rev: 7,
        };
        assert_eq!(rewrite(&d, &c, &WriteOpts::default()).unwrap(), text);
    }

    #[test]
    fn foreign_file_gains_a_block_and_keeps_its_bytes() {
        let text = "\u{FEFF}# Note\r\n\r\n- [ ] a  \r\n";
        let d = Document::parse(text);
        let c = Canonical {
            id: "0192f3c1-7c4e-7a0b-9d3e-5e1a2b3c4d5f".into(),
            status: "todo".into(),
            write_status: false,
            created: "2026-10-18T09:00:00.000Z".into(),
            updated: "2026-10-18T09:00:00.000Z".into(),
            rev: 1,
            ..Default::default()
        };
        let out = rewrite(&d, &c, &WriteOpts::default()).unwrap();
        assert_eq!(
            out,
            "\u{FEFF}---\r\nid: 0192f3c1-7c4e-7a0b-9d3e-5e1a2b3c4d5f\r\ncreated: 2026-10-18T09:00:00.000Z\r\nupdated: 2026-10-18T09:00:00.000Z\r\nrev: 1\r\n---\r\n# Note\r\n\r\n- [ ] a  \r\n"
        );
        let again = Document::parse(&out);
        assert_eq!(again.body(), "# Note\r\n\r\n- [ ] a  \r\n");
    }

    #[test]
    fn quoting_rules() {
        assert_eq!(scalar_text("doing"), "doing");
        assert_eq!(scalar_text("waiting on bob"), "\"waiting on bob\"");
        assert_eq!(scalar_text("true"), "\"true\"");
        assert_eq!(tag_text("big project"), "big project");
        assert_eq!(tag_text("true"), "\"true\"");
        assert_eq!(tag_text("2026"), "\"2026\"");
        assert_eq!(tag_text("a:b"), "\"a:b\"");
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        let d = Document::parse("---\nk: \"a\\\"b\\u00e9\"\n---\n");
        assert_eq!(d.entry("k").unwrap().value, Value::Scalar("a\"bé".into()));
    }
}
