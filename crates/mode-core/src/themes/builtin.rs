//! The built-in theme packs of 10 §5 as data: the seven `themes` rows seeded
//! by `0001_initial.sql` (System carries the two System packs, §5.1 System
//! Light and §5.2 System Dark, as its light and dark sets), each with the
//! fifteen roles of §5.9, the avatar hues of §5.10 and the terminal palettes
//! of §5.11. Paper's document is §4.2 verbatim (its terminal palette is the
//! §4.2 one; see the module docs of [`super`]). [`reseed`] upserts the full
//! documents by id with `kind = 'builtin'` on every start (10 §4.5), so an
//! update refreshes them; built-in rows never reach `change_log`.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::db::Result;

/// The fifteen color roles of 10 §3.2, in table order, keyed without the prefix.
pub const ROLES: [&str; 15] = [
    "bg",
    "surface",
    "sunken",
    "hover",
    "selected",
    "line",
    "line-strong",
    "fg",
    "fg-2",
    "fg-3",
    "accent",
    "accent-bg",
    "ok",
    "bad",
    "info",
];

/// ANSI names of a terminal palette (10 §5.11 order).
pub const ANSI: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-magenta",
    "bright-cyan",
    "bright-white",
];

/// The UI stack of 10 §3.3.
pub const FONT_UI: &str =
    r#"-apple-system, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif"#;
/// The mono stack of 10 §3.3.
pub const FONT_MONO: &str =
    r#""SF Mono", Menlo, "Cascadia Mono", Consolas, "DejaVu Sans Mono", ui-monospace, monospace"#;
/// Paper's mono stack as written in 10 §4.2.
pub const PAPER_MONO: &str =
    r#""SF Mono", Menlo, "Cascadia Mono", Consolas, ui-monospace, monospace"#;
/// `shadow.pop` defaults (10 §3.6).
pub const SHADOW_LIGHT: &str = "0 8px 24px rgba(0,0,0,.14)";
pub const SHADOW_DARK: &str = "0 8px 24px rgba(0,0,0,.5)";
/// The OS-accent fallbacks of the System packs (10 §5.1, §5.2).
pub const SYSTEM_ACCENT_LIGHT: &str = "#0066CC";
pub const SYSTEM_ACCENT_DARK: &str = "#0A84FF";

/// One color set: roles, avatars, terminal (16 ANSI then fg / bg / cursor / selection).
pub struct Set {
    pub m: [&'static str; 15],
    pub avatar: [&'static str; 12],
    pub ansi: [&'static str; 16],
    pub term: [&'static str; 4],
}

/// One built-in pack.
pub struct Builtin {
    pub id: &'static str,
    pub slug: &'static str,
    pub name: &'static str,
    pub appearance: &'static str,
    pub version: &'static str,
    pub light: Option<Set>,
    pub dark: Option<Set>,
}

const SYSTEM_LIGHT: Set = Set {
    m: [
        "#F5F5F7",
        "#FFFFFF",
        "#ECECEE",
        "#EAEAEC",
        "#E4E4E8",
        "#D9D9DE",
        "#B9B9BE",
        "#1D1D1F",
        "#515154",
        "#6E6E73",
        SYSTEM_ACCENT_LIGHT,
        "#E5F0FF",
        "#1F8A3B",
        "#C62828",
        "#0066CC",
    ],
    avatar: [
        "#E0553F", "#E07B2E", "#C99A1E", "#7FA832", "#2FA36B", "#1F9E98", "#2389C9", "#4D6FE3",
        "#7B5CD6", "#B04FBF", "#D04C98", "#D2546A",
    ],
    ansi: [
        "#000000", "#C91B00", "#00C200", "#C7C400", "#0225C7", "#CA30C7", "#00C5C7", "#C7C7C7",
        "#686868", "#FF6E67", "#5FFA68", "#FFFC67", "#6871FF", "#FF77FF", "#60FDFF", "#FFFFFF",
    ],
    term: ["#000000", "#FFFFFF", "#4D4D4D", "#B4D5FF"],
};

const SYSTEM_DARK: Set = Set {
    m: [
        "#1E1E20",
        "#2A2A2D",
        "#161618",
        "#333336",
        "#3D3D41",
        "#3A3A3E",
        "#525257",
        "#F5F5F7",
        "#C7C7CC",
        "#98989D",
        SYSTEM_ACCENT_DARK,
        "#1C2E4A",
        "#30D158",
        "#FF453A",
        "#0A84FF",
    ],
    avatar: [
        "#FF6B5B", "#FF9452", "#E6B84D", "#9ACB5B", "#5CC28E", "#4FC2BA", "#5BA9E6", "#7A94F2",
        "#A08AE8", "#C97CD9", "#E27AB5", "#EE7B8D",
    ],
    ansi: [
        "#0C0C0C", "#C50F1F", "#13A10E", "#C19C00", "#0037DA", "#881798", "#3A96DD", "#CCCCCC",
        "#767676", "#E74856", "#16C60C", "#F9F1A5", "#3B78FF", "#B4009E", "#61D6D6", "#F2F2F2",
    ],
    term: ["#CCCCCC", "#0C0C0C", "#FFFFFF", "#264F78"],
};

const PAPER_LIGHT: Set = Set {
    m: [
        "#F3F1EC", "#FAF9F6", "#ECE9E2", "#EFECE6", "#E9E5DC", "#E1DDD4", "#C9C4B8", "#1E1D1A",
        "#5B5952", "#8A877E", "#D9480F", "#FBECE3", "#2F7D4A", "#B3261E", "#35669A",
    ],
    avatar: [
        "#C0392B", "#D35400", "#B9770E", "#7D8C2A", "#3F7D4E", "#2E8B7A", "#2F6F9F", "#4F5FA8",
        "#6E4FA3", "#8E4A8E", "#B0497A", "#B85C5C",
    ],
    ansi: [
        "#1E1D1A", "#B3261E", "#2F7D4A", "#B9770E", "#35669A", "#7B4EA3", "#2E8B7A", "#C9C4B8",
        "#8A877E", "#E74C3C", "#4E9A5F", "#D28A1A", "#3F86BD", "#9366C0", "#3AA58F", "#FAF9F6",
    ],
    term: ["#1E1D1A", "#FAF9F6", "#D9480F", "#E9E5DC"],
};

const PAPER_DARK: Set = Set {
    m: [
        "#1A1917", "#22211E", "#141311", "#2A2925", "#33312C", "#3A3833", "#58554D", "#ECEAE4",
        "#B3B0A7", "#85827A", "#F0652A", "#3A2418", "#5DB37A", "#E5635A", "#6E9FD1",
    ],
    avatar: [
        "#FF6B5B", "#FF9452", "#E6B84D", "#9ACB5B", "#5CC28E", "#4FC2BA", "#5BA9E6", "#7A94F2",
        "#A08AE8", "#C97CD9", "#E27AB5", "#EE7B8D",
    ],
    ansi: [
        "#1A1917", "#E5635A", "#5DB37A", "#D9A441", "#6E9FD1", "#B48EAD", "#5FB3A3", "#B3B0A7",
        "#85827A", "#F2857C", "#7FCB97", "#E8BC63", "#8FB6E0", "#C9A7C4", "#7FC7BA", "#ECEAE4",
    ],
    term: ["#ECEAE4", "#1A1917", "#F0652A", "#33312C"],
};

const GRUVBOX_DARK: Set = Set {
    m: [
        "#282828", "#32302f", "#1d2021", "#3c3836", "#504945", "#3c3836", "#665c54", "#ebdbb2",
        "#d5c4a1", "#a89984", "#fe8019", "#3d2a1c", "#b8bb26", "#fb4934", "#83a598",
    ],
    avatar: [
        "#fb4934", "#fe8019", "#fabd2f", "#b8bb26", "#8ec07c", "#83a598", "#d3869b", "#cc241d",
        "#d65d0e", "#d79921", "#98971a", "#689d6a",
    ],
    ansi: [
        "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
        "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
    ],
    term: ["#ebdbb2", "#282828", "#ebdbb2", "#504945"],
};

const GRUVBOX_LIGHT: Set = Set {
    m: [
        "#fbf1c7", "#f9f5d7", "#f2e5bc", "#ebdbb2", "#d5c4a1", "#ebdbb2", "#bdae93", "#3c3836",
        "#504945", "#665c54", "#af3a03", "#f2dcc9", "#79740e", "#9d0006", "#076678",
    ],
    avatar: [
        "#9d0006", "#af3a03", "#b57614", "#79740e", "#427b58", "#076678", "#8f3f71", "#cc241d",
        "#d65d0e", "#d79921", "#98971a", "#689d6a",
    ],
    ansi: [
        "#fbf1c7", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#7c6f64",
        "#928374", "#9d0006", "#79740e", "#b57614", "#076678", "#8f3f71", "#427b58", "#3c3836",
    ],
    term: ["#3c3836", "#fbf1c7", "#3c3836", "#d5c4a1"],
};

const CURSOR_DARK: Set = Set {
    m: [
        "#171717", "#1F1F1F", "#111111", "#262626", "#2A2D35", "#2E2E2E", "#3D3D3D", "#E6E6E6",
        "#B3B3B3", "#8C8C8C", "#F0883E", "#3A2A1C", "#3FB950", "#F85149", "#5B8DEF",
    ],
    avatar: [
        "#F87171", "#FB923C", "#FACC15", "#A3E635", "#34D399", "#2DD4BF", "#38BDF8", "#818CF8",
        "#A78BFA", "#E879F9", "#F472B6", "#FB7185",
    ],
    ansi: [
        "#171717", "#F85149", "#3FB950", "#D29922", "#5B8DEF", "#BC8CFF", "#39C5CF", "#B3B3B3",
        "#4D4D4D", "#FF7B72", "#56D364", "#E3B341", "#79C0FF", "#D2A8FF", "#56D4DD", "#E6E6E6",
    ],
    term: ["#E6E6E6", "#171717", "#E6E6E6", "#2A2D35"],
};

const NORD: Set = Set {
    m: [
        "#2E3440", "#3B4252", "#272C36", "#434C5E", "#4C566A", "#434C5E", "#5E6779", "#ECEFF4",
        "#D8DEE9", "#A8B2C5", "#D08770", "#4A3B38", "#A3BE8C", "#BF616A", "#81A1C1",
    ],
    avatar: [
        "#BF616A", "#D08770", "#EBCB8B", "#A3BE8C", "#8FBCBB", "#88C0D0", "#81A1C1", "#5E81AC",
        "#B48EAD", "#C9826B", "#9DB9A0", "#A0A8C8",
    ],
    ansi: [
        "#3B4252", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#88C0D0", "#E5E9F0",
        "#4C566A", "#BF616A", "#A3BE8C", "#EBCB8B", "#81A1C1", "#B48EAD", "#8FBCBB", "#ECEFF4",
    ],
    term: ["#D8DEE9", "#2E3440", "#D8DEE9", "#434C5E"],
};

const CATPPUCCIN_MOCHA: Set = Set {
    m: [
        "#1E1E2E", "#313244", "#181825", "#45475A", "#585B70", "#45475A", "#6C7086", "#CDD6F4",
        "#BAC2DE", "#A6ADC8", "#FAB387", "#45352F", "#A6E3A1", "#F38BA8", "#89B4FA",
    ],
    avatar: [
        "#F38BA8", "#FAB387", "#F9E2AF", "#A6E3A1", "#94E2D5", "#89DCEB", "#89B4FA", "#B4BEFE",
        "#CBA6F7", "#F5C2E7", "#EBA0AC", "#74C7EC",
    ],
    ansi: [
        "#45475A", "#F38BA8", "#A6E3A1", "#F9E2AF", "#89B4FA", "#F5C2E7", "#94E2D5", "#BAC2DE",
        "#585B70", "#F38BA8", "#A6E3A1", "#F9E2AF", "#89B4FA", "#F5C2E7", "#94E2D5", "#A6ADC8",
    ],
    term: ["#CDD6F4", "#1E1E2E", "#F5E0DC", "#585B70"],
};

/// The seven built-in rows (ids and slugs of `0001_initial.sql` / `db::seeds`).
pub const BUILTINS: [Builtin; 7] = [
    Builtin {
        id: "00000000-0000-7000-8000-000000000301",
        slug: "system",
        name: "System",
        appearance: "auto",
        version: "1",
        light: Some(SYSTEM_LIGHT),
        dark: Some(SYSTEM_DARK),
    },
    Builtin {
        id: "00000000-0000-7000-8000-000000000302",
        slug: "paper",
        name: "Paper",
        appearance: "auto",
        version: "2.0.0",
        light: Some(PAPER_LIGHT),
        dark: Some(PAPER_DARK),
    },
    Builtin {
        id: "00000000-0000-7000-8000-000000000303",
        slug: "gruvbox-dark",
        name: "Gruvbox Dark",
        appearance: "dark",
        version: "1",
        light: None,
        dark: Some(GRUVBOX_DARK),
    },
    Builtin {
        id: "00000000-0000-7000-8000-000000000304",
        slug: "gruvbox-light",
        name: "Gruvbox Light",
        appearance: "light",
        version: "1",
        light: Some(GRUVBOX_LIGHT),
        dark: None,
    },
    Builtin {
        id: "00000000-0000-7000-8000-000000000305",
        slug: "cursor-dark",
        name: "Cursor Dark",
        appearance: "dark",
        version: "1",
        light: None,
        dark: Some(CURSOR_DARK),
    },
    Builtin {
        id: "00000000-0000-7000-8000-000000000306",
        slug: "nord",
        name: "Nord",
        appearance: "dark",
        version: "1",
        light: None,
        dark: Some(NORD),
    },
    Builtin {
        id: "00000000-0000-7000-8000-000000000307",
        slug: "catppuccin-mocha",
        name: "Catppuccin Mocha",
        appearance: "dark",
        version: "1",
        light: None,
        dark: Some(CATPPUCCIN_MOCHA),
    },
];

/// Slugs a user pack may not take (10 §4.1).
pub fn is_reserved(slug: &str) -> bool {
    BUILTINS.iter().any(|b| b.slug == slug)
}

pub fn by_slug(slug: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.slug == slug)
}

fn set_json(s: &Set) -> Value {
    let mut m = Map::new();
    for (k, v) in ROLES.iter().zip(s.m.iter()) {
        m.insert((*k).into(), json!(v));
    }
    let mut ansi = Map::new();
    for (k, v) in ANSI.iter().zip(s.ansi.iter()) {
        ansi.insert((*k).into(), json!(v));
    }
    json!({
        "m": m,
        "avatar": s.avatar,
        "terminal": {
            "fg": s.term[0], "bg": s.term[1], "cursor": s.term[2], "selection": s.term[3],
            "ansi": ansi,
        },
        "pattern": { "kind": "none" },
    })
}

impl Builtin {
    /// The pack document (10 §4.1) stored in `themes.tokens_json`.
    pub fn document(&self) -> Value {
        let mut colors = Map::new();
        let mut shadow = Map::new();
        if let Some(s) = &self.light {
            colors.insert("light".into(), set_json(s));
            shadow.insert("light".into(), json!({ "pop": SHADOW_LIGHT }));
        }
        if let Some(s) = &self.dark {
            colors.insert("dark".into(), set_json(s));
            shadow.insert("dark".into(), json!({ "pop": SHADOW_DARK }));
        }
        let mut doc = json!({
            "format": 1,
            "slug": self.slug,
            "name": self.name,
            "author": "Mode",
            "version": self.version,
            "appearance": self.appearance,
        });
        match self.slug {
            "system" => {
                doc["typography"] = json!({ "ui": FONT_UI, "mono": FONT_MONO, "scale": 1.0 });
                doc["radius"] = json!({ "control": 6, "card": 10 });
                doc["motion"] = json!({ "scale": 1.0 });
            }
            "paper" => {
                doc["typography"] = json!({ "ui": FONT_UI, "mono": PAPER_MONO, "scale": 1.0 });
                doc["radius"] = json!({ "control": 6, "card": 10 });
                doc["motion"] = json!({ "scale": 1.0 });
            }
            _ => {}
        }
        doc["colors"] = Value::Object(colors);
        doc["shadow"] = Value::Object(shadow);
        if self.slug == "paper" {
            doc["sounds"] = json!({});
        }
        doc
    }

    /// One role of one appearance (`dark = true` for the dark set).
    pub fn role(&self, dark: bool, role: &str) -> Option<&'static str> {
        let set = if dark { &self.dark } else { &self.light };
        let i = ROLES.iter().position(|r| *r == role)?;
        set.as_ref().map(|s| s.m[i])
    }
}

/// The System pack's document (the fallback of every resolution, 10 §3.1).
pub fn system_document() -> Value {
    BUILTINS[0].document()
}

/// What [`reseed`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReseedReport {
    pub inserted: usize,
    pub updated: usize,
}

/// Upsert the built-in documents by id (10 §4.5 "upserted by slug on every
/// start"): a missing row is inserted unless a user pack holds the slug, a
/// stored built-in row whose document, name, appearance or version differs
/// is refreshed. Tombstoned built-in rows are left alone.
pub fn reseed(conn: &Connection, device_id: &str, now: &str) -> Result<ReseedReport> {
    let mut report = ReseedReport::default();
    for b in &BUILTINS {
        let doc = b.document().to_string();
        let existing: Option<(Option<String>, String, String, String, String, String)> = conn
            .query_row(
                "SELECT deleted_at, kind, name, appearance, version, tokens_json FROM themes WHERE id = ?1",
                [b.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        match existing {
            None => {
                let taken = conn
                    .query_row(
                        "SELECT 1 FROM themes WHERE slug = ?1 AND deleted_at IS NULL",
                        [b.slug],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if taken {
                    continue;
                }
                conn.execute(
                    "INSERT INTO themes (id, slug, name, kind, appearance, version, author, tokens_json,
                                         source_path, created_at, updated_at, rev, device_id, deleted_at)
                     VALUES (?1, ?2, ?3, 'builtin', ?4, ?5, 'Mode', ?6, NULL, ?7, ?7, 1, ?8, NULL)",
                    params![b.id, b.slug, b.name, b.appearance, b.version, doc, now, device_id],
                )?;
                report.inserted += 1;
            }
            Some((None, kind, name, appearance, version, stored)) if kind == "builtin" => {
                if name != b.name
                    || appearance != b.appearance
                    || version != b.version
                    || stored != doc
                {
                    conn.execute(
                        "UPDATE themes SET name = ?2, appearance = ?3, version = ?4, tokens_json = ?5
                          WHERE id = ?1 AND kind = 'builtin'",
                        params![b.id, b.name, b.appearance, b.version, doc],
                    )?;
                    report.updated += 1;
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
    fn eight_packs_in_seven_rows() {
        let sets: usize = BUILTINS
            .iter()
            .map(|b| usize::from(b.light.is_some()) + usize::from(b.dark.is_some()))
            .sum();
        // The eight packs of 10 §5 (System Light and System Dark share the
        // `system` row), Paper carrying both appearances: nine color sets.
        assert_eq!(sets, 9);
        assert_eq!(BUILTINS.len(), 7);
        for (b, s) in BUILTINS.iter().zip(crate::db::seeds::THEMES.iter()) {
            assert_eq!(
                (b.id, b.slug, b.name, b.appearance),
                (s.id, s.slug, s.name, s.appearance)
            );
        }
    }

    #[test]
    fn documents_carry_the_declared_sets() {
        for b in &BUILTINS {
            let d = b.document();
            assert_eq!(d["slug"], b.slug);
            assert_eq!(d["colors"].get("light").is_some(), b.light.is_some());
            assert_eq!(d["colors"].get("dark").is_some(), b.dark.is_some());
        }
        assert_eq!(by_slug("nord").unwrap().role(true, "fg-3"), Some("#A8B2C5"));
        assert!(is_reserved("paper") && !is_reserved("paper-2"));
    }
}
