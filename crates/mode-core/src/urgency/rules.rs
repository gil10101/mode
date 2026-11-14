//! Keyword and deadline rules (07 §3.3): optional, off by default
//! (`urgency.keyword_rules = []`). Each rule matches a field with
//! `contains`, `word`, `regex` (Rust `regex`, linear time, ≤ 512 chars) or
//! the built-in deadline detector, and raises (`ai-urgent`) or lowers
//! (`low`) with `urgency_source = 'rule'` and the why `rule: <name>`. They
//! run after the deterministic stage, so they never touch an important person or a DM.
//!
//! Extension (reported): `labels_any` + `labels_mode` express the premade
//! rules' label conditions ("on messages whose `labels_json` has
//! `broadcast`", "or `labels_json` has `list` / `bulk` / …").

use std::cell::RefCell;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use time::{Date, Duration, Month, OffsetDateTime, Weekday};

use super::{Candidate, Tier, why};
use crate::notes::fields::LocalTz;

/// Regex patterns longer than this are refused (07 §3.3).
pub const REGEX_MAX: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleField {
    Subject,
    Snippet,
    Sender,
    Channel,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    Contains,
    Word,
    Regex,
    Deadline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Raise,
    Lower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelsMode {
    /// The rule matches when the pattern matches **or** a label is present.
    #[default]
    Or,
    /// The pattern must match **and** a label must be present.
    And,
}

fn three() -> i64 {
    3
}

/// One entry of `urgency.keyword_rules` (07 §3.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeywordRule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub field: RuleField,
    #[serde(rename = "match")]
    pub match_kind: MatchKind,
    #[serde(default)]
    pub pattern: String,
    pub effect: Effect,
    #[serde(default = "three")]
    pub horizon_days: i64,
    #[serde(default)]
    pub position: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels_any: Vec<String>,
    #[serde(default)]
    pub labels_mode: LabelsMode,
}

/// The three premade rules, shipped disabled (07 §3.3).
pub fn premade() -> Vec<KeywordRule> {
    vec![
        KeywordRule {
            id: "00000000-0000-7000-8000-000000000301".into(),
            name: "Mentions a deadline".into(),
            enabled: false,
            field: RuleField::Any,
            match_kind: MatchKind::Deadline,
            pattern: String::new(),
            effect: Effect::Raise,
            horizon_days: 3,
            position: 0,
            labels_any: vec![],
            labels_mode: LabelsMode::Or,
        },
        KeywordRule {
            id: "00000000-0000-7000-8000-000000000302".into(),
            name: "Slack broadcasts".into(),
            enabled: false,
            field: RuleField::Any,
            match_kind: MatchKind::Regex,
            pattern: "(?i)@(here|channel|everyone)".into(),
            effect: Effect::Raise,
            horizon_days: 3,
            position: 1,
            labels_any: vec!["broadcast".into()],
            labels_mode: LabelsMode::And,
        },
        KeywordRule {
            id: "00000000-0000-7000-8000-000000000303".into(),
            name: "Newsletters and bots".into(),
            enabled: false,
            field: RuleField::Any,
            match_kind: MatchKind::Regex,
            pattern: "(?i)(unsubscribe|view (this|in) browser|digest|newsletter)".into(),
            effect: Effect::Lower,
            horizon_days: 3,
            position: 2,
            labels_any: vec!["list".into(), "bulk".into(), "auto".into(), "bot".into()],
            labels_mode: LabelsMode::Or,
        },
    ]
}

/// Check one rule for the Settings editor.
pub fn validate(rule: &KeywordRule) -> Result<(), String> {
    let name = rule.name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("a rule name is 1–40 characters".into());
    }
    if !(1..=30).contains(&rule.horizon_days) {
        return Err("horizon_days is 1–30".into());
    }
    match rule.match_kind {
        MatchKind::Regex => {
            if rule.pattern.len() > REGEX_MAX {
                return Err(format!("a regex is at most {REGEX_MAX} characters"));
            }
            compile(&rule.pattern).map(|_| ())
        }
        MatchKind::Contains | MatchKind::Word if rule.pattern.trim().is_empty() => {
            if rule.labels_any.is_empty() {
                Err("the pattern is empty".into())
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

thread_local! {
    static REGEX_CACHE: RefCell<HashMap<String, regex::Regex>> = RefCell::new(HashMap::new());
}

fn compile(pattern: &str) -> Result<regex::Regex, String> {
    if pattern.len() > REGEX_MAX {
        return Err("regex too long".into());
    }
    REGEX_CACHE.with(|c| {
        if let Some(r) = c.borrow().get(pattern) {
            return Ok(r.clone());
        }
        let r = regex::RegexBuilder::new(pattern)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| format!("invalid regex: {e}"))?;
        let mut m = c.borrow_mut();
        if m.len() > 256 {
            m.clear();
        }
        m.insert(pattern.to_string(), r.clone());
        Ok(r)
    })
}

fn field_text(c: &Candidate, field: RuleField) -> String {
    let subject = c.subject.clone().unwrap_or_default();
    match field {
        RuleField::Subject => subject,
        RuleField::Snippet => c.snippet.clone(),
        RuleField::Sender => format!("{} {}", c.sender_name, c.sender_address),
        RuleField::Channel => c.channel_name.clone().unwrap_or_default(),
        RuleField::Any => format!("{subject}\n{}", c.snippet),
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whole-word, case-insensitive (word boundaries at alphanumerics only, so
/// `@client.com` works as a word too).
pub fn word_match(text: &str, word: &str) -> bool {
    let w = word.trim().to_lowercase();
    if w.is_empty() {
        return false;
    }
    let t = text.to_lowercase();
    let mut start = 0;
    while let Some(pos) = t[start..].find(&w) {
        let i = start + pos;
        let before = t[..i].chars().next_back();
        let after = t[i + w.len()..].chars().next();
        let first = w.chars().next().unwrap_or(' ');
        let last = w.chars().next_back().unwrap_or(' ');
        let ok_before = !is_word_char(first) || before.is_none_or(|b| !is_word_char(b));
        let ok_after = !is_word_char(last) || after.is_none_or(|a| !is_word_char(a));
        if ok_before && ok_after {
            return true;
        }
        start = i + w.len().max(1);
        if start >= t.len() {
            break;
        }
    }
    false
}

/// Whether one rule matches a candidate.
pub fn rule_matches(rule: &KeywordRule, c: &Candidate, today: Date) -> bool {
    let text = field_text(c, rule.field);
    let pattern = match rule.match_kind {
        MatchKind::Contains => {
            !rule.pattern.trim().is_empty()
                && text
                    .to_lowercase()
                    .contains(&rule.pattern.trim().to_lowercase())
        }
        MatchKind::Word => word_match(&text, &rule.pattern),
        MatchKind::Regex => compile(&rule.pattern).is_ok_and(|r| r.is_match(&text)),
        MatchKind::Deadline => detect_deadline(&text, today, rule.horizon_days).is_some(),
    };
    if rule.labels_any.is_empty() {
        return pattern;
    }
    let labelled = rule.labels_any.iter().any(|l| c.has_label(l));
    match rule.labels_mode {
        LabelsMode::Or => pattern || labelled,
        LabelsMode::And => pattern && labelled,
    }
}

/// The first enabled rule (by `position`) that matches: its tier and why.
pub fn evaluate(
    rules: &[KeywordRule],
    c: &Candidate,
    now: OffsetDateTime,
    tz: &dyn LocalTz,
) -> Option<(Tier, String)> {
    if rules.iter().all(|r| !r.enabled) {
        return None;
    }
    let today = tz.local_date(now);
    let mut ordered: Vec<&KeywordRule> = rules.iter().filter(|r| r.enabled).collect();
    ordered.sort_by_key(|r| r.position);
    ordered
        .into_iter()
        .find(|r| rule_matches(r, c, today))
        .map(|r| {
            let tier = match r.effect {
                Effect::Raise => Tier::AiUrgent,
                Effect::Lower => Tier::Low,
            };
            (tier, why::rule(&r.name))
        })
}

// -------------------------------------------------------------- deadlines

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

fn month_of(word: &str) -> Option<Month> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    let i = MONTHS.iter().position(|m| w.starts_with(m))?;
    // reject words like "mayor" / "marching" that start with a month prefix
    let full = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ][i];
    let ok = full.starts_with(&w) || (w.len() == 4 && w == "sept");
    if !ok {
        return None;
    }
    Month::try_from(i as u8 + 1).ok()
}

fn weekday_of(word: &str) -> Option<Weekday> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    let i = WEEKDAYS.iter().position(|d| w.starts_with(d))?;
    let full = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ][i];
    let alt = ["mon", "tues", "wed", "thur", "fri", "sat", "sun"];
    if !(full.starts_with(&w) || w == alt[i] || w == "thurs") {
        return None;
    }
    Some(
        [
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
            Weekday::Saturday,
            Weekday::Sunday,
        ][i],
    )
}

fn next_weekday(today: Date, wd: Weekday, strictly_after: bool) -> Date {
    let from = today.weekday().number_days_from_monday() as i64;
    let to = wd.number_days_from_monday() as i64;
    let mut d = (to - from).rem_euclid(7);
    if strictly_after && d == 0 {
        d = 7;
    }
    today + Duration::days(d)
}

fn date_in_year(today: Date, month: Month, day: u8) -> Option<Date> {
    let d = Date::from_calendar_date(today.year(), month, day).ok()?;
    if d < today - Duration::days(30) {
        Date::from_calendar_date(today.year() + 1, month, day).ok()
    } else {
        Some(d)
    }
}

fn day_number(tok: &str) -> Option<u8> {
    let digits: String = tok.chars().take_while(|c| c.is_ascii_digit()).collect();
    let rest = &tok[digits.len()..];
    if digits.is_empty() || digits.len() > 2 {
        return None;
    }
    if !rest.is_empty() && !matches!(rest, "st" | "nd" | "rd" | "th") {
        return None;
    }
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

/// Every date expression in `text`, resolved against `today`.
pub fn dates_in(text: &str, today: Date) -> Vec<Date> {
    let lower = text.to_lowercase();
    let toks: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '/'))
        .filter(|t| !t.is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let prev = if i > 0 { toks[i - 1] } else { "" };
        let next = toks.get(i + 1).copied().unwrap_or("");
        match *t {
            "today" | "tonight" | "eod" | "cob" | "eob" => out.push(today),
            "tomorrow" | "tmrw" => out.push(today + Duration::days(1)),
            _ => {}
        }
        if let Some(wd) = weekday_of(t) {
            out.push(next_weekday(today, wd, prev == "next"));
        }
        // ISO 2026-10-21
        if t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' {
            let p: Vec<&str> = t.split('-').collect();
            if let (Ok(y), Ok(m), Ok(d)) =
                (p[0].parse::<i32>(), p[1].parse::<u8>(), p[2].parse::<u8>())
                && let Ok(mm) = Month::try_from(m)
                && let Ok(date) = Date::from_calendar_date(y, mm, d)
            {
                out.push(date);
            }
        }
        // 10/21 (month/day)
        if let Some((a, b)) = t.split_once('/')
            && !b.contains('/')
            && let (Ok(m), Ok(d)) = (a.parse::<u8>(), b.parse::<u8>())
            && let Ok(mm) = Month::try_from(m)
            && let Some(date) = date_in_year(today, mm, d)
        {
            out.push(date);
        }
        // 21 Oct / 21st October
        if let Some(d) = day_number(t)
            && let Some(m) = month_of(next)
            && let Some(date) = date_in_year(today, m, d)
        {
            out.push(date);
        }
        // Oct 21 / October 21st
        if let Some(m) = month_of(t)
            && let Some(d) = day_number(next)
            && let Some(date) = date_in_year(today, m, d)
        {
            out.push(date);
        }
    }
    out
}

/// The built-in deadline detector (07 §3.3): `by <weekday/date>`, `before
/// <date>`, `due <date>`, `deadline`, `EOD`, `end of day`, `COB`, `ASAP`,
/// ISO / natural / relative dates. A match counts when a deadline trigger is
/// present and a date in the text falls within `horizon_days`, or when the
/// phrase carries no date (`ASAP`, `EOD`, a bare `deadline`). Returns the
/// phrase that matched.
pub fn detect_deadline(text: &str, today: Date, horizon_days: i64) -> Option<String> {
    let lower = text.to_lowercase();
    static DATELESS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static TRIGGER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let dateless = DATELESS.get_or_init(|| {
        regex::Regex::new(
            r"\b(asap|a\.s\.a\.p\.?|eod|cob|eob|end of (the )?day|end of business|close of business|as soon as possible)\b",
        )
        .expect("static regex")
    });
    if let Some(m) = dateless.find(&lower) {
        return Some(m.as_str().to_string());
    }
    let trigger = TRIGGER.get_or_init(|| {
        regex::Regex::new(r"\b(by|before|due|deadline|until|no later than)\b")
            .expect("static regex")
    });
    let mut found_trigger = None;
    for m in trigger.find_iter(&lower) {
        found_trigger.get_or_insert(m.as_str().to_string());
        // a date within the next ~40 characters after the trigger
        let tail: String = lower[m.end()..].chars().take(40).collect();
        let horizon = today + Duration::days(horizon_days);
        if dates_in(&tail, today)
            .into_iter()
            .any(|d| d >= today && d <= horizon)
        {
            return Some(format!("{} {}", m.as_str(), tail.trim()));
        }
    }
    if found_trigger.as_deref() == Some("deadline") && dates_in(&lower, today).is_empty() {
        return Some("deadline".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wed() -> Date {
        // Wednesday 2026-10-21
        Date::from_calendar_date(2026, Month::October, 21).unwrap()
    }

    #[test]
    fn deadline_phrases() {
        let t = wed();
        assert!(detect_deadline("Can you sign off by Friday?", t, 3).is_some());
        assert!(detect_deadline("Please send it by next Monday", t, 3).is_none());
        assert!(detect_deadline("Please send it by next Monday", t, 7).is_some());
        assert!(detect_deadline("need this ASAP", t, 3).is_some());
        assert!(detect_deadline("by EOD please", t, 3).is_some());
        assert!(detect_deadline("due 2026-10-22", t, 3).is_some());
        assert!(detect_deadline("due 2026-11-30", t, 3).is_none());
        assert!(detect_deadline("before 23 Oct", t, 3).is_some());
        assert!(detect_deadline("deadline is October 23rd", t, 3).is_some());
        assert!(detect_deadline("deadline is 10/30", t, 3).is_none());
        assert!(detect_deadline("the deadline moved", t, 3).is_some());
        assert!(detect_deadline("lunch on Friday?", t, 3).is_none());
        assert!(detect_deadline("by tomorrow", t, 3).is_some());
        assert!(detect_deadline("the mayor stopped by", t, 3).is_none());
    }

    #[test]
    fn weekday_resolution() {
        let t = wed();
        assert_eq!(next_weekday(t, Weekday::Wednesday, false), t);
        assert_eq!(
            next_weekday(t, Weekday::Wednesday, true),
            t + Duration::days(7)
        );
        assert_eq!(
            next_weekday(t, Weekday::Monday, true),
            t + Duration::days(5)
        );
        assert_eq!(dates_in("Oct 23", t), vec![t + Duration::days(2)]);
    }

    #[test]
    fn word_matching() {
        assert!(word_match("Invoice from @client.com team", "@client.com"));
        assert!(word_match("URGENT: deploy", "urgent"));
        assert!(!word_match("urgently", "urgent"));
        assert!(!word_match("", "x"));
    }

    #[test]
    fn premade_rules_are_disabled_and_valid() {
        for r in premade() {
            assert!(!r.enabled);
            validate(&r).unwrap();
        }
        let mut bad = premade()[1].clone();
        bad.pattern = "(".into();
        assert!(validate(&bad).is_err());
        let v = serde_json::to_value(&premade()[0]).unwrap();
        assert_eq!(v["match"], "deadline");
        assert_eq!(v["field"], "any");
    }
}
