//! `schedule_json` (09 §5.4A): `{"kind": "daily" | "weekdays" | "weekly" |
//! "cron", "time": "08:00", "tz": "Europe/Madrid", "weekday": 1, "cron":
//! "0 8 * * 1-5"}`. Every kind compiles to one 5-field cron plan (minute
//! hour day-of-month month day-of-week; standard ranges, lists and steps,
//! no seconds, no `@` aliases) evaluated in the IANA zone `tz`, so DST
//! follows the zone (03 §7B.4): a wall time inside a spring-forward gap
//! fires at the first instant after the gap; a wall time repeated by a
//! fall-back fold fires once, at its first occurrence.

use jiff::Timestamp;
use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// The four schedule kinds of the editor (03 §7B.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    Daily,
    Weekdays,
    Weekly,
    Cron,
}

/// `automations.schedule_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub kind: ScheduleKind,
    /// `HH:MM`, local to `tz` (daily / weekdays / weekly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// An IANA zone; the device zone when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// ISO weekday 1–7, Monday = 1 (weekly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekday: Option<u8>,
    /// A 5-field expression (cron).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
}

/// A schedule that does not compile; `field` is the editor field it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleError {
    /// `schedule.time` | `schedule.tz` | `schedule.weekday` | `schedule.cron`.
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn serr(field: &str, message: impl Into<String>) -> ScheduleError {
    ScheduleError {
        field: field.into(),
        message: message.into(),
    }
}

/// The device's IANA zone (`UTC` when the OS does not name one).
pub fn device_zone() -> String {
    TimeZone::system()
        .iana_name()
        .map(str::to_string)
        .unwrap_or_else(|| "UTC".into())
}

/// `HH:MM` (24 h; one-digit hours accepted).
pub fn parse_time(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u8, u8) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// One compiled cron field: a bitset of the allowed values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Field {
    bits: u64,
    /// The field was `*` (or `*/1`): day-of-month / day-of-week OR rule.
    star: bool,
}

impl Field {
    fn has(&self, v: u32) -> bool {
        self.bits & (1u64 << v) != 0
    }

    fn values(&self, lo: u32, hi: u32) -> Vec<u32> {
        (lo..=hi).filter(|v| self.has(*v)).collect()
    }
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn value(s: &str, lo: u32, hi: u32, names: &[&str], name: &str) -> Result<u32, String> {
    let l = s.to_ascii_lowercase();
    if let Some(i) = names.iter().position(|n| *n == l) {
        return Ok(i as u32 + lo);
    }
    let v: u32 = s
        .parse()
        .map_err(|_| format!("{name}: {s:?} is not a number"))?;
    if v < lo || v > hi {
        return Err(format!("{name}: {v} is outside {lo}–{hi}"));
    }
    Ok(v)
}

fn field(text: &str, lo: u32, hi: u32, names: &[&str], name: &str) -> Result<Field, String> {
    let mut bits = 0u64;
    let mut star = false;
    for item in text.split(',') {
        if item.is_empty() {
            return Err(format!("{name}: empty list item"));
        }
        let (range, step) = match item.split_once('/') {
            Some((r, s)) => {
                let st: u32 = s
                    .parse()
                    .map_err(|_| format!("{name}: step {s:?} is not a number"))?;
                if st == 0 {
                    return Err(format!("{name}: a step of 0"));
                }
                (r, st)
            }
            None => (item, 1),
        };
        let (a, b) = if range == "*" {
            if step == 1 {
                star = true;
            }
            (lo, hi)
        } else if let Some((x, y)) = range.split_once('-') {
            let (a, b) = (
                value(x, lo, hi, names, name)?,
                value(y, lo, hi, names, name)?,
            );
            if a > b {
                return Err(format!("{name}: range {a}-{b} runs backwards"));
            }
            (a, b)
        } else {
            let a = value(range, lo, hi, names, name)?;
            (a, if item.contains('/') { hi } else { a })
        };
        let mut v = a;
        while v <= b {
            bits |= 1u64 << v;
            v += step;
        }
    }
    Ok(Field { bits, star })
}

/// A compiled 5-field cron expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minute: Field,
    hour: Field,
    dom: Field,
    month: Field,
    /// 0 = Sunday … 6 = Saturday (7 folds onto 0).
    dow: Field,
    source: String,
}

impl Cron {
    pub fn parse(expr: &str) -> Result<Cron, String> {
        let t = expr.trim();
        if t.starts_with('@') {
            return Err("@ aliases are not supported; use five fields".into());
        }
        let parts: Vec<&str> = t.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!(
                "a cron needs five fields (minute hour day month weekday), got {}",
                parts.len()
            ));
        }
        let mut dow = field(parts[4], 0, 7, &DAYS, "weekday")?;
        if dow.has(7) {
            dow.bits = (dow.bits & !(1 << 7)) | 1;
        }
        Ok(Cron {
            minute: field(parts[0], 0, 59, &[], "minute")?,
            hour: field(parts[1], 0, 23, &[], "hour")?,
            dom: field(parts[2], 1, 31, &[], "day of month")?,
            month: field(parts[3], 1, 12, &MONTHS, "month")?,
            dow,
            source: parts.join(" "),
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Vixie semantics: when both day fields are restricted, either matches.
    fn day_matches(&self, d: Date) -> bool {
        if !self.month.has(d.month() as u32) {
            return false;
        }
        let dom = self.dom.has(d.day() as u32);
        let dow = self.dow.has(d.weekday().to_sunday_zero_offset() as u32);
        match (self.dom.star, self.dow.star) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            (false, false) => dom || dow,
        }
    }
}

/// A schedule ready to evaluate: the cron plan and its zone.
#[derive(Debug, Clone)]
pub struct Plan {
    pub cron: Cron,
    pub tz: TimeZone,
    pub tz_name: String,
}

impl Schedule {
    /// The 5-field equivalent of the schedule.
    pub fn cron_expr(&self) -> Result<String, ScheduleError> {
        let hm = || {
            let t = self
                .time
                .as_deref()
                .ok_or_else(|| serr("schedule.time", "Choose a time."))?;
            parse_time(t)
                .ok_or_else(|| serr("schedule.time", format!("{t:?} is not a time (HH:MM)")))
        };
        Ok(match self.kind {
            ScheduleKind::Daily => {
                let (h, m) = hm()?;
                format!("{m} {h} * * *")
            }
            ScheduleKind::Weekdays => {
                let (h, m) = hm()?;
                format!("{m} {h} * * 1-5")
            }
            ScheduleKind::Weekly => {
                let (h, m) = hm()?;
                let wd = self
                    .weekday
                    .ok_or_else(|| serr("schedule.weekday", "Choose a weekday."))?;
                if !(1..=7).contains(&wd) {
                    return Err(serr(
                        "schedule.weekday",
                        format!("{wd} is not a weekday (1–7)"),
                    ));
                }
                format!("{m} {h} * * {}", wd % 7)
            }
            ScheduleKind::Cron => self
                .cron
                .clone()
                .filter(|c| !c.trim().is_empty())
                .ok_or_else(|| serr("schedule.cron", "Enter a cron expression."))?,
        })
    }

    /// Compile against `device_tz` when the schedule names no zone.
    pub fn plan(&self, device_tz: &str) -> Result<Plan, ScheduleError> {
        let expr = self.cron_expr()?;
        let cron = Cron::parse(&expr).map_err(|e| serr("schedule.cron", e))?;
        let tz_name = self
            .tz
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| device_tz.to_string());
        let tz = TimeZone::get(&tz_name)
            .map_err(|_| serr("schedule.tz", format!("{tz_name:?} is not a time zone")))?;
        Ok(Plan { cron, tz, tz_name })
    }

    /// The schedule with `tz` filled in (stored form).
    pub fn normalised(&self, device_tz: &str) -> Schedule {
        let mut s = self.clone();
        if s.tz.as_deref().is_none_or(|t| t.trim().is_empty()) {
            s.tz = Some(device_tz.to_string());
        }
        if let Some(t) = s.time.as_deref().and_then(parse_time) {
            s.time = Some(format!("{:02}:{:02}", t.0, t.1));
        }
        if s.kind != ScheduleKind::Weekly {
            s.weekday = None;
        }
        if s.kind != ScheduleKind::Cron {
            s.cron = None;
        } else {
            s.time = None;
            s.cron = s
                .cron
                .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        s
    }
}

fn to_ts(t: OffsetDateTime) -> Timestamp {
    Timestamp::from_second(t.unix_timestamp()).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn from_ts(t: Timestamp) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(t.as_second()).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

impl Plan {
    /// The instant a wall time fires: itself; inside a spring-forward gap
    /// the first instant after the gap (the transition); inside a fall-back
    /// fold its first occurrence.
    fn instant(&self, dt: DateTime) -> Option<Timestamp> {
        let amb = self.tz.to_ambiguous_zoned(dt);
        match amb.offset() {
            jiff::tz::AmbiguousOffset::Gap { before, after } => {
                let (a, b) = (before.to_timestamp(dt).ok()?, after.to_timestamp(dt).ok()?);
                let lo = a.min(b);
                self.tz
                    .following(lo)
                    .next()
                    .map(|t| t.timestamp())
                    .filter(|t| *t <= a.max(b))
                    .or_else(|| amb.compatible().ok().map(|z| z.timestamp()))
            }
            _ => amb.compatible().ok().map(|z| z.timestamp()),
        }
    }

    fn day_min_after(&self, d: Date, after: Timestamp) -> Option<Timestamp> {
        if !self.cron.day_matches(d) {
            return None;
        }
        let mut best: Option<Timestamp> = None;
        for h in self.cron.hour.values(0, 23) {
            for m in self.cron.minute.values(0, 59) {
                let dt =
                    DateTime::from_parts(d, jiff::civil::Time::new(h as i8, m as i8, 0, 0).ok()?);
                let Some(ts) = self.instant(dt) else {
                    continue;
                };
                if ts > after && best.is_none_or(|b| ts < b) {
                    best = Some(ts);
                }
            }
        }
        best
    }

    /// The first fire time strictly after `after`; `None` when the
    /// expression never matches within eight years (e.g. 31 February).
    pub fn next_after(&self, after: OffsetDateTime) -> Option<OffsetDateTime> {
        let a = to_ts(after);
        let start = a.to_zoned(self.tz.clone()).date();
        let mut d = start.yesterday().unwrap_or(start);
        for _ in 0..(366 * 8 + 2) {
            if let Some(ts) = self.day_min_after(d, a) {
                // A DST gap can map the next day's first time earlier.
                let next = d
                    .tomorrow()
                    .ok()
                    .and_then(|n| self.day_min_after(n, a))
                    .filter(|n| *n < ts);
                return Some(from_ts(next.unwrap_or(ts)));
            }
            d = d.tomorrow().ok()?;
        }
        None
    }

    /// The next `n` fire times after `after` (the editor's preview).
    pub fn next_n(&self, after: OffsetDateTime, n: usize) -> Vec<OffsetDateTime> {
        let mut out = Vec::with_capacity(n);
        let mut cur = after;
        while out.len() < n {
            match self.next_after(cur) {
                Some(t) => {
                    out.push(t);
                    cur = t;
                }
                None => break,
            }
        }
        out
    }

    /// Fire times in `(from, to]`, at most `cap` (missed-firing counting).
    pub fn between(
        &self,
        from: OffsetDateTime,
        to: OffsetDateTime,
        cap: usize,
    ) -> Vec<OffsetDateTime> {
        let mut out = Vec::new();
        let mut cur = from;
        while out.len() < cap {
            match self.next_after(cur) {
                Some(t) if t <= to => {
                    out.push(t);
                    cur = t;
                }
                _ => break,
            }
        }
        out
    }

    /// The local wall time of an instant in the plan's zone, `YYYY-MM-DD HH:MM`.
    pub fn local(&self, t: OffsetDateTime) -> String {
        let z = to_ts(t).to_zoned(self.tz.clone());
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            z.year(),
            z.month(),
            z.day(),
            z.hour(),
            z.minute()
        )
    }
}

// ------------------------------------------------------------------ copy

const WEEKDAY_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAY_LONG: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// "New York" for `America/New_York`.
pub fn city(tz: &str) -> String {
    tz.rsplit('/').next().unwrap_or(tz).replace('_', " ")
}

/// The trigger phrase of 10 §12.1: "Daily 23:00", "Weekdays 08:00",
/// "Weekly · Mon 09:00", "Cron 0 8 * * 1-5"; " · New York" only when the
/// zone differs from the device's.
pub fn trigger_phrase(s: &Schedule, device_tz: &str) -> String {
    let time = s
        .time
        .as_deref()
        .and_then(parse_time)
        .map(|(h, m)| format!("{h:02}:{m:02}"))
        .unwrap_or_default();
    let mut p = match s.kind {
        ScheduleKind::Daily => format!("Daily {time}"),
        ScheduleKind::Weekdays => format!("Weekdays {time}"),
        ScheduleKind::Weekly => {
            let d = s
                .weekday
                .filter(|w| (1..=7).contains(w))
                .map(|w| WEEKDAY_SHORT[w as usize - 1])
                .unwrap_or("?");
            format!("Weekly · {d} {time}")
        }
        ScheduleKind::Cron => format!("Cron {}", s.cron.as_deref().unwrap_or("").trim()),
    };
    if let Some(tz) = s.tz.as_deref()
        && !tz.is_empty()
        && tz != device_tz
    {
        p.push_str(" · ");
        p.push_str(&city(tz));
    }
    p
}

fn join_words(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} and {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// The plain-language echo under the cron field (03 §7B.2: "every weekday
/// at 08:00"); a generic line when the expression has no short reading.
pub fn describe(s: &Schedule) -> String {
    let Ok(expr) = s.cron_expr() else {
        return String::new();
    };
    let Ok(c) = Cron::parse(&expr) else {
        return String::new();
    };
    let single = |f: &Field, lo: u32, hi: u32| {
        let v = f.values(lo, hi);
        (v.len() == 1).then(|| v[0])
    };
    let parts: Vec<&str> = expr.split_whitespace().collect();
    let at = match (single(&c.minute, 0, 59), single(&c.hour, 0, 23)) {
        (Some(m), Some(h)) => Some(format!("at {h:02}:{m:02}")),
        _ => None,
    };
    if let (Some(step), "*") = (parts[0].strip_prefix("*/"), parts[1])
        && parts[2] == "*"
        && parts[3] == "*"
        && parts[4] == "*"
    {
        return format!("every {step} minutes");
    }
    if parts[1] == "*" && parts[2..] == ["*", "*", "*"] {
        if let Some(m) = single(&c.minute, 0, 59) {
            return format!("every hour at :{m:02}");
        }
    }
    let Some(at) = at else {
        return "on a custom schedule".into();
    };
    let month_all = c.month.values(1, 12).len() == 12;
    if !month_all {
        return format!("on a custom schedule {at}");
    }
    let days: Vec<u32> = c.dow.values(0, 6);
    let dow_name = |d: u32| WEEKDAY_LONG[((d + 6) % 7) as usize].to_string();
    match (c.dom.star, c.dow.star) {
        (true, true) => format!("every day {at}"),
        (true, false) if days == [1, 2, 3, 4, 5] => format!("every weekday {at}"),
        (true, false) if days == [0, 6] => format!("every weekend day {at}"),
        (true, false) => {
            let mut names: Vec<u32> = days.clone();
            names.sort_by_key(|d| (d + 6) % 7);
            format!(
                "every {} {at}",
                join_words(&names.into_iter().map(dow_name).collect::<Vec<_>>())
            )
        }
        (false, true) => {
            let d = c.dom.values(1, 31);
            let list = join_words(&d.iter().map(|x| x.to_string()).collect::<Vec<_>>());
            format!("on day {list} of every month {at}")
        }
        (false, false) => format!("on a custom schedule {at}"),
    }
}

/// ISO weekday 1–7 of a jiff weekday.
pub fn iso_weekday(w: Weekday) -> u8 {
    w.to_monday_one_offset() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> OffsetDateTime {
        crate::db::parse_timestamp(s).unwrap()
    }

    fn sched(kind: ScheduleKind, time: &str, tz: &str) -> Schedule {
        Schedule {
            kind,
            time: Some(time.into()),
            tz: Some(tz.into()),
            weekday: None,
            cron: None,
        }
    }

    #[test]
    fn cron_fields_parse_with_ranges_lists_steps_and_names() {
        let c = Cron::parse("*/15 8-10 1,15 jan-mar mon-fri").unwrap();
        assert_eq!(c.minute.values(0, 59), vec![0, 15, 30, 45]);
        assert_eq!(c.hour.values(0, 23), vec![8, 9, 10]);
        assert_eq!(c.dom.values(1, 31), vec![1, 15]);
        assert_eq!(c.month.values(1, 12), vec![1, 2, 3]);
        assert_eq!(c.dow.values(0, 6), vec![1, 2, 3, 4, 5]);
        assert!(Cron::parse("0 8 * * 7").unwrap().dow.has(0), "7 is Sunday");
        assert!(Cron::parse("@daily").is_err());
        assert!(Cron::parse("0 8 * *").is_err());
        assert!(Cron::parse("60 8 * * *").is_err());
        assert!(Cron::parse("0 8 * * 1/0").is_err());
        assert!(Cron::parse("5-1 8 * * *").is_err());
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let p = sched(ScheduleKind::Weekdays, "08:00", "UTC")
            .plan("UTC")
            .unwrap();
        // Fri 2026-10-23 09:00 → Mon 26th 08:00.
        let n = p.next_after(t("2026-10-23T09:00:00Z")).unwrap();
        assert_eq!(crate::db::format_timestamp(n), "2026-10-26T08:00:00.000Z");
    }

    #[test]
    fn phrases_follow_the_copy_rules() {
        let mut s = sched(ScheduleKind::Weekdays, "8:00", "America/New_York");
        assert_eq!(
            trigger_phrase(&s, "Europe/Madrid"),
            "Weekdays 08:00 · New York"
        );
        assert_eq!(trigger_phrase(&s, "America/New_York"), "Weekdays 08:00");
        assert_eq!(describe(&s), "every weekday at 08:00");
        s.kind = ScheduleKind::Weekly;
        s.weekday = Some(1);
        assert_eq!(trigger_phrase(&s, "America/New_York"), "Weekly · Mon 08:00");
        assert_eq!(describe(&s), "every Monday at 08:00");
        let c = Schedule {
            kind: ScheduleKind::Cron,
            time: None,
            tz: None,
            weekday: None,
            cron: Some("0 8 * * 1-5".into()),
        };
        assert_eq!(trigger_phrase(&c, "UTC"), "Cron 0 8 * * 1-5");
        assert_eq!(describe(&c), "every weekday at 08:00");
        let every = Schedule {
            cron: Some("*/15 * * * *".into()),
            ..c.clone()
        };
        assert_eq!(describe(&every), "every 15 minutes");
    }
}
