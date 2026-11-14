//! Seeds (09 §12): the premade groupings, premade alert rules and built-in
//! theme headers that `0001_initial.sql` inserts, held here a second time so
//! they can be re-applied on every start ("upserted by slug on each start").
//! Premade rows use fixed ids so every device agrees on them; a user edit
//! (`rev > 1`) or a tombstone is never overwritten, and a missing row (a newer
//! build adding one) is inserted. Built-in theme rows are not user data and are
//! refreshed unconditionally.
//!
//! [`SeedContext`] also carries the environment-derived values the migration
//! text refers to through `__TOKENS__`.

use rusqlite::{Connection, OptionalExtension, params};

use super::{Result, now_rfc3339};

/// Values substituted into migration text before execution.
#[derive(Debug, Clone)]
pub struct SeedContext {
    pub device_id: String,
    pub device_name: String,
    pub profile_name: String,
    pub capture_hotkey: String,
    pub toggle_hotkey: String,
    pub pin_hotkey: String,
}

impl SeedContext {
    /// Detect everything but the device id from the environment.
    pub fn detect(device_id: String) -> Self {
        use crate::settings::defaults;
        Self {
            device_id,
            device_name: defaults::device_name(),
            profile_name: defaults::profile_name(),
            capture_hotkey: defaults::capture_hotkey().to_string(),
            toggle_hotkey: defaults::toggle_hotkey().to_string(),
            pin_hotkey: defaults::pin_hotkey().to_string(),
        }
    }

    /// Replace the `__TOKENS__` of a migration; values are escaped for a
    /// single-quoted SQL literal.
    pub fn render(&self, sql: &str) -> String {
        let esc = |s: &str| s.replace('\'', "''");
        sql.replace("__DEVICE_ID__", &esc(&self.device_id))
            .replace("__DEVICE_NAME__", &esc(&self.device_name))
            .replace("__PROFILE_NAME__", &esc(&self.profile_name))
            .replace("__CAPTURE_HOTKEY__", &esc(&self.capture_hotkey))
            .replace("__TOGGLE_HOTKEY__", &esc(&self.toggle_hotkey))
            .replace("__PIN_HOTKEY__", &esc(&self.pin_hotkey))
    }
}

/// A premade grouping (09 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupingSeed {
    pub id: &'static str,
    pub name: &'static str,
    pub slug: &'static str,
    pub filter_json: &'static str,
    pub sort_json: &'static str,
    pub glyph: &'static str,
    pub position: i64,
    pub show_in_overlay: bool,
}

pub const DEFAULT_SORT: &str = r#"[{"by":"updated_at","dir":"desc"}]"#;
const DUE_SORT: &str = r#"[{"by":"urgent","dir":"desc"},{"by":"when_date","dir":"asc"},{"by":"updated_at","dir":"desc"}]"#;
const URGENT_SORT: &str = r#"[{"by":"when_date","dir":"asc"},{"by":"updated_at","dir":"desc"}]"#;

/// The six premade groupings; Today and Urgent show in the overlay (02 §7.3).
pub const GROUPINGS: &[GroupingSeed] = &[
    GroupingSeed {
        id: "00000000-0000-7000-8000-000000000101",
        name: "Todo",
        slug: "todo",
        filter_json: r#"{"status":{"in":["todo"]}}"#,
        sort_json: DEFAULT_SORT,
        glyph: "○",
        position: 0,
        show_in_overlay: false,
    },
    GroupingSeed {
        id: "00000000-0000-7000-8000-000000000102",
        name: "Doing",
        slug: "doing",
        filter_json: r#"{"status":{"in":["doing"]}}"#,
        sort_json: DEFAULT_SORT,
        glyph: "◑",
        position: 1,
        show_in_overlay: false,
    },
    GroupingSeed {
        id: "00000000-0000-7000-8000-000000000103",
        name: "Urgent",
        slug: "urgent",
        filter_json: r#"{"urgent":true,"status":{"not_in":["closed"]}}"#,
        sort_json: URGENT_SORT,
        glyph: "✱",
        position: 2,
        show_in_overlay: true,
    },
    GroupingSeed {
        id: "00000000-0000-7000-8000-000000000104",
        name: "Done",
        slug: "done",
        filter_json: r#"{"status":{"in":["done"]}}"#,
        sort_json: DEFAULT_SORT,
        glyph: "✓",
        position: 3,
        show_in_overlay: false,
    },
    GroupingSeed {
        id: "00000000-0000-7000-8000-000000000105",
        name: "Today",
        slug: "today",
        filter_json: r#"{"when":{"in":["overdue","today"]},"status":{"not_in":["closed"]}}"#,
        sort_json: DUE_SORT,
        glyph: "▦",
        position: 4,
        show_in_overlay: true,
    },
    GroupingSeed {
        id: "00000000-0000-7000-8000-000000000106",
        name: "Tomorrow",
        slug: "tomorrow",
        filter_json: r#"{"when":{"in":["tomorrow"]},"status":{"not_in":["closed"]}}"#,
        sort_json: DUE_SORT,
        glyph: "▦",
        position: 5,
        show_in_overlay: false,
    },
];

/// A premade alert rule (07 §8.2; 09 §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertRuleSeed {
    pub id: &'static str,
    pub name: &'static str,
    pub enabled: bool,
    pub trigger_json: &'static str,
    pub sound: Option<&'static str>,
    pub cooldown_seconds: i64,
    pub position: i64,
}

pub const ALERT_RULES: &[AlertRuleSeed] = &[
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000201",
        name: "Session needs you",
        enabled: true,
        trigger_json: r#"{"event":"session.waiting","filters":{"waiting_kind":["permission","question","elicitation","agent_needs_input"]}}"#,
        sound: Some("waiting"),
        cooldown_seconds: 0,
        position: 0,
    },
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000202",
        name: "Session failed",
        enabled: true,
        trigger_json: r#"{"event":"session.failed","filters":{}}"#,
        sound: Some("failed"),
        cooldown_seconds: 0,
        position: 1,
    },
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000203",
        name: "Session finished",
        enabled: true,
        trigger_json: r#"{"event":"session.finished","filters":{"min_duration_seconds":0}}"#,
        sound: Some("finished"),
        cooldown_seconds: 0,
        position: 2,
    },
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000204",
        name: "Urgent message",
        enabled: true,
        trigger_json: r#"{"event":"message.received","filters":{"tier":["vip","direct","ai-urgent"]}}"#,
        sound: Some("urgent"),
        cooldown_seconds: 120,
        position: 3,
    },
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000205",
        name: "Meeting starting",
        enabled: true,
        trigger_json: r#"{"event":"calendar.starting","filters":{"minutes_before":5,"requires_join_url":false}}"#,
        sound: Some("starting"),
        cooldown_seconds: 0,
        position: 4,
    },
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000206",
        name: "Due today",
        enabled: true,
        trigger_json: r#"{"event":"note.due","filters":{"at":"start_of_day","urgent_only":false}}"#,
        sound: None,
        cooldown_seconds: 0,
        position: 5,
    },
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000207",
        name: "AI budget at 80 %",
        enabled: false,
        trigger_json: r#"{"event":"usage.threshold","filters":{"percent":80}}"#,
        sound: None,
        cooldown_seconds: 0,
        position: 6,
    },
    AlertRuleSeed {
        id: "00000000-0000-7000-8000-000000000208",
        name: "Automation failed",
        enabled: true,
        trigger_json: r#"{"event":"automation.failed","filters":{}}"#,
        sound: None,
        cooldown_seconds: 0,
        position: 7,
    },
];

/// A built-in theme pack header (10 §5); the token document is the loader's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeSeed {
    pub id: &'static str,
    pub slug: &'static str,
    pub name: &'static str,
    pub appearance: &'static str,
}

pub const THEMES: &[ThemeSeed] = &[
    ThemeSeed {
        id: "00000000-0000-7000-8000-000000000301",
        slug: "system",
        name: "System",
        appearance: "auto",
    },
    ThemeSeed {
        id: "00000000-0000-7000-8000-000000000302",
        slug: "paper",
        name: "Paper",
        appearance: "auto",
    },
    ThemeSeed {
        id: "00000000-0000-7000-8000-000000000303",
        slug: "gruvbox-dark",
        name: "Gruvbox Dark",
        appearance: "dark",
    },
    ThemeSeed {
        id: "00000000-0000-7000-8000-000000000304",
        slug: "gruvbox-light",
        name: "Gruvbox Light",
        appearance: "light",
    },
    ThemeSeed {
        id: "00000000-0000-7000-8000-000000000305",
        slug: "cursor-dark",
        name: "Cursor Dark",
        appearance: "dark",
    },
    ThemeSeed {
        id: "00000000-0000-7000-8000-000000000306",
        slug: "nord",
        name: "Nord",
        appearance: "dark",
    },
    ThemeSeed {
        id: "00000000-0000-7000-8000-000000000307",
        slug: "catppuccin-mocha",
        name: "Catppuccin Mocha",
        appearance: "dark",
    },
];

impl ThemeSeed {
    /// The header document stored in `tokens_json` until the theme loader fills the pack.
    pub fn tokens_json(&self) -> String {
        serde_json::json!({
            "format": 1,
            "slug": self.slug,
            "name": self.name,
            "author": "Mode",
            "version": "1",
            "appearance": self.appearance,
        })
        .to_string()
    }
}

/// `(rev, deleted_at, name, filter_json, sort_json, glyph, position, show_in_overlay)` of a stored grouping.
type GroupingState = (
    i64,
    Option<String>,
    String,
    String,
    String,
    String,
    i64,
    bool,
);
/// `(id, rev, deleted_at, name, enabled, trigger_json, sound, cooldown_seconds, position)` of a stored rule.
type AlertRuleState = (
    String,
    i64,
    Option<String>,
    String,
    bool,
    String,
    Option<String>,
    i64,
    i64,
);

/// What a refresh did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RefreshReport {
    pub groupings_inserted: usize,
    pub groupings_updated: usize,
    pub alert_rules_inserted: usize,
    pub alert_rules_updated: usize,
    pub themes_inserted: usize,
    pub themes_updated: usize,
}

/// Re-apply the seeds: insert missing premade rows, refresh untouched ones
/// (`rev = 1`, not tombstoned) whose definition changed, always refresh
/// built-in themes.
pub fn refresh(conn: &Connection, ctx: &SeedContext) -> Result<RefreshReport> {
    let now = now_rfc3339();
    let mut report = RefreshReport::default();

    for g in GROUPINGS {
        let existing: Option<GroupingState> = conn
            .query_row(
                "SELECT rev, deleted_at, name, filter_json, sort_json, COALESCE(glyph, ''), position, show_in_overlay
                   FROM groupings WHERE id = ?1",
                [g.id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )
            .optional()?;
        match existing {
            None => {
                let slug_taken: bool = conn
                    .query_row(
                        "SELECT 1 FROM groupings WHERE slug = ?1 AND deleted_at IS NULL",
                        [g.slug],
                        |_| Ok(true),
                    )
                    .optional()?
                    .unwrap_or(false);
                if slug_taken {
                    continue;
                }
                conn.execute(
                    "INSERT INTO groupings (id, name, slug, kind, filter_json, sort_json, glyph, color, position,
                                            show_in_overlay, show_in_sidebar, created_at, updated_at, rev, device_id, deleted_at)
                     VALUES (?1, ?2, ?3, 'premade', ?4, ?5, ?6, NULL, ?7, ?8, 1, ?9, ?9, 1, ?10, NULL)",
                    params![
                        g.id,
                        g.name,
                        g.slug,
                        g.filter_json,
                        g.sort_json,
                        g.glyph,
                        g.position,
                        g.show_in_overlay,
                        now,
                        ctx.device_id
                    ],
                )?;
                report.groupings_inserted += 1;
            }
            Some((1, None, name, filter, sort, glyph, position, overlay)) => {
                let same = name == g.name
                    && filter == g.filter_json
                    && sort == g.sort_json
                    && glyph == g.glyph
                    && position == g.position
                    && overlay == g.show_in_overlay;
                if !same {
                    conn.execute(
                        "UPDATE groupings SET name = ?2, filter_json = ?3, sort_json = ?4, glyph = ?5, position = ?6,
                                              show_in_overlay = ?7
                          WHERE id = ?1 AND rev = 1 AND deleted_at IS NULL",
                        params![
                            g.id,
                            g.name,
                            g.filter_json,
                            g.sort_json,
                            g.glyph,
                            g.position,
                            g.show_in_overlay
                        ],
                    )?;
                    report.groupings_updated += 1;
                }
            }
            Some(_) => {}
        }
    }

    // Premade rules are upserted by name (07 §8.2): the seed's id, else a
    // live rule with the seed's name (a restored or merged database keeps
    // its own id) is the premade row; edited rows (`rev > 1`) are left alone.
    for a in ALERT_RULES {
        let existing: Option<AlertRuleState> = conn.query_row(
                "SELECT id, rev, deleted_at, name, enabled, trigger_json, sound, cooldown_seconds, position
                   FROM alert_rules
                  WHERE id = ?1 OR (name = ?2 AND deleted_at IS NULL)
                  ORDER BY id = ?1 DESC LIMIT 1",
                params![a.id, a.name],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .optional()?;
        match existing {
            None => {
                conn.execute(
                    "INSERT INTO alert_rules (id, name, kind, enabled, trigger_json, channels_json, sound, respect_dnd, quiet_hours,
                                              cooldown_seconds, position, last_fired_at, created_at, updated_at, rev, device_id, deleted_at)
                     VALUES (?1, ?2, 'premade', ?3, ?4, '[\"overlay\"]', ?5, 1, 1, ?6, ?7, NULL, ?8, ?8, 1, ?9, NULL)",
                    params![
                        a.id,
                        a.name,
                        a.enabled,
                        a.trigger_json,
                        a.sound,
                        a.cooldown_seconds,
                        a.position,
                        now,
                        ctx.device_id
                    ],
                )?;
                report.alert_rules_inserted += 1;
            }
            Some((id, 1, None, name, enabled, trigger, sound, cooldown, position)) => {
                let same = name == a.name
                    && enabled == a.enabled
                    && trigger == a.trigger_json
                    && sound.as_deref() == a.sound
                    && cooldown == a.cooldown_seconds
                    && position == a.position;
                if !same {
                    conn.execute(
                        "UPDATE alert_rules SET name = ?2, enabled = ?3, trigger_json = ?4, sound = ?5,
                                                cooldown_seconds = ?6, position = ?7
                          WHERE id = ?1 AND rev = 1 AND deleted_at IS NULL",
                        params![
                            id,
                            a.name,
                            a.enabled,
                            a.trigger_json,
                            a.sound,
                            a.cooldown_seconds,
                            a.position
                        ],
                    )?;
                    report.alert_rules_updated += 1;
                }
            }
            Some(_) => {}
        }
    }

    for t in THEMES {
        let existing: Option<(Option<String>, String, String, String)> = conn
            .query_row(
                "SELECT deleted_at, name, appearance, tokens_json FROM themes WHERE id = ?1",
                [t.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let tokens = t.tokens_json();
        match existing {
            None => {
                let slug_taken: bool = conn
                    .query_row(
                        "SELECT 1 FROM themes WHERE slug = ?1 AND deleted_at IS NULL",
                        [t.slug],
                        |_| Ok(true),
                    )
                    .optional()?
                    .unwrap_or(false);
                if slug_taken {
                    continue;
                }
                conn.execute(
                    "INSERT INTO themes (id, slug, name, kind, appearance, version, author, tokens_json, source_path,
                                         created_at, updated_at, rev, device_id, deleted_at)
                     VALUES (?1, ?2, ?3, 'builtin', ?4, '1', 'Mode', ?5, NULL, ?6, ?6, 1, ?7, NULL)",
                    params![t.id, t.slug, t.name, t.appearance, tokens, now, ctx.device_id],
                )?;
                report.themes_inserted += 1;
            }
            Some((None, name, appearance, stored)) => {
                if name != t.name || appearance != t.appearance || stored != tokens {
                    conn.execute(
                        "UPDATE themes SET name = ?2, appearance = ?3, tokens_json = ?4 WHERE id = ?1 AND kind = 'builtin'",
                        params![t.id, t.name, t.appearance, tokens],
                    )?;
                    report.themes_updated += 1;
                }
            }
            Some(_) => {}
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_and_escapes_tokens() {
        let ctx = SeedContext {
            device_id: "dev-1".into(),
            device_name: "O'Brien's Mac".into(),
            profile_name: "Pat".into(),
            capture_hotkey: "Alt+Space".into(),
            toggle_hotkey: "Alt+Shift+Space".into(),
            pin_hotkey: "Alt+Shift+P".into(),
        };
        let out = ctx.render("x '__DEVICE_ID__' json_quote('__DEVICE_NAME__') '__PROFILE_NAME__'");
        assert_eq!(out, "x 'dev-1' json_quote('O''Brien''s Mac') 'Pat'");
    }

    #[test]
    fn seed_ids_are_fixed_valid_uuids() {
        for id in GROUPINGS
            .iter()
            .map(|g| g.id)
            .chain(ALERT_RULES.iter().map(|a| a.id))
            .chain(THEMES.iter().map(|t| t.id))
        {
            assert!(uuid::Uuid::parse_str(id).is_ok(), "{id}");
        }
        assert_eq!(GROUPINGS.len(), 6);
        assert_eq!(ALERT_RULES.len(), 8);
        assert_eq!(THEMES.len(), 7);
    }
}
