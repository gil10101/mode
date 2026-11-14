//! The command palette index (03 §12 assumption 35, 10 §7.4.15; brief
//! §7.11): one ranked list over sessions, notes, messages, automations,
//! Settings pages and keys, views and commands. The index is built from the
//! tables (titles, recency) and the settings registry and kept in memory;
//! notes and messages also match on their bodies through the FTS5 tables.
//! A query is synchronous and stays under 10 ms on a 10 000-row database
//! once the index is warm (`tests/palette.rs`).
//!
//! Ranking: an exact title, then a title prefix, then every query word
//! starting a title word, then a substring, then a fuzzy subsequence, then
//! a match in the detail line or keywords, then a body hit; a recency boost
//! of at most 100 points (half-life about ten days) orders items inside a
//! tier but never lifts one across a tier (tiers are ≥ 150 points apart).

pub mod api;
pub mod index;

pub use api::{Hit, QueryArgs, palette_query};
pub use index::Palette;
