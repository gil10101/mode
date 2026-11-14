//! Citations (03 §7.4, 06 §10.6): every answer cites the items it used as
//! numbered markers inline (`[1]`) and as a list under the answer; a chip
//! carries the item kind, its title, a snippet and the route that opens it
//! (03 §2.5). The model's citations are validated against what this turn
//! retrieved (tool results and attachments): an id no tool returned is
//! dropped, duplicates merge, markers are renumbered by first appearance,
//! unknown markers are removed, titles and routes come from the retrieved
//! item, a missing snippet is filled from it. An answer with no retrieved
//! items is labeled "no sources — answered from the question alone".

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// The label of an answer with nothing retrieved (03 §7.4).
pub const NO_SOURCES: &str = "no sources — answered from the question alone";
/// Longest stored snippet, in characters.
pub const SNIPPET_MAX: usize = 200;

/// Item kinds a citation may name.
pub const KINDS: [&str; 6] = [
    "note",
    "message",
    "thread",
    "session",
    "calendar_event",
    "file",
];

/// A retrieved item: a Sources panel chip, a citation candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceItem {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub snippet: String,
    /// 03 §2.5 route (`notes/<id>`, `inbox/message/<id>`, …).
    pub route: String,
    /// One meta line ("Ana Ruiz · 2026-10-21 09:12").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
}

/// The route that opens an item in its view (03 §2.5).
pub fn route(kind: &str, id: &str) -> String {
    match kind {
        "note" => format!("notes/{id}"),
        "message" => format!("inbox/message/{id}"),
        "thread" => format!("inbox/thread/{id}"),
        "session" => format!("sessions/{id}"),
        "calendar_event" => format!("calendar/{id}"),
        "automation" => format!("automations/{id}"),
        "file" => format!("file://{id}"),
        _ => String::new(),
    }
}

/// What the model proposed (`citations` of the reply envelope).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCitation {
    #[serde(default)]
    pub n: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub snippet: String,
}

/// A validated citation, stored in `chat_turns.citations_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub n: u32,
    pub kind: String,
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub route: String,
}

impl Citation {
    /// The kind word on the chip.
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_str() {
            "note" => "note",
            "message" => "message",
            "thread" => "thread",
            "session" => "session",
            "calendar_event" => "event",
            "file" => "file",
            _ => "item",
        }
    }

    /// The `links.to_kind` for a `references` link (files have none).
    pub fn link_kind(&self) -> Option<&'static str> {
        match self.kind.as_str() {
            "note" => Some("note"),
            "message" => Some("message"),
            "thread" => Some("thread"),
            "session" => Some("session"),
            "calendar_event" => Some("calendar_event"),
            _ => None,
        }
    }
}

/// A proposed citation that did not survive validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dropped {
    pub kind: String,
    pub id: String,
    pub reason: String,
}

/// The validator's answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    /// The answer with markers renumbered (unknown ones removed).
    pub text: String,
    pub citations: Vec<Citation>,
    pub dropped: Vec<Dropped>,
    /// Nothing was retrieved this turn (03 §7.4 label).
    pub no_sources: bool,
    /// Items were retrieved but the answer cites none of them.
    pub uncited: bool,
}

/// Normalize a model's kind word.
pub fn normalise_kind(k: &str) -> String {
    match k.trim().to_ascii_lowercase().as_str() {
        "event" | "calendar" | "calendar_event" | "calendar-event" => "calendar_event".into(),
        "email" | "mail" | "message" | "msg" => "message".into(),
        other => other.to_string(),
    }
}

fn clip(s: &str, max: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    let mut out: String = t.chars().take(max - 1).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

fn markers() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(\s*)\[(\d{1,3}(?:\s*,\s*\d{1,3})*)\](\()?")
            .unwrap_or_else(|_| Regex::new("$^").expect("empty regex"))
    })
}

/// Validate the model's citations against the turn's retrieved items.
pub fn validate(answer: &str, proposed: &[RawCitation], retrieved: &[SourceItem]) -> Checked {
    let mut index: HashMap<(String, String), &SourceItem> = HashMap::new();
    let mut by_id: HashMap<&str, &SourceItem> = HashMap::new();
    for it in retrieved {
        index.entry((it.kind.clone(), it.id.clone())).or_insert(it);
        by_id.entry(it.id.as_str()).or_insert(it);
    }
    // old marker number → (kind, id) of a valid item.
    let mut valid: HashMap<i64, (String, String)> = HashMap::new();
    let mut snippets: HashMap<(String, String), String> = HashMap::new();
    let mut dropped = Vec::new();
    for c in proposed {
        let kind = normalise_kind(&c.kind);
        let id = c.id.trim().to_string();
        let item = index
            .get(&(kind.clone(), id.clone()))
            .copied()
            // The right id under another kind word (message vs thread).
            .or_else(|| by_id.get(id.as_str()).copied());
        match item {
            Some(it) => {
                let key = (it.kind.clone(), it.id.clone());
                if !c.snippet.trim().is_empty() {
                    snippets
                        .entry(key.clone())
                        .or_insert_with(|| c.snippet.clone());
                }
                valid.entry(c.n).or_insert(key);
            }
            None => dropped.push(Dropped {
                kind,
                id,
                reason: if id_is_blank(&c.id) {
                    "no id".into()
                } else {
                    "not among the items retrieved for this turn".into()
                },
            }),
        }
    }
    // Renumber by first inline appearance.
    let mut order: Vec<(String, String)> = Vec::new();
    let re = markers();
    let text = re
        .replace_all(answer, |caps: &regex::Captures<'_>| {
            let whole = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            if caps.get(3).is_some() {
                // A Markdown link `[1](…)`: leave it.
                return whole.to_string();
            }
            let mut nums = Vec::new();
            for part in caps[2].split(',') {
                let Ok(n) = part.trim().parse::<i64>() else {
                    continue;
                };
                let Some(key) = valid.get(&n) else { continue };
                let pos = match order.iter().position(|k| k == key) {
                    Some(p) => p,
                    None => {
                        order.push(key.clone());
                        order.len() - 1
                    }
                };
                if !nums.contains(&(pos + 1)) {
                    nums.push(pos + 1);
                }
            }
            if nums.is_empty() {
                // An unknown marker goes, with the space before it.
                return String::new();
            }
            let lead = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            format!(
                "{lead}{}",
                nums.iter().map(|n| format!("[{n}]")).collect::<String>()
            )
        })
        .into_owned();
    // Valid citations never referenced inline still go in the list.
    let mut rest: Vec<(i64, (String, String))> = valid.into_iter().collect();
    rest.sort_by_key(|(n, _)| *n);
    for (_, key) in rest {
        if !order.contains(&key) {
            order.push(key);
        }
    }
    let citations: Vec<Citation> = order
        .iter()
        .enumerate()
        .filter_map(|(i, key)| {
            let it = index.get(key)?;
            let snippet = snippets
                .get(key)
                .filter(|s| s.chars().count() <= 400)
                .cloned()
                .unwrap_or_else(|| it.snippet.clone());
            Some(Citation {
                n: i as u32 + 1,
                kind: it.kind.clone(),
                id: it.id.clone(),
                title: it.title.clone(),
                snippet: clip(&snippet, SNIPPET_MAX),
                route: it.route.clone(),
            })
        })
        .collect();
    Checked {
        text: text.trim_end().to_string(),
        uncited: !retrieved.is_empty() && citations.is_empty(),
        no_sources: retrieved.is_empty(),
        citations,
        dropped,
    }
}

fn id_is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, id: &str, title: &str) -> SourceItem {
        SourceItem {
            kind: kind.into(),
            id: id.into(),
            title: title.into(),
            snippet: format!("snippet of {title}"),
            route: route(kind, id),
            meta: None,
        }
    }

    fn raw(n: i64, kind: &str, id: &str) -> RawCitation {
        RawCitation {
            n,
            kind: kind.into(),
            id: id.into(),
            snippet: String::new(),
        }
    }

    #[test]
    fn invented_ids_are_dropped_and_markers_renumbered() {
        let got = vec![
            item("note", "n1", "Plan"),
            item("message", "m1", "Re: budget"),
        ];
        let c = validate(
            "Budget is due Friday [3]. The plan says so [1][9].",
            &[
                raw(1, "note", "n1"),
                raw(3, "email", "m1"),
                raw(9, "note", "made-up"),
            ],
            &got,
        );
        assert_eq!(c.text, "Budget is due Friday [1]. The plan says so [2].");
        assert_eq!(c.citations.len(), 2);
        assert_eq!(
            (c.citations[0].n, c.citations[0].kind.as_str()),
            (1, "message")
        );
        assert_eq!(c.citations[1].route, "notes/n1");
        assert_eq!(c.citations[1].snippet, "snippet of Plan");
        assert_eq!(c.dropped.len(), 1);
        assert!(!c.uncited && !c.no_sources);
    }

    #[test]
    fn nothing_retrieved_means_no_sources() {
        let c = validate("Hello.", &[raw(1, "note", "x")], &[]);
        assert!(c.no_sources);
        assert!(c.citations.is_empty());
        assert_eq!(c.dropped.len(), 1);
        let c = validate("I could not find it.", &[], &[item("note", "n1", "A")]);
        assert!(c.uncited);
    }

    #[test]
    fn grouped_markers_and_links_survive() {
        let got = vec![item("note", "a", "A"), item("note", "b", "B")];
        let c = validate(
            "Both [1, 2]. See [docs](https://x.example).",
            &[raw(1, "note", "a"), raw(2, "note", "b")],
            &got,
        );
        assert_eq!(c.text, "Both [1][2]. See [docs](https://x.example).");
    }
}
