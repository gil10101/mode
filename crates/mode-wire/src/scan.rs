//! A span scanner for JSON text, `std` only: where the members of an object
//! and the elements of an array sit in the source bytes.
//!
//! Two users: the relay salvages the complete top-level members of a payload
//! that was cut at the 8 MB cap (05 §3.3 "forwarded truncated"), and the hook
//! installer splices Mode's entries into `~/.claude/settings.json` so every
//! byte outside Mode's own entries stays as it was (05 §3.6 "the writer
//! re-serializes only the hook entries and keeps the rest of the file text").
//!
//! The scanner does not validate JSON; callers parse the spans they use with
//! `serde_json`. On truncated input it stops at the first incomplete value and
//! reports `close: None`.

use std::ops::Range;

/// One `"key": value` pair of an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The decoded key.
    pub key: String,
    /// Index of the key's opening quote (the member starts here).
    pub start: usize,
    /// Byte range of the value (exclusive end).
    pub value: Range<usize>,
}

impl Member {
    /// The whole member, key through value.
    pub fn span(&self) -> Range<usize> {
        self.start..self.value.end
    }
}

/// The members of one object and where its braces are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// Index of `{`.
    pub open: usize,
    pub members: Vec<Member>,
    /// Index of `}`; `None` when the input ends (or breaks) before it.
    pub close: Option<usize>,
}

impl Object {
    pub fn is_complete(&self) -> bool {
        self.close.is_some()
    }

    pub fn member(&self, key: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.key == key)
    }

    /// The range of the whole object, `{` through `}`.
    pub fn span(&self) -> Option<Range<usize>> {
        self.close.map(|c| self.open..c + 1)
    }
}

/// The elements of one array and where its brackets are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Array {
    /// Index of `[`.
    pub open: usize,
    pub elements: Vec<Range<usize>>,
    /// Index of `]`; `None` when the input ends (or breaks) before it.
    pub close: Option<usize>,
}

impl Array {
    pub fn is_complete(&self) -> bool {
        self.close.is_some()
    }
}

/// Index of the first byte after the JSON whitespace starting at `i`.
pub fn skip_ws(src: &[u8], mut i: usize) -> usize {
    while i < src.len() && matches!(src[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// Index of the first non-whitespace byte of the document, if any.
pub fn first_value(src: &[u8]) -> Option<usize> {
    let i = skip_ws(src, 0);
    (i < src.len()).then_some(i)
}

/// Exclusive end of the string literal whose opening quote is at `i`.
fn string_end(src: &[u8], i: usize) -> Option<usize> {
    debug_assert_eq!(src.get(i), Some(&b'"'));
    let mut j = i + 1;
    while j < src.len() {
        match src[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// Exclusive end of the value that starts at `i`, or `None` when the value is
/// cut off by the end of the input or does not start like a JSON value.
pub fn value_end(src: &[u8], i: usize) -> Option<usize> {
    match src.get(i)? {
        b'"' => string_end(src, i),
        b'{' => scan_object(src, i)?.close.map(|c| c + 1),
        b'[' => scan_array(src, i)?.close.map(|c| c + 1),
        b't' | b'n' => (src.len() >= i + 4).then_some(i + 4),
        b'f' => (src.len() >= i + 5).then_some(i + 5),
        b'-' | b'0'..=b'9' => {
            let mut j = i + 1;
            while j < src.len() && matches!(src[j], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            {
                j += 1;
            }
            // A number touching the end of the input may itself be cut off.
            (j < src.len()).then_some(j)
        }
        _ => None,
    }
}

/// Scan the object whose `{` is at `open`. `None` when `src[open]` is not `{`.
pub fn scan_object(src: &[u8], open: usize) -> Option<Object> {
    if src.get(open) != Some(&b'{') {
        return None;
    }
    let mut obj = Object {
        open,
        members: Vec::new(),
        close: None,
    };
    let mut i = skip_ws(src, open + 1);
    if src.get(i) == Some(&b'}') {
        obj.close = Some(i);
        return Some(obj);
    }
    loop {
        if src.get(i) != Some(&b'"') {
            return Some(obj);
        }
        let Some(key_end) = string_end(src, i) else {
            return Some(obj);
        };
        let key: String = serde_json::from_slice(&src[i..key_end]).ok()?;
        let mut j = skip_ws(src, key_end);
        if src.get(j) != Some(&b':') {
            return Some(obj);
        }
        j = skip_ws(src, j + 1);
        let Some(end) = value_end(src, j) else {
            return Some(obj);
        };
        obj.members.push(Member {
            key,
            start: i,
            value: j..end,
        });
        i = skip_ws(src, end);
        match src.get(i) {
            Some(b',') => i = skip_ws(src, i + 1),
            Some(b'}') => {
                obj.close = Some(i);
                return Some(obj);
            }
            _ => return Some(obj),
        }
    }
}

/// Scan the array whose `[` is at `open`. `None` when `src[open]` is not `[`.
pub fn scan_array(src: &[u8], open: usize) -> Option<Array> {
    if src.get(open) != Some(&b'[') {
        return None;
    }
    let mut arr = Array {
        open,
        elements: Vec::new(),
        close: None,
    };
    let mut i = skip_ws(src, open + 1);
    if src.get(i) == Some(&b']') {
        arr.close = Some(i);
        return Some(arr);
    }
    loop {
        let Some(end) = value_end(src, i) else {
            return Some(arr);
        };
        arr.elements.push(i..end);
        i = skip_ws(src, end);
        match src.get(i) {
            Some(b',') => i = skip_ws(src, i + 1),
            Some(b']') => {
                arr.close = Some(i);
                return Some(arr);
            }
            _ => return Some(arr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(src: &str, r: &Range<usize>) -> String {
        src[r.clone()].to_string()
    }

    #[test]
    fn complete_object_yields_every_member_span() {
        let src =
            r#" { "a": 1, "b": "x\"y", "c": [1, {"d": null}], "e": {"f": true} , "g": -2.5e3 } "#;
        let open = first_value(src.as_bytes()).unwrap();
        let obj = scan_object(src.as_bytes(), open).unwrap();
        assert!(obj.is_complete());
        let keys: Vec<&str> = obj.members.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["a", "b", "c", "e", "g"],
            "nested keys are not top-level members"
        );
        assert_eq!(s(src, &obj.member("a").unwrap().value), "1");
        assert_eq!(s(src, &obj.member("b").unwrap().value), r#""x\"y""#);
        assert_eq!(
            s(src, &obj.member("c").unwrap().value),
            r#"[1, {"d": null}]"#
        );
        assert_eq!(s(src, &obj.member("e").unwrap().value), r#"{"f": true}"#);
        assert_eq!(s(src, &obj.member("g").unwrap().value), "-2.5e3");
        assert_eq!(s(src, &obj.member("g").unwrap().span()), r#""g": -2.5e3"#);
        assert_eq!(&src[obj.span().unwrap()], src.trim());
        let arr = scan_array(src.as_bytes(), obj.member("c").unwrap().value.start).unwrap();
        assert_eq!(arr.elements.len(), 2);
        assert_eq!(s(src, &arr.elements[1]), r#"{"d": null}"#);
        assert!(arr.is_complete());
    }

    #[test]
    fn truncated_object_keeps_the_complete_members_only() {
        let full = r#"{"session_id": "s", "tool_name": "Bash", "tool_response": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "x": 1}"#;
        let cut = &full[..full.len() - 20];
        let obj = scan_object(cut.as_bytes(), 0).unwrap();
        assert!(!obj.is_complete());
        let keys: Vec<&str> = obj.members.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["session_id", "tool_name"]);
        // Cut inside a nested value: the member holding it is dropped.
        let nested = r#"{"a": 1, "b": {"c": [1, 2, 3"#;
        let obj = scan_object(nested.as_bytes(), 0).unwrap();
        assert_eq!(obj.members.len(), 1);
        // Cut right after a number: ambiguous, dropped.
        let obj = scan_object(br#"{"a": 1, "b": 12"#, 0).unwrap();
        assert_eq!(obj.members.len(), 1);
        // Escaped keys decode.
        let obj = scan_object(br#"{"k\u0041": true}"#, 0).unwrap();
        assert_eq!(obj.members[0].key, "kA");
        assert!(obj.is_complete());
    }

    #[test]
    fn empty_containers_and_non_containers() {
        let obj = scan_object(b"{ }", 0).unwrap();
        assert!(obj.members.is_empty());
        assert_eq!(obj.close, Some(2));
        let arr = scan_array(b"[]", 0).unwrap();
        assert!(arr.elements.is_empty());
        assert_eq!(arr.close, Some(1));
        assert!(scan_object(b"[]", 0).is_none());
        assert!(scan_array(b"{}", 0).is_none());
        assert_eq!(first_value(b"   "), None);
        assert_eq!(value_end(b"tru", 0), None);
        assert_eq!(value_end(b"true", 0), Some(4));
        assert_eq!(value_end(b"false,", 0), Some(5));
        assert_eq!(value_end(b"\"abc", 0), None);
    }
}
