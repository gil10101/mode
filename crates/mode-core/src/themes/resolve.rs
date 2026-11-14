//! The token resolver (10 §3.1): the active pack, the OS appearance, accent
//! and accessibility flags and the `theme.*` settings become one flat map of
//! `--m-*` custom properties that both webviews apply on `:root` (and the
//! Terminal layer reads first). Resolution order, highest first: forced
//! colors → reduce transparency → user settings → active pack → System
//! pack → built-in defaults. The webview never parses a pack.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::builtin::{self, ANSI, ROLES};
use super::color::{self, Rgba};

/// `mode.css`'s control and card radius tokens (the pack's `radius.control`
/// / `radius.card`).
pub const RADIUS_CONTROL: &str = "--m-r-ctl";
pub const RADIUS_CARD: &str = "--m-r-card";

/// What the OS reports (the app feeds it: `NSApp.effectiveAppearance`,
/// `UISettings`, `prefers-color-scheme`, the accessibility flags).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsAppearance {
    /// The OS is in dark mode.
    #[serde(default)]
    pub dark: bool,
    /// The OS accent color (`#rrggbb`), when the OS exposes one.
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub increase_contrast: bool,
    /// Windows high contrast / `forced-colors: active`.
    #[serde(default)]
    pub forced_colors: bool,
    #[serde(default)]
    pub reduce_transparency: bool,
    #[serde(default)]
    pub reduce_motion: bool,
}

/// `theme.appearance`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearancePref {
    #[default]
    Auto,
    Light,
    Dark,
}

impl AppearancePref {
    pub fn parse(s: &str) -> Option<AppearancePref> {
        match s {
            "auto" => Some(AppearancePref::Auto),
            "light" => Some(AppearancePref::Light),
            "dark" => Some(AppearancePref::Dark),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AppearancePref::Auto => "auto",
            AppearancePref::Light => "light",
            AppearancePref::Dark => "dark",
        }
    }
}

/// The derived text color on the accent (10 §3.2 `--m-accent-fg`, §4.3).
#[derive(Debug, Clone, PartialEq)]
pub struct AccentFg {
    pub text: String,
    pub ratio: f64,
    /// The accent after any contrast adjustment.
    pub accent: String,
    pub adjusted: bool,
}

/// `#FFFFFF`, else the appearance's `--m-fg`, else its `--m-bg` (the dark
/// text of 10 §5.8 on a light accent in a dark pack), the first that reaches
/// 4.5:1; when none does, the best one if it reaches the 3:1 floor of §11.1
/// (Paper's 4.3:1 white stays, §3.2); below that the accent is darkened or
/// lightened in OKLCH until a candidate reaches 4.5:1 ("accent adjusted for
/// contrast").
pub fn accent_fg(accent: Rgba, fg: Rgba, bg: Rgba) -> AccentFg {
    let white = Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    let candidates = [white, fg, bg];
    let ratios: Vec<f64> = candidates
        .iter()
        .map(|c| color::contrast(*c, accent))
        .collect();
    if let Some(i) = ratios.iter().position(|r| *r >= 4.5) {
        return AccentFg {
            text: color::to_hex(candidates[i]),
            ratio: ratios[i],
            accent: color::to_hex(accent),
            adjusted: false,
        };
    }
    let (best, ratio) =
        ratios.iter().enumerate().fold(
            (0, 0.0_f64),
            |acc, (i, r)| if *r > acc.1 { (i, *r) } else { acc },
        );
    if ratio >= 3.0 {
        return AccentFg {
            text: color::to_hex(candidates[best]),
            ratio,
            accent: color::to_hex(accent),
            adjusted: false,
        };
    }
    // White text on a darker accent, or dark text on a lighter one.
    let dark_text = if color::luminance(fg) < color::luminance(bg) {
        fg
    } else {
        bg
    };
    let darker = color::adjust_lightness(accent, true, |c| color::contrast(white, c) >= 4.5);
    let lighter = color::adjust_lightness(accent, false, |c| color::contrast(dark_text, c) >= 4.5);
    let dist = |c: Option<Rgba>| {
        c.map(|c| (color::to_oklch(c).0 - color::to_oklch(accent).0).abs())
            .unwrap_or(f64::MAX)
    };
    let (text, adj) = if dist(darker) <= dist(lighter) {
        (white, darker)
    } else {
        (dark_text, lighter)
    };
    let adj = adj.unwrap_or(accent);
    AccentFg {
        text: color::to_hex(text),
        ratio: color::contrast(text, adj),
        accent: color::to_hex(adj),
        adjusted: true,
    }
}

/// The OS accent normalized for contrast (10 §4.4): moved in OKLCH until the
/// state-glyph target of 3:1 on `--m-surface` holds.
pub fn normalise_os_accent(os_accent: &str, surface: &str, dark: bool) -> Option<String> {
    let a = color::parse_hex(os_accent)?;
    let s = color::parse_hex(surface)?;
    if color::contrast(a, s) >= 3.0 {
        return Some(color::to_hex(a));
    }
    color::adjust_lightness(a, !dark, |c| color::contrast(c, s) >= 3.0).map(color::to_hex)
}

/// The resolver's answer: the `theme:changed` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTheme {
    /// The pack the colors came from (`system` when it fell back).
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub name: String,
    /// The active pack the user chose.
    #[serde(default)]
    pub active: String,
    /// `light` | `dark`: the appearance rendered.
    #[serde(default)]
    pub appearance: String,
    /// `[data-theme=dark]` on the webviews.
    pub dark: bool,
    /// `--m-*` → value.
    pub tokens: BTreeMap<String, String>,
    /// The Themes card lines ("Gruvbox Light has no dark set — showing System Dark").
    #[serde(default)]
    pub notices: Vec<String>,
    /// The resolver changed the accent for contrast.
    #[serde(default)]
    pub accent_adjusted: bool,
}

/// Everything [`resolve`] reads.
#[derive(Debug, Clone)]
pub struct Inputs<'a> {
    /// The active pack's document.
    pub pack: &'a Value,
    /// `theme.terminal_pack` when it is not `follow`.
    pub terminal_pack: Option<&'a Value>,
    pub appearance: AppearancePref,
    pub os: &'a OsAppearance,
    /// `theme.font_scale` when the user set it.
    pub font_scale: Option<f64>,
}

fn s<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(Value::as_str)
}

fn num(v: &Value, ptr: &str) -> Option<f64> {
    v.pointer(ptr).and_then(Value::as_f64)
}

fn set_name(slug: &str, name: &str, dark: bool) -> String {
    if slug == "system" {
        format!("System {}", if dark { "Dark" } else { "Light" })
    } else {
        name.to_string()
    }
}

fn fmt_num(n: f64) -> String {
    let r = (n * 1000.0).round() / 1000.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// Resolve to the flat token map (10 §3.1, §4.4, §5.10, §5.11, §11.5).
pub fn resolve(i: &Inputs) -> ResolvedTheme {
    let system = builtin::system_document();
    let dark = match i.appearance {
        AppearancePref::Auto => i.os.dark,
        AppearancePref::Light => false,
        AppearancePref::Dark => true,
    };
    let which = if dark { "dark" } else { "light" };
    let active = s(i.pack, "/slug").unwrap_or("system").to_string();
    let active_name = s(i.pack, "/name").unwrap_or("System").to_string();
    let mut notices = Vec::new();

    // The color source: the pack's set for this appearance, else System's.
    let (src, src_slug) = if i.pack.pointer(&format!("/colors/{which}")).is_some() {
        (i.pack, active.clone())
    } else {
        let sys_name = set_name("system", "System", dark);
        let pack_app = s(i.pack, "/appearance").unwrap_or("light");
        notices.push(match i.appearance {
            AppearancePref::Auto => format!(
                "{active_name} ({pack_app}) · {sys_name} {}",
                if dark { "at night" } else { "by day" }
            ),
            _ => format!("{active_name} has no {which} set — showing {sys_name}"),
        });
        (&system, "system".to_string())
    };
    let set_ptr = format!("/colors/{which}");
    let role = |r: &str| -> String {
        s(src, &format!("{set_ptr}/m/{r}"))
            .or_else(|| s(&system, &format!("{set_ptr}/m/{r}")))
            .unwrap_or("#000000")
            .to_string()
    };
    let mut roles: BTreeMap<&str, String> = ROLES.iter().map(|r| (*r, role(r))).collect();

    // System packs take the OS accent (10 §4.4).
    if src_slug == "system"
        && let Some(os_accent) = &i.os.accent
        && let Some(a) = normalise_os_accent(os_accent, &roles["surface"], dark)
    {
        roles.insert("accent", a);
    }

    let mut tokens = BTreeMap::new();
    let parse = |h: &str| {
        color::parse_hex(h).unwrap_or(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        })
    };
    let afg = accent_fg(
        parse(&roles["accent"]),
        parse(&roles["fg"]),
        parse(&roles["bg"]),
    );
    if afg.adjusted {
        notices.push("accent adjusted for contrast".into());
        roles.insert("accent", afg.accent.clone());
    }
    for r in ROLES {
        tokens.insert(format!("--m-{r}"), roles[r].clone());
    }
    tokens.insert("--m-accent-fg".into(), afg.text.clone());
    tokens.insert("--m-selection".into(), roles["selected"].clone());
    tokens.insert(
        "--m-scrollbar".into(),
        color::to_hex_alpha(parse(&roles["fg-3"]), 0.4),
    );

    // Typography, radius, motion, shadow, pattern: the pack, then System.
    let pick = |ptr: &str| {
        s(i.pack, ptr)
            .or_else(|| s(&system, ptr))
            .map(str::to_string)
    };
    tokens.insert(
        "--m-font-ui".into(),
        pick("/typography/ui").unwrap_or_else(|| builtin::FONT_UI.into()),
    );
    tokens.insert(
        "--m-font-mono".into(),
        pick("/typography/mono").unwrap_or_else(|| builtin::FONT_MONO.into()),
    );
    let scale = i
        .font_scale
        .or_else(|| num(i.pack, "/typography/scale"))
        .unwrap_or(1.0)
        .clamp(0.9, 1.3);
    tokens.insert("--m-font-scale".into(), fmt_num(scale));
    let radius = |k: &str, legacy: [&str; 2], def: f64| {
        num(i.pack, &format!("/radius/{k}"))
            .or_else(|| {
                legacy
                    .iter()
                    .find_map(|l| num(i.pack, &format!("/radius/{l}")))
            })
            .unwrap_or(def)
            .clamp(0.0, 16.0)
    };
    // `radius.control` / `radius.card` in the pack, `mode.css`'s names on the way out.
    tokens.insert(
        RADIUS_CONTROL.into(),
        format!("{}px", fmt_num(radius("control", ["sm", "xs"], 6.0))),
    );
    tokens.insert(
        RADIUS_CARD.into(),
        format!("{}px", fmt_num(radius("card", ["md", "lg"], 10.0))),
    );
    let motion = if i.os.reduce_motion {
        0.0
    } else {
        num(i.pack, "/motion/scale").unwrap_or(1.0).clamp(0.5, 1.5)
    };
    tokens.insert("--m-motion-scale".into(), fmt_num(motion));
    let shadow = s(src, &format!("/shadow/{which}/pop"))
        .map(str::to_string)
        .unwrap_or_else(|| {
            if dark {
                builtin::SHADOW_DARK
            } else {
                builtin::SHADOW_LIGHT
            }
            .to_string()
        });
    tokens.insert("--m-shadow-pop".into(), shadow);
    tokens.insert(
        "--m-pattern-kind".into(),
        s(src, &format!("{set_ptr}/pattern/kind"))
            .unwrap_or("none")
            .to_string(),
    );
    if let Some(c) = s(src, &format!("{set_ptr}/pattern/color")) {
        tokens.insert("--m-pattern-color".into(), c.to_string());
    }
    if let Some(n) = num(src, &format!("{set_ptr}/pattern/size")) {
        tokens.insert("--m-pattern-size".into(), format!("{}px", fmt_num(n)));
    }

    // Avatar hues (§5.10): the pack's twelve, else generated.
    let avatars: Vec<String> = src
        .pointer(&format!("{set_ptr}/avatar"))
        .and_then(Value::as_array)
        .filter(|a| a.len() == 12 && a.iter().all(|h| h.as_str().is_some_and(color::is_hex)))
        .map(|a| {
            a.iter()
                .filter_map(|h| h.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_else(|| color::generated_avatars(dark));
    for (n, hue) in avatars.iter().enumerate() {
        tokens.insert(format!("--m-avatar-{}", n + 1), hue.clone());
    }

    // Terminal (§5.11): `theme.terminal_pack`, else the color source, else System.
    let term_src = i
        .terminal_pack
        .filter(|p| p.pointer(&format!("{set_ptr}/terminal")).is_some())
        .unwrap_or(src);
    let term = |k: &str| -> String {
        s(term_src, &format!("{set_ptr}/terminal/{k}"))
            .or_else(|| s(&system, &format!("{set_ptr}/terminal/{k}")))
            .unwrap_or("#000000")
            .to_string()
    };
    for k in ["fg", "bg", "cursor", "selection"] {
        tokens.insert(format!("--m-terminal-{k}"), term(k));
    }
    // The 21st token: the glyph under the block cursor (xterm's
    // `cursorAccent`), the pack's `terminal.cursor_accent` else the background.
    let cursor_accent = s(term_src, &format!("{set_ptr}/terminal/cursor_accent"))
        .filter(|c| color::is_hex(c))
        .map(str::to_string)
        .unwrap_or_else(|| term("bg"));
    tokens.insert("--m-terminal-cursor-accent".into(), cursor_accent);
    for n in ANSI {
        tokens.insert(format!("--m-terminal-ansi-{n}"), term(&format!("ansi/{n}")));
    }

    // Accessibility overrides (§11.5).
    tokens.insert(
        "--m-reduce-transparency".into(),
        if i.os.reduce_transparency { "1" } else { "0" }.into(),
    );
    if i.os.increase_contrast && !i.os.forced_colors {
        let strong = tokens["--m-line-strong"].clone();
        let fg2 = tokens["--m-fg-2"].clone();
        let fg = tokens["--m-fg"].clone();
        tokens.insert("--m-line".into(), strong);
        tokens.insert("--m-fg-3".into(), fg2);
        tokens.insert("--m-fg-2".into(), fg);
        tokens.insert("--m-focus-ring-width".into(), "2px".into());
        tokens.insert("--m-card-border".into(), "1px".into());
    }
    if i.os.forced_colors {
        for (k, v) in [
            ("bg", "Canvas"),
            ("surface", "Canvas"),
            ("sunken", "Canvas"),
            ("hover", "Canvas"),
            ("selected", "Highlight"),
            ("line", "CanvasText"),
            ("line-strong", "CanvasText"),
            ("fg", "CanvasText"),
            ("fg-2", "CanvasText"),
            ("fg-3", "CanvasText"),
            ("accent", "Highlight"),
            ("accent-bg", "Canvas"),
            ("ok", "CanvasText"),
            ("bad", "CanvasText"),
            ("info", "LinkText"),
        ] {
            tokens.insert(format!("--m-{k}"), v.into());
        }
        tokens.insert("--m-accent-fg".into(), "HighlightText".into());
        tokens.insert("--m-selection".into(), "Highlight".into());
        tokens.insert("--m-scrollbar".into(), "CanvasText".into());
        tokens.insert("--m-shadow-pop".into(), "none".into());
        notices.push("high contrast: the pack is not used".into());
    }

    ResolvedTheme {
        name: if src_slug == "system" {
            set_name("system", "System", dark)
        } else {
            active_name.clone()
        },
        slug: src_slug,
        active,
        appearance: which.into(),
        dark,
        tokens,
        notices,
        accent_adjusted: afg.adjusted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(slug: &str, pref: AppearancePref, os: OsAppearance) -> ResolvedTheme {
        let doc = builtin::by_slug(slug).unwrap().document();
        resolve(&Inputs {
            pack: &doc,
            terminal_pack: None,
            appearance: pref,
            os: &os,
            font_scale: None,
        })
    }

    #[test]
    fn paper_keeps_white_on_its_accent() {
        let r = run("paper", AppearancePref::Light, OsAppearance::default());
        assert_eq!(r.tokens["--m-accent"], "#D9480F");
        assert_eq!(r.tokens["--m-accent-fg"], "#FFFFFF");
        assert!(!r.accent_adjusted);
    }

    #[test]
    fn catppuccin_takes_dark_text_on_peach() {
        let r = run(
            "catppuccin-mocha",
            AppearancePref::Dark,
            OsAppearance::default(),
        );
        assert_eq!(r.tokens["--m-accent"], "#FAB387");
        assert_eq!(r.tokens["--m-accent-fg"], "#1E1E2E");
    }

    #[test]
    fn system_takes_the_os_accent() {
        let os = OsAppearance {
            accent: Some("#7D3CFF".into()),
            ..Default::default()
        };
        let r = run("system", AppearancePref::Auto, os);
        assert_eq!(r.tokens["--m-accent"], "#7D3CFF");
        assert_eq!(r.tokens["--m-info"], "#0066CC", "every other role is fixed");
        assert_eq!(r.name, "System Light");
    }

    #[test]
    fn forced_colours_ignore_the_pack() {
        let os = OsAppearance {
            forced_colors: true,
            ..Default::default()
        };
        let r = run("nord", AppearancePref::Dark, os);
        assert_eq!(r.tokens["--m-bg"], "Canvas");
        assert_eq!(r.tokens["--m-accent-fg"], "HighlightText");
    }
}
