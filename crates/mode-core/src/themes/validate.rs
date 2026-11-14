//! Theme pack validation on import and on seed (10 §4.3, contrast targets
//! §11.1). Every issue carries the JSON path of the token it is about and a
//! message that names it ("`colors.dark.m.fg` is missing."). Errors refuse
//! the pack; contrast findings are warnings (§4.3 "Imports proceed"), each
//! marked `required` when it misses the §11.1 floor rather than the target;
//! fills are the defaults the resolver supplies (avatar hues, terminal).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::builtin::{ANSI, ROLES};
use super::color;

/// The pack format this build reads (10 §4.1).
pub const FORMAT: i64 = 1;
/// Largest sound cue (10 §4.1).
pub const SOUND_MAX_BYTES: usize = 64 * 1024;
/// Cue ids a pack may replace (10 §10).
pub const CUES: [&str; 6] = [
    "waiting", "urgent", "finished", "failed", "starting", "captured",
];

const TOP_KEYS: [&str; 12] = [
    "format",
    "slug",
    "name",
    "author",
    "version",
    "appearance",
    "typography",
    "radius",
    "motion",
    "colors",
    "shadow",
    "sounds",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Fill,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// Format, required key, type, color syntax, appearance, sounds, tokens-only.
    #[default]
    Structure,
    /// A §11.1 contrast pair below its target.
    Contrast,
    /// A key this format does not know (kept on export, ignored).
    UnknownKey,
    /// A default the resolver supplies.
    Fill,
}

/// One finding (Settings → Appearance lists them with the JSON path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    pub message: String,
    pub severity: Severity,
    /// What the finding is about (beyond `PackIssue` in the contract).
    #[serde(default)]
    pub kind: IssueKind,
    /// The measured ratio of a contrast finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    /// A contrast finding below the §11.1 floor (not only the target).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}

/// The validator's answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// No errors: the pack may be imported or seeded.
    pub valid: bool,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
    pub fills: Vec<Issue>,
}

impl Report {
    fn push(&mut self, i: Issue) {
        match i.severity {
            Severity::Error => self.errors.push(i),
            Severity::Warning => self.warnings.push(i),
            Severity::Fill => self.fills.push(i),
        }
    }

    fn error(&mut self, path: &str, message: String) {
        self.push(Issue {
            path: path.into(),
            message,
            severity: Severity::Error,
            kind: IssueKind::Structure,
            ratio: None,
            required: false,
        });
    }

    /// Contrast warnings only.
    pub fn contrast(&self) -> impl Iterator<Item = &Issue> {
        self.warnings
            .iter()
            .filter(|i| i.kind == IssueKind::Contrast)
    }
}

fn missing(path: &str) -> String {
    format!("`{path}` is missing.")
}

fn type_error(path: &str, what: &str) -> String {
    format!("`{path}` must be {what}.")
}

/// `[a-z0-9-]{2,32}` (10 §4.1).
pub fn slug_ok(s: &str) -> bool {
    (2..=32).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn tokens_only(v: &Value, path: &str, r: &mut Report) {
    match v {
        Value::String(s) => {
            let l = s.to_ascii_lowercase();
            if [
                "url(",
                "expression(",
                "<",
                "://",
                "javascript:",
                "data:",
                "@import",
            ]
            .iter()
            .any(|p| l.contains(p))
            {
                r.error(path, format!("`{path}`: Packs are tokens only."));
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                tokens_only(x, &format!("{path}[{i}]"), r);
            }
        }
        Value::Object(m) => {
            for (k, x) in m {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if p == "sounds" {
                    continue;
                }
                tokens_only(x, &p, r);
            }
        }
        _ => {}
    }
}

fn number_in(doc: &Value, path: &str, lo: f64, hi: f64, r: &mut Report) {
    if let Some(v) = pointer(doc, path) {
        match v.as_f64() {
            Some(n) if (lo..=hi).contains(&n) => {}
            Some(_) => r.error(path, format!("`{path}` must be between {lo} and {hi}.")),
            None => r.error(path, type_error(path, "a number")),
        }
    }
}

/// `a.b.c` lookup.
pub fn pointer<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = doc;
    for part in path.split('.') {
        cur = cur.get(part)?;
    }
    Some(cur)
}

fn string_opt(doc: &Value, path: &str, r: &mut Report) {
    if let Some(v) = pointer(doc, path)
        && !v.is_string()
    {
        r.error(path, type_error(path, "a string"));
    }
}

fn colour(doc: &Value, path: &str, required: bool, r: &mut Report) {
    match pointer(doc, path) {
        None if required => r.error(path, missing(path)),
        None => {}
        Some(Value::String(s)) if color::is_hex(s) => {}
        Some(Value::String(_)) => r.error(
            path,
            format!("`{path}` is not a color (#rrggbb or #rrggbbaa)."),
        ),
        Some(_) => r.error(path, type_error(path, "a color string")),
    }
}

fn unknown_keys(obj: &Value, allowed: &[&str], prefix: &str, r: &mut Report) {
    if let Some(m) = obj.as_object() {
        for k in m.keys() {
            if !allowed.contains(&k.as_str()) {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                r.push(Issue {
                    message: format!("`{path}` is not a pack key and is ignored."),
                    path,
                    severity: Severity::Warning,
                    kind: IssueKind::UnknownKey,
                    ratio: None,
                    required: false,
                });
            }
        }
    }
}

fn sounds(doc: &Value, r: &mut Report) {
    let Some(s) = doc.get("sounds") else { return };
    let Some(m) = s.as_object() else {
        r.error("sounds", type_error("sounds", "an object"));
        return;
    };
    for (cue, v) in m {
        let path = format!("sounds.{cue}");
        if !CUES.contains(&cue.as_str()) {
            r.push(Issue {
                message: format!("`{path}` is not a sound cue and is ignored."),
                path,
                severity: Severity::Warning,
                kind: IssueKind::UnknownKey,
                ratio: None,
                required: false,
            });
            continue;
        }
        let ok = v
            .as_str()
            .and_then(|s| s.strip_prefix("data:audio/wav;base64,"))
            .and_then(|b64| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(b64).ok()
            })
            .is_some_and(|bytes| bytes.len() <= SOUND_MAX_BYTES && bytes.starts_with(b"RIFF"));
        if !ok {
            r.error(
                &path,
                format!("`{path}` must be a WAV data URI under 64 KB."),
            );
        }
    }
}

fn colour_set(doc: &Value, which: &str, r: &mut Report) {
    let base = format!("colors.{which}");
    let Some(set) = pointer(doc, &base) else {
        return;
    };
    if !set.is_object() {
        r.error(&base, type_error(&base, "an object"));
        return;
    }
    unknown_keys(set, &["m", "avatar", "terminal", "pattern"], &base, r);
    let mp = format!("{base}.m");
    match set.get("m") {
        None => r.error(&mp, missing(&mp)),
        Some(m) if !m.is_object() => r.error(&mp, type_error(&mp, "an object")),
        Some(m) => {
            for role in ROLES {
                colour(doc, &format!("{mp}.{role}"), true, r);
            }
            unknown_keys(m, &ROLES, &mp, r);
        }
    }
    let ap = format!("{base}.avatar");
    match set.get("avatar") {
        Some(Value::Array(a)) if a.len() == 12 => {
            for (i, hue) in a.iter().enumerate() {
                let ok = hue.as_str().is_some_and(color::is_hex);
                if !ok {
                    r.error(
                        &format!("{ap}[{i}]"),
                        format!("`{ap}[{i}]` is not a color (#rrggbb or #rrggbbaa)."),
                    );
                }
            }
        }
        _ => r.push(Issue {
            path: ap,
            message: "avatar palette generated".into(),
            severity: Severity::Fill,
            kind: IssueKind::Fill,
            ratio: None,
            required: false,
        }),
    }
    let tp = format!("{base}.terminal");
    match set.get("terminal") {
        None => r.push(Issue {
            message: format!(
                "terminal palette from System {}",
                if which == "dark" { "Dark" } else { "Light" }
            ),
            path: tp,
            severity: Severity::Fill,
            kind: IssueKind::Fill,
            ratio: None,
            required: false,
        }),
        Some(t) if !t.is_object() => r.error(&tp, type_error(&tp, "an object")),
        Some(t) => {
            for k in ["fg", "bg", "cursor", "selection"] {
                colour(doc, &format!("{tp}.{k}"), true, r);
            }
            if t.get("cursor_accent").is_some() {
                colour(doc, &format!("{tp}.cursor_accent"), false, r);
            }
            match t.get("ansi") {
                None => r.error(&format!("{tp}.ansi"), missing(&format!("{tp}.ansi"))),
                Some(_) => {
                    for n in ANSI {
                        colour(doc, &format!("{tp}.ansi.{n}"), true, r);
                    }
                }
            }
        }
    }
    let pp = format!("{base}.pattern");
    if let Some(p) = set.get("pattern") {
        match p.get("kind").and_then(Value::as_str) {
            Some("none") | Some("dots") | None => {}
            Some(_) => r.error(
                &format!("{pp}.kind"),
                format!("`{pp}.kind` must be none or dots."),
            ),
        }
        colour(doc, &format!("{pp}.color"), false, r);
        number_in(doc, &format!("{pp}.size"), 1.0, 64.0, r);
    }
}

fn contrast_issue(
    r: &mut Report,
    path: String,
    message: String,
    ratio: f64,
    target: f64,
    floor: f64,
) {
    if ratio + 1e-9 >= target {
        return;
    }
    r.push(Issue {
        path,
        message,
        severity: Severity::Warning,
        kind: IssueKind::Contrast,
        ratio: Some((ratio * 100.0).round() / 100.0),
        required: ratio + 1e-9 < floor,
    });
}

/// The §11.1 pairs of one color set; appended as contrast warnings.
fn contrast_checks(doc: &Value, which: &str, r: &mut Report) {
    let base = format!("colors.{which}.m");
    let get = |role: &str| {
        pointer(doc, &format!("{base}.{role}"))
            .and_then(Value::as_str)
            .and_then(color::parse_hex)
    };
    let pair = |r: &mut Report, text: &str, on: &str, target: f64, floor: f64| {
        if let (Some(t), Some(b)) = (get(text), get(on)) {
            let ratio = color::contrast(t, b);
            contrast_issue(
                r,
                format!("{base}.{text}"),
                format!("`--m-{text}` on `--m-{on}` is {ratio:.1}:1 (target {target}:1)."),
                ratio,
                target,
                floor,
            );
        }
    };
    let backgrounds = ["bg", "surface", "sunken", "hover", "selected", "accent-bg"];
    for text in ["fg", "fg-2", "info"] {
        for on in backgrounds {
            pair(r, text, on, 4.5, 4.5);
        }
    }
    for on in backgrounds {
        pair(r, "fg-3", on, 4.5, 3.0);
    }
    if let (Some(accent), Some(fg), Some(bg)) = (get("accent"), get("fg"), get("bg")) {
        let d = super::resolve::accent_fg(accent, fg, bg);
        if let Some(text) = color::parse_hex(&d.text) {
            let ratio = color::contrast(text, accent);
            contrast_issue(
                r,
                format!("{base}.accent"),
                format!("`--m-accent-fg` on `--m-accent` is {ratio:.1}:1 (target 4.5:1)."),
                ratio,
                4.5,
                3.0,
            );
        }
    }
    for glyph in ["accent", "ok", "bad", "info", "fg-3"] {
        for on in ["surface", "bg"] {
            if let (Some(t), Some(b)) = (get(glyph), get(on)) {
                let ratio = color::contrast(t, b);
                contrast_issue(
                    r,
                    format!("{base}.{glyph}"),
                    format!(
                        "`--m-{glyph}` (state glyphs) on `--m-{on}` is {ratio:.1}:1 (target 3:1)."
                    ),
                    ratio,
                    3.0,
                    0.0,
                );
            }
        }
    }
    pair(r, "line-strong", "surface", 3.0, 3.0);
}

/// Validate a pack document (10 §4.3).
pub fn validate(doc: &Value) -> Report {
    let mut r = Report::default();
    if !doc.is_object() {
        r.error("", "A theme pack is one JSON object.".into());
        return r;
    }
    match doc.get("format") {
        None => r.error("format", missing("format")),
        Some(v) => match v.as_i64() {
            Some(n) if (1..=FORMAT).contains(&n) => {}
            Some(n) if n > FORMAT => r.error(
                "format",
                format!(
                    "This pack needs a newer Mode (format {n}; this Mode reads format {FORMAT})."
                ),
            ),
            _ => r.error("format", type_error("format", "a whole number ≥ 1")),
        },
    }
    match doc.get("slug") {
        None => r.error("slug", missing("slug")),
        Some(Value::String(s)) if slug_ok(s) => {}
        Some(_) => r.error(
            "slug",
            "`slug` must be 2–32 characters of a–z, 0–9 and -.".into(),
        ),
    }
    match doc.get("name") {
        None => r.error("name", missing("name")),
        Some(Value::String(s)) if !s.trim().is_empty() => {}
        Some(_) => r.error("name", type_error("name", "a non-empty string")),
    }
    string_opt(doc, "author", &mut r);
    string_opt(doc, "version", &mut r);
    let appearance = doc.get("appearance").and_then(Value::as_str);
    match (doc.get("appearance"), appearance) {
        (None, _) => r.error("appearance", missing("appearance")),
        (_, Some("light" | "dark" | "auto")) => {}
        _ => r.error(
            "appearance",
            "`appearance` must be light, dark or auto.".into(),
        ),
    }
    unknown_keys(doc, &TOP_KEYS, "", &mut r);

    if let Some(t) = doc.get("typography") {
        if t.is_object() {
            string_opt(doc, "typography.ui", &mut r);
            string_opt(doc, "typography.mono", &mut r);
            number_in(doc, "typography.scale", 0.9, 1.3, &mut r);
            unknown_keys(t, &["ui", "mono", "scale", "style"], "typography", &mut r);
        } else {
            r.error("typography", type_error("typography", "an object"));
        }
    }
    if let Some(t) = doc.get("radius") {
        if t.is_object() {
            for k in ["control", "card", "xs", "sm", "md", "lg"] {
                number_in(doc, &format!("radius.{k}"), 0.0, 16.0, &mut r);
            }
            unknown_keys(
                t,
                &["control", "card", "xs", "sm", "md", "lg"],
                "radius",
                &mut r,
            );
        } else {
            r.error("radius", type_error("radius", "an object"));
        }
    }
    if let Some(t) = doc.get("motion") {
        if t.is_object() {
            number_in(doc, "motion.scale", 0.5, 1.5, &mut r);
            unknown_keys(t, &["scale"], "motion", &mut r);
        } else {
            r.error("motion", type_error("motion", "an object"));
        }
    }
    match doc.get("colors") {
        None => r.error("colors", missing("colors")),
        Some(c) if !c.is_object() => r.error("colors", type_error("colors", "an object")),
        Some(c) => {
            unknown_keys(c, &["light", "dark"], "colors", &mut r);
            let (has_l, has_d) = (c.get("light").is_some(), c.get("dark").is_some());
            match appearance {
                Some("auto") => {
                    for (has, which) in [(has_l, "light"), (has_d, "dark")] {
                        if !has {
                            r.error(
                                &format!("colors.{which}"),
                                format!("`appearance` is `auto` but `colors.{which}` is missing."),
                            );
                        }
                    }
                }
                Some(one @ ("light" | "dark")) => {
                    let other = if one == "light" { "dark" } else { "light" };
                    if c.get(one).is_none() {
                        r.error(
                            &format!("colors.{one}"),
                            format!("`appearance` is `{one}` but `colors.{one}` is missing."),
                        );
                    }
                    if c.get(other).is_some() {
                        r.error(
                            &format!("colors.{other}"),
                            format!("`appearance` is `{one}` but `colors.{other}` is present."),
                        );
                    }
                }
                _ => {}
            }
            for which in ["light", "dark"] {
                colour_set(doc, which, &mut r);
            }
        }
    }
    if let Some(s) = doc.get("shadow") {
        if s.is_object() {
            unknown_keys(s, &["light", "dark"], "shadow", &mut r);
            for which in ["light", "dark"] {
                string_opt(doc, &format!("shadow.{which}.pop"), &mut r);
            }
        } else {
            r.error("shadow", type_error("shadow", "an object"));
        }
    }
    sounds(doc, &mut r);
    tokens_only(doc, "", &mut r);
    if r.errors.is_empty() {
        for which in ["light", "dark"] {
            if pointer(doc, &format!("colors.{which}")).is_some() {
                contrast_checks(doc, which, &mut r);
            }
        }
    }
    r.valid = r.errors.is_empty();
    r
}

/// The document as stored on import (10 §4.5 "the file plus filled
/// defaults"): a missing or malformed avatar list becomes the generated
/// twelve hues, a missing terminal palette the System one of that appearance.
pub fn fill_defaults(doc: &mut Value) {
    for which in ["light", "dark"] {
        let dark = which == "dark";
        let Some(set) = doc
            .get_mut("colors")
            .and_then(|c| c.get_mut(which))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        let avatar_ok = set
            .get("avatar")
            .and_then(Value::as_array)
            .is_some_and(|a| a.len() == 12);
        if !avatar_ok {
            set.insert("avatar".into(), Value::from(color::generated_avatars(dark)));
        }
        if !set.contains_key("terminal") {
            let sys = super::builtin::system_document();
            if let Some(t) = sys.pointer(&format!("/colors/{which}/terminal")) {
                set.insert("terminal".into(), t.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slug_rule() {
        assert!(slug_ok("my-pack-2"));
        assert!(!slug_ok("a"));
        assert!(!slug_ok("Mine"));
        assert!(!slug_ok(&"x".repeat(33)));
    }

    #[test]
    fn newer_format_is_refused_with_the_spec_message() {
        let r = validate(
            &json!({"format": 2, "slug": "x2", "name": "X", "appearance": "dark", "colors": {}}),
        );
        assert!(!r.valid);
        assert_eq!(
            r.errors[0].message,
            "This pack needs a newer Mode (format 2; this Mode reads format 1)."
        );
    }

    #[test]
    fn tokens_only_rejects_urls_and_markup() {
        let mut doc = super::super::builtin::BUILTINS[2].document();
        doc["slug"] = json!("evil");
        doc["typography"] = json!({"ui": "url(https://x.example/f.woff)"});
        let r = validate(&doc);
        assert!(r.errors.iter().any(|e| e.path == "typography.ui"
            && e.message.ends_with("Packs are tokens only.")));
    }
}
