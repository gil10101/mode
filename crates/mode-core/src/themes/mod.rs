//! Theme packs (10 §3–§5): the pack format and its validator, the built-in
//! packs seeded by migration and refreshed on every start, import / export,
//! and the resolver that turns the active pack, the OS appearance and the
//! `theme.*` settings into the flat `--m-*` token map both webviews apply
//! (and the 21 `--m-terminal-*` tokens the Terminal tab reads first).
//!
//! * [`builtin`]: the seven `themes` rows (System holds System Light and
//!   System Dark) with the §5.9 roles, §5.10 avatar hues and §5.11 terminal
//!   palettes; [`builtin::reseed`] upserts the full documents.
//! * [`validate`]: §4.3 checks; every message names its JSON path.
//! * [`resolve`]: §3.1 resolution order, §4.4 light / dark mapping, the
//!   derived tokens, §11.5 accessibility overrides.
//! * [`api`]: one function per command plus the [`Themes`] service that
//!   computes the `theme.changed` payload.
//!
//! Paper's terminal palette is the §4.2 document (the normative Paper
//! list); §5.11's Paper row predates the current Paper roles and differs.

pub mod api;
pub mod builtin;
pub mod color;
pub mod resolve;
pub mod validate;

pub use api::Themes;
pub use resolve::{AppearancePref, OsAppearance, ResolvedTheme};
pub use validate::{Issue, Report};
