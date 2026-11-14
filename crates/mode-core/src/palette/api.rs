//! `palette_query {text, limit?, kinds?} → [{kind, id, title, detail, route, score}]`
//! (`PaletteQueryArgs` / `PaletteResult` in `ui/src/state/palette-store.ts`).
//! Kinds: `session`, `note`, `message`, `event`, `chat`, `automation`,
//! `setting`, and beyond the contract's list `command` (views and actions)
//! and `file` (files the sessions touched). A `message` row reads like the
//! kit's message row: title "Sam · #release · can we ship v2…" (sender,
//! chat, subject or snippet), detail the received time rendered at query
//! time ("4m", "2h", then "18 Oct"), route `inbox/message/<id>`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::index::{Entry, Kind, Palette, char_mask, received_label};
use crate::queries::fts::{Fts, build};

pub use crate::notes::api::{ApiError, ApiResult};

/// Default and largest result counts.
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryArgs {
    pub text: String,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Restrict to these kinds (`session`, `note`, `message`, `event`,
    /// `chat`, `automation`, `setting`, `command`, `file`): the group
    /// filters and Chat's Attach pickers.
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
}

/// One result row (32 px: kind glyph · title · detail).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub detail: String,
    /// The 03 §2.5 route that opens it (commands: the view they act in).
    pub route: String,
    pub score: f64,
}

/// Tier scores (see the module docs of [`super`]).
pub mod tier {
    pub const EXACT: f64 = 1000.0;
    pub const PREFIX: f64 = 850.0;
    pub const WORD: f64 = 650.0;
    pub const SUBSTRING: f64 = 500.0;
    pub const FUZZY: f64 = 300.0;
    pub const KEYWORD: f64 = 150.0;
    /// Below `KEYWORD` by more than `RECENCY_MAX`: a body hit never
    /// outranks a keyword match however recent.
    pub const BODY: f64 = 40.0;
    pub const RECENCY_MAX: f64 = 100.0;
}

fn word_prefix(text: &str, starts: &[u16], token: &str) -> Option<usize> {
    starts.iter().map(|s| *s as usize).find(|s| {
        text.as_bytes()
            .get(*s..)
            .is_some_and(|b| b.starts_with(token.as_bytes()))
    })
}

fn keywords_match(kw: &str, tokens: &[&str]) -> bool {
    tokens.iter().all(|t| {
        kw.match_indices(t).any(|(i, _)| {
            i == 0
                || !kw[..i]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
        })
    })
}

/// Subsequence match: `Some(compactness 0–1)`.
fn fuzzy(text: &str, q: &[u8]) -> Option<f64> {
    let t = text.as_bytes();
    if q.is_empty() || q.len() > t.len() {
        return None;
    }
    let mut qi = 0;
    let mut first = None;
    let mut last = 0;
    for (i, b) in t.iter().enumerate() {
        if *b == q[qi] {
            if first.is_none() {
                first = Some(i);
            }
            qi += 1;
            last = i;
            if qi == q.len() {
                break;
            }
        }
    }
    if qi < q.len() {
        return None;
    }
    let span = last - first.unwrap_or(0) + 1;
    Some(q.len() as f64 / span as f64)
}

/// The text score of one entry (0 = no match). `q_mask` is the query's
/// [`char_mask`]: an entry missing one of its letters is skipped unread.
pub fn score_text(e: &Entry, q: &str, tokens: &[&str], q_compact: &[u8]) -> f64 {
    score_masked(e, q, tokens, q_compact, char_mask(q))
}

fn score_masked(e: &Entry, q: &str, tokens: &[&str], q_compact: &[u8], q_mask: u64) -> f64 {
    if e.title_mask & q_mask != q_mask {
        return if e.keywords_mask & q_mask == q_mask && keywords_match(&e.keywords_lc, tokens) {
            tier::KEYWORD
        } else {
            0.0
        };
    }
    let t = e.title_lc.as_str();
    if t == q {
        return tier::EXACT;
    }
    if t.starts_with(q) {
        return tier::PREFIX + 50.0 * (q.len() as f64 / t.len().max(1) as f64);
    }
    let mut positions = Vec::with_capacity(tokens.len());
    let all_words = tokens
        .iter()
        .all(|tok| match word_prefix(t, &e.starts, tok) {
            Some(p) => {
                positions.push(p);
                true
            }
            None => false,
        });
    if all_words {
        let in_order = positions.windows(2).all(|w| w[0] < w[1]);
        return tier::WORD + if in_order { 50.0 } else { 0.0 };
    }
    if t.contains(q) {
        return tier::SUBSTRING;
    }
    if q_compact.len() >= 2
        && let Some(c) = fuzzy(t, q_compact)
    {
        return tier::FUZZY + 50.0 * c;
    }
    if keywords_match(&e.keywords_lc, tokens) {
        return tier::KEYWORD;
    }
    0.0
}

/// Up to 100 points, halving every ten days.
pub fn recency(ts: i64, now: i64) -> f64 {
    if ts <= 0 {
        return 0.0;
    }
    let days = ((now - ts).max(0) as f64) / 86_400.0;
    tier::RECENCY_MAX * 0.5f64.powf(days / 10.0)
}

/// The group order of `PALETTE_GROUPS` (commands first, files last).
fn kind_order(k: Kind) -> u8 {
    match k {
        Kind::Command => 0,
        Kind::Session => 1,
        Kind::Note => 2,
        Kind::Message => 3,
        Kind::Event => 4,
        Kind::Chat => 5,
        Kind::Automation => 6,
        Kind::Setting => 7,
        Kind::File => 8,
    }
}

fn body_hits(p: &Palette, text: &str) -> Vec<(Kind, String)> {
    if text.chars().count() < 3 {
        return Vec::new();
    }
    let notes = build(text, Fts::Notes, true).and_then(|q| q.match_expr);
    let msgs = build(text, Fts::Messages, true).and_then(|q| q.match_expr);
    p.db()
        .read(|c| {
            let mut out = Vec::new();
            if let Some(m) = notes {
                let mut stmt = c.prepare_cached(
                    "SELECT n.id FROM notes_fts JOIN notes n ON n.seq = notes_fts.rowid
                      WHERE notes_fts MATCH ?1 AND n.deleted_at IS NULL AND n.archived_at IS NULL
                      ORDER BY bm25(notes_fts, 4.0, 1.0, 2.0, 0.0) LIMIT 20",
                )?;
                for id in stmt.query_map([&m], |r| r.get::<_, String>(0))? {
                    out.push((Kind::Note, id?));
                }
            }
            if let Some(m) = msgs {
                let mut stmt = c.prepare_cached(
                    "SELECT m.id FROM messages_fts JOIN messages m ON m.seq = messages_fts.rowid
                      WHERE messages_fts MATCH ?1 AND m.is_archived = 0
                      ORDER BY bm25(messages_fts, 3.0, 1.0, 1.0, 2.0) LIMIT 20",
                )?;
                for id in stmt.query_map([&m], |r| r.get::<_, String>(0))? {
                    out.push((Kind::Message, id?));
                }
            }
            Ok(out)
        })
        .unwrap_or_default()
}

/// `palette_query`: ranked, synchronous.
pub fn palette_query(p: &Palette, args: QueryArgs) -> ApiResult<Vec<Hit>> {
    let index = p.index()?;
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let kinds: Option<Vec<Kind>> = args
        .kinds
        .as_ref()
        .map(|ks| ks.iter().filter_map(|k| Kind::parse(k)).collect());
    let allowed = |k: Kind| kinds.as_ref().is_none_or(|ks| ks.contains(&k));
    let now = p.now().unix_timestamp();
    let q = args.text.trim().to_lowercase();
    let mut scored: Vec<(f64, usize)> = Vec::new();
    if q.is_empty() {
        // The recent items: anything with activity; messages, events and
        // files only when asked for by kind.
        let quiet =
            |k: Kind| kinds.is_none() && matches!(k, Kind::Message | Kind::Event | Kind::File);
        for (i, e) in index.entries.iter().enumerate() {
            if e.ts > 0 && allowed(e.kind) && !quiet(e.kind) {
                scored.push((recency(e.ts, now), i));
            }
        }
    } else {
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let compact: Vec<u8> = q.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        let q_mask = char_mask(&q);
        let mut seen: HashMap<usize, usize> = HashMap::new();
        for (i, e) in index.entries.iter().enumerate() {
            if !allowed(e.kind) {
                continue;
            }
            if e.title_mask & q_mask != q_mask && e.keywords_mask & q_mask != q_mask {
                continue;
            }
            let s = score_masked(e, &q, &tokens, &compact, q_mask);
            if s > 0.0 {
                seen.insert(i, scored.len());
                scored.push((s + recency(e.ts, now), i));
            }
        }
        let wants_bodies = allowed(Kind::Note) || allowed(Kind::Message);
        if wants_bodies {
            let hits = body_hits(p, args.text.trim());
            if !hits.is_empty() {
                for (k, id) in hits {
                    if !allowed(k) {
                        continue;
                    }
                    if let Some(&i) = index.by_id.get(&(k, id))
                        && !seen.contains_key(&i)
                    {
                        seen.insert(i, scored.len());
                        scored.push((tier::BODY + recency(index.entries[i].ts, now), i));
                    }
                }
            }
        }
    }
    let by = |a: &(f64, usize), b: &(f64, usize)| {
        let (ea, eb) = (&index.entries[a.1], &index.entries[b.1]);
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(kind_order(ea.kind).cmp(&kind_order(eb.kind)))
            .then(ea.title.len().cmp(&eb.title.len()))
            .then(ea.title.cmp(&eb.title))
    };
    if scored.len() > limit {
        scored.select_nth_unstable_by(limit - 1, by);
        scored.truncate(limit);
    }
    scored.sort_by(by);
    Ok(scored
        .into_iter()
        .map(|(score, i)| {
            let e = &index.entries[i];
            Hit {
                kind: e.kind.as_str().into(),
                id: e.id.clone(),
                title: e.title.clone(),
                // A relative time is only true when shown: not the build's.
                detail: match e.kind {
                    Kind::Message => received_label(e.ts, now, p.tz()),
                    _ => e.detail.clone(),
                },
                route: e.route.clone(),
                score: (score * 10.0).round() / 10.0,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(title: &str) -> Entry {
        Entry::new(Kind::Note, "x", title, "", "notes/x", "", 0)
    }

    fn sc(title: &str, q: &str) -> f64 {
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let compact: Vec<u8> = q.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        score_text(&e(title), q, &tokens, &compact)
    }

    #[test]
    fn tiers_order_prefix_word_substring_fuzzy() {
        let exact = sc("buy milk", "buy milk");
        let prefix = sc("buy milk today", "buy mi");
        let word = sc("today buy milk", "milk buy");
        let sub = sc("rebuy", "buy");
        let fuzz = sc("bring your umbrella", "byu");
        assert!(exact > prefix && prefix > word && word > sub && sub > fuzz && fuzz > 0.0);
        assert_eq!(sc("plan", "zzz"), 0.0);
        assert!(
            word + tier::RECENCY_MAX < prefix,
            "recency never crosses a tier"
        );
        assert!(fuzz + tier::RECENCY_MAX < sub);
    }

    #[test]
    fn recency_halves_every_ten_days() {
        let now = 1_000_000_000;
        assert!((recency(now, now) - 100.0).abs() < 1e-9);
        assert!((recency(now - 10 * 86_400, now) - 50.0).abs() < 1e-6);
        assert_eq!(recency(0, now), 0.0);
    }
}
