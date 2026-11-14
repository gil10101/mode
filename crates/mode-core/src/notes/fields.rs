//! Field semantics (04 §5): `status` slugs (§4.6), `urgent` booleans (§5.2),
//! `when` with its relative words, buckets and the timezone rules of §5.3,
//! `tags` normalization (§5.4), the relative date tokens of groupings (§6.4),
//! and the instants Mode writes into `created` / `updated` (09 §4.1).
//!
//! Local dates carry no zone. "Today" is the device's local date; the core
//! asks SQLite (`localtime` / `utc` modifiers, the C library's zone rules)
//! through [`SqliteTz`] so DST and zone changes follow the OS, and tests use
//! [`FixedTz`].

use rusqlite::Connection;
use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset, Weekday};

// ------------------------------------------------------------------ dates

/// Parse a strict `YYYY-MM-DD` local date.
pub fn parse_date(s: &str) -> Option<Date> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !s
        .bytes()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u8 = s[5..7].parse().ok()?;
    let day: u8 = s[8..10].parse().ok()?;
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

/// `YYYY-MM-DD`.
pub fn fmt_date(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// `d + n` days as `YYYY-MM-DD`.
pub fn add_days(d: &str, n: i64) -> Option<String> {
    parse_date(d)
        .and_then(|d| d.checked_add(Duration::days(n)))
        .map(fmt_date)
}

/// The first day of the week (`settings['notes.week_start']`, 04 §16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekStart {
    #[default]
    Monday,
    Sunday,
    Saturday,
}

impl WeekStart {
    pub fn parse(s: &str) -> WeekStart {
        match s.trim().to_ascii_lowercase().as_str() {
            "sunday" => WeekStart::Sunday,
            "saturday" => WeekStart::Saturday,
            _ => WeekStart::Monday,
        }
    }

    fn weekday(self) -> Weekday {
        match self {
            WeekStart::Monday => Weekday::Monday,
            WeekStart::Sunday => Weekday::Sunday,
            WeekStart::Saturday => Weekday::Saturday,
        }
    }
}

/// The start of the week containing `d`.
pub fn week_start_of(d: Date, ws: WeekStart) -> Date {
    let back = (i64::from(d.weekday().number_days_from_monday()) + 7
        - i64::from(ws.weekday().number_days_from_monday()))
        % 7;
    d - Duration::days(back)
}

pub fn month_start(d: Date) -> Date {
    d.replace_day(1).unwrap_or(d)
}

pub fn month_end(d: Date) -> Date {
    let (y, m) = if d.month() == Month::December {
        (d.year() + 1, Month::January)
    } else {
        (d.year(), d.month().next())
    };
    Date::from_calendar_date(y, m, 1)
        .map(|first| first - Duration::days(1))
        .unwrap_or(d)
}

/// A weekday name (`mon` … `sun`, `monday` … `sunday`, `tues`, `thur`, `thurs`), case-insensitive.
pub fn weekday_from_name(s: &str) -> Option<Weekday> {
    Some(match s.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Weekday::Monday,
        "tue" | "tues" | "tuesday" => Weekday::Tuesday,
        "wed" | "weds" | "wednesday" => Weekday::Wednesday,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thursday,
        "fri" | "friday" => Weekday::Friday,
        "sat" | "saturday" => Weekday::Saturday,
        "sun" | "sunday" => Weekday::Sunday,
        _ => return None,
    })
}

/// The next `wd` on or after `today` (today included, 04 §7.2 and §8.2).
pub fn next_weekday(today: Date, wd: Weekday) -> Date {
    let ahead = (i64::from(wd.number_days_from_monday()) + 7
        - i64::from(today.weekday().number_days_from_monday()))
        % 7;
    today + Duration::days(ahead)
}

/// A `from` / `to` bound of `filter_json` (04 §6.4): a literal date or a
/// relative token `today`, `tomorrow`, `week_start`, `week_end`,
/// `month_start`, `month_end`, `next_month_start`, `next_month_end`, each
/// optionally followed by `+N` / `-N` days (`today+7`, `week_start+7`).
/// `None` for anything else.
pub fn resolve_date_token(token: &str, today: Date, ws: WeekStart) -> Option<Date> {
    let token = token.trim();
    if let Some(d) = parse_date(token) {
        return Some(d);
    }
    let (anchor, offset) = match token.find(['+', '-']) {
        Some(i) => {
            let n: i64 = token[i + 1..].parse().ok()?;
            if !(0..=3660).contains(&n) {
                return None;
            }
            let n = if token.as_bytes()[i] == b'-' { -n } else { n };
            (&token[..i], n)
        }
        None => (token, 0),
    };
    let base = match anchor {
        "today" => today,
        "tomorrow" => today + Duration::days(1),
        "yesterday" => today - Duration::days(1),
        "week_start" => week_start_of(today, ws),
        "week_end" => week_start_of(today, ws) + Duration::days(6),
        "month_start" => month_start(today),
        "month_end" => month_end(today),
        "next_month_start" => month_end(today) + Duration::days(1),
        "next_month_end" => month_end(month_end(today) + Duration::days(1)),
        _ => return None,
    };
    base.checked_add(Duration::days(offset))
}

/// Whether `s` is a valid `from` / `to` bound.
pub fn is_date_token(s: &str) -> bool {
    let anchor = Date::from_calendar_date(2000, Month::January, 3).unwrap_or(Date::MIN);
    resolve_date_token(s, anchor, WeekStart::Monday).is_some()
}

// ------------------------------------------------------------------ zones

/// Local ↔ UTC conversions (04 §5.3: dates are local and never converted;
/// instants are UTC).
pub trait LocalTz {
    /// The local wall-clock time of a UTC instant.
    fn to_local(&self, utc: OffsetDateTime) -> PrimitiveDateTime;
    /// The UTC instant of a local wall-clock time.
    fn to_utc(&self, local: PrimitiveDateTime) -> OffsetDateTime;

    fn local_date(&self, utc: OffsetDateTime) -> Date {
        self.to_local(utc).date()
    }

    /// Local midnight of `d` as a UTC instant.
    fn local_midnight(&self, d: Date) -> OffsetDateTime {
        self.to_utc(PrimitiveDateTime::new(d, Time::MIDNIGHT))
    }
}

/// A fixed offset in minutes east of UTC (tests, and a fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FixedTz(pub i32);

impl FixedTz {
    fn offset(self) -> UtcOffset {
        UtcOffset::from_whole_seconds(self.0.saturating_mul(60)).unwrap_or(UtcOffset::UTC)
    }
}

impl LocalTz for FixedTz {
    fn to_local(&self, utc: OffsetDateTime) -> PrimitiveDateTime {
        let t = utc.to_offset(self.offset());
        PrimitiveDateTime::new(t.date(), t.time())
    }

    fn to_utc(&self, local: PrimitiveDateTime) -> OffsetDateTime {
        local.assume_offset(self.offset()).to_offset(UtcOffset::UTC)
    }
}

/// The OS zone through SQLite's `localtime` / `utc` modifiers.
pub struct SqliteTz<'c>(pub &'c Connection);

fn sql_wall(t: PrimitiveDateTime) -> String {
    format!(
        "{} {:02}:{:02}:{:02}.{:03}",
        fmt_date(t.date()),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

fn parse_wall(s: &str) -> Option<PrimitiveDateTime> {
    let t = crate::db::parse_timestamp(&s.replacen(' ', "T", 1))?;
    Some(PrimitiveDateTime::new(t.date(), t.time()))
}

impl LocalTz for SqliteTz<'_> {
    fn to_local(&self, utc: OffsetDateTime) -> PrimitiveDateTime {
        let utc = utc.to_offset(UtcOffset::UTC);
        let wall = PrimitiveDateTime::new(utc.date(), utc.time());
        self.0
            .query_row(
                "SELECT strftime('%Y-%m-%d %H:%M:%f', ?1, 'localtime')",
                [sql_wall(wall)],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .and_then(|s| parse_wall(&s))
            .unwrap_or(wall)
    }

    fn to_utc(&self, local: PrimitiveDateTime) -> OffsetDateTime {
        self.0
            .query_row(
                "SELECT strftime('%Y-%m-%d %H:%M:%f', ?1, 'utc')",
                [sql_wall(local)],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .and_then(|s| parse_wall(&s))
            .map(|w| w.assume_utc())
            .unwrap_or_else(|| local.assume_utc())
    }
}

// --------------------------------------------------------------- instants

/// 09 §4.1 timestamp: `2026-10-18T20:17:03.123Z`.
pub fn fmt_instant(t: OffsetDateTime) -> String {
    crate::db::format_timestamp(t)
}

/// Parse an instant from frontmatter (04 §3.1 `created` / `updated`): any
/// RFC 3339 instant (`Z` or `±HH:MM`, fraction optional, `T` or a space);
/// a date-time without an offset is local wall-clock time (what Obsidian
/// writes); a bare date is local midnight.
pub fn parse_instant(s: &str, tz: &dyn LocalTz) -> Option<OffsetDateTime> {
    let s = s.trim();
    if let Some(d) = parse_date(s) {
        return Some(tz.local_midnight(d));
    }
    if s.len() < 16 {
        return None;
    }
    let (date_part, rest) = (&s[..10], &s[11..]);
    let sep = s.as_bytes()[10];
    if sep != b'T' && sep != b't' && sep != b' ' {
        return None;
    }
    let date = parse_date(date_part)?;
    // Split the offset off the time.
    let (time_part, offset) = if let Some(t) = rest.strip_suffix(['Z', 'z']) {
        (t, Some(0i32))
    } else if let Some(i) = rest.rfind(['+', '-']).filter(|i| *i >= 5) {
        let off = &rest[i + 1..];
        let (h, m) = match off.split_once(':') {
            Some((h, m)) => (h, m),
            None if off.len() == 4 => (&off[..2], &off[2..]),
            None if off.len() == 2 => (off, "00"),
            None => return None,
        };
        let h: i32 = h.parse().ok()?;
        let m: i32 = m.parse().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        let sign = if rest.as_bytes()[i] == b'-' { -1 } else { 1 };
        (&rest[..i], Some(sign * (h * 60 + m)))
    } else {
        (rest, None)
    };
    let time = parse_time(time_part.trim())?;
    let wall = PrimitiveDateTime::new(date, time);
    Some(match offset {
        Some(minutes) => {
            let off = UtcOffset::from_whole_seconds(minutes * 60).ok()?;
            wall.assume_offset(off).to_offset(UtcOffset::UTC)
        }
        None => tz.to_utc(wall),
    })
}

fn parse_time(s: &str) -> Option<Time> {
    let mut it = s.split(':');
    let h: u8 = it.next()?.parse().ok()?;
    let m: u8 = it.next()?.parse().ok()?;
    let (sec, milli) = match it.next() {
        Some(sec) => {
            let (whole, frac) = sec.split_once(['.', ',']).unwrap_or((sec, ""));
            if !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let frac: String = frac.chars().take(3).collect();
            let milli: u16 = if frac.is_empty() {
                0
            } else {
                format!("{frac:0<3}").parse().ok()?
            };
            (whole.parse::<u8>().ok()?, milli)
        }
        None => (0, 0),
    };
    if it.next().is_some() {
        return None;
    }
    Time::from_hms_milli(h, m, sec.min(59), milli).ok()
}

/// `created`'s and `updated`'s canonical text from any accepted spelling.
pub fn canonical_instant(s: &str, tz: &dyn LocalTz) -> Option<String> {
    parse_instant(s, tz).map(fmt_instant)
}

// ------------------------------------------------------------------ urgent

/// `true`, `false`, `yes`, `no`, `1`, `0`, case-insensitive (04 §3.1).
pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

// ------------------------------------------------------------------ status

/// Chip words that cannot be status slugs (04 §4.6).
pub const RESERVED_STATUSES: &[&str] = &[
    "urgent",
    "overdue",
    "today",
    "tomorrow",
    "scheduled",
    "none",
    "pinned",
    "archived",
];

/// The premade statuses (04 §4.6): they cannot be removed.
pub const PREMADE_STATUSES: &[&str] = &["todo", "doing", "done"];

/// The four built-in glyphs a custom status may not reuse.
pub const BUILTIN_GLYPHS: &[&str] = &["\u{25CB}", "\u{25D1}", "\u{2731}", "\u{2713}"];

/// The glyph of a status not in `notes.statuses` (04 §3.1 assumption).
pub const UNKNOWN_GLYPH: &str = "?";

/// `^[a-z][a-z0-9-]{0,31}$`.
pub fn is_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Whether `slug` may be stored as a custom status (04 §4.6).
pub fn custom_status_ok(slug: &str) -> Result<(), String> {
    if !is_slug(slug) {
        return Err(format!(
            "{slug:?} is not a slug (a-z, 0-9, '-', 1-32 characters, starting with a letter)"
        ));
    }
    if RESERVED_STATUSES.contains(&slug) {
        return Err(format!("{slug:?} is a reserved word"));
    }
    Ok(())
}

/// Whether `g` is one grapheme cluster (approximately: one character plus
/// combining marks, variation selectors, skin tones and ZWJ sequences).
pub fn single_grapheme(g: &str) -> bool {
    let mut chars = g.chars();
    if chars.next().is_none() {
        return false;
    }
    let mut joined = false;
    for c in chars {
        let cp = c as u32;
        let extends = (0x0300..=0x036F).contains(&cp)
            || (0xFE00..=0xFE0F).contains(&cp)
            || (0x1F3FB..=0x1F3FF).contains(&cp)
            || (0x20D0..=0x20FF).contains(&cp)
            || cp == 0x200D;
        if cp == 0x200D {
            joined = true;
            continue;
        }
        if extends {
            continue;
        }
        if joined {
            joined = false;
            continue;
        }
        return false;
    }
    true
}

/// Validate a new `settings['notes.statuses']` list (04 §4.6): the three
/// premade entries stay, slugs are unique, well-formed and not reserved,
/// glyphs are one grapheme and custom ones differ from the built-ins, and
/// a status still used by notes cannot be removed (`in_use`: slug → count).
pub fn validate_statuses(
    list: &[crate::settings::StatusDef],
    in_use: &[(String, i64)],
) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::new();
    for s in list {
        if seen.contains(&s.slug.as_str()) {
            return Err(format!("{:?} is listed twice", s.slug));
        }
        seen.push(&s.slug);
        let premade = PREMADE_STATUSES.contains(&s.slug.as_str());
        if !premade {
            custom_status_ok(&s.slug)?;
        }
        if s.label.trim().is_empty() {
            return Err(format!("{:?} needs a label", s.slug));
        }
        match s.glyph.as_deref() {
            Some(g) if !single_grapheme(g) => {
                return Err(format!("the glyph of {:?} must be one character", s.slug));
            }
            Some(g) if !premade && BUILTIN_GLYPHS.contains(&g) => {
                return Err(format!(
                    "the glyph of {:?} must differ from the built-in glyphs",
                    s.slug
                ));
            }
            _ => {}
        }
    }
    for p in PREMADE_STATUSES {
        if !seen.contains(p) {
            return Err(format!("the premade status {p:?} cannot be removed"));
        }
    }
    for (slug, n) in in_use {
        if *n > 0 && !seen.contains(&slug.as_str()) {
            return Err(format!(
                "{n} notes use {slug:?}; move them to another status first"
            ));
        }
    }
    Ok(())
}

/// A status value as read from a file: trimmed and case-folded (04 §3.1).
pub fn fold_status(raw: &str) -> String {
    raw.trim().to_lowercase()
}

// -------------------------------------------------------------------- tags

/// A tag's display form (04 §5.4): leading `#` stripped, trimmed, internal
/// whitespace collapsed to one space, case kept. `None` when the result is
/// empty, longer than 64 characters or carries `#`, `,`, `[`, `]` or `"`.
pub fn tag_display(raw: &str) -> Option<String> {
    let s = raw.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = s.chars().count();
    if n == 0 || n > 64 || s.contains(['#', ',', '[', ']', '"']) {
        return None;
    }
    Some(s)
}

/// A tag's identity, `tags.name_norm` (lowercase display form).
pub fn tag_norm(raw: &str) -> Option<String> {
    tag_display(raw).map(|s| s.to_lowercase())
}

/// Normalize a tag list: invalid items dropped (returned separately),
/// duplicates collapsed by `name_norm`, first appearance kept (04 §3.3).
pub fn normalize_tags<I, S>(items: I) -> (Vec<String>, Vec<String>)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut invalid = Vec::new();
    for item in items {
        match tag_display(item.as_ref()) {
            Some(d) => {
                let norm = d.to_lowercase();
                if !seen.contains(&norm) {
                    seen.push(norm);
                    out.push(d);
                }
            }
            None => {
                if !item.as_ref().trim().is_empty() {
                    invalid.push(item.as_ref().to_string());
                }
            }
        }
    }
    (out, invalid)
}

/// A `tags` value given as one string: comma-separated when it has commas,
/// else whitespace-separated (04 §3.1).
pub fn split_tag_string(s: &str) -> Vec<String> {
    if s.contains(',') {
        s.split(',').map(|t| t.trim().to_string()).collect()
    } else {
        s.split_whitespace().map(str::to_string).collect()
    }
}

// -------------------------------------------------------------------- when

/// A `when` value as written in a file (04 §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhenRaw {
    Today,
    Tomorrow,
    Date(Date),
}

impl WhenRaw {
    /// Read `today`, `tomorrow`, `YYYY-MM-DD` or a date-time (its date part).
    pub fn parse(s: &str) -> Option<WhenRaw> {
        let t = s.trim();
        match t.to_ascii_lowercase().as_str() {
            "today" => return Some(WhenRaw::Today),
            "tomorrow" => return Some(WhenRaw::Tomorrow),
            _ => {}
        }
        if let Some(d) = parse_date(t) {
            return Some(WhenRaw::Date(d));
        }
        if t.len() > 10 && matches!(t.as_bytes()[10], b'T' | b't' | b' ') {
            return parse_date(&t[..10]).map(WhenRaw::Date);
        }
        None
    }

    /// The resolved local date against `anchor` (the local date of the
    /// frontmatter `updated`, else of the file mtime).
    pub fn resolve(&self, anchor: Date) -> Date {
        match self {
            WhenRaw::Today => anchor,
            WhenRaw::Tomorrow => anchor + Duration::days(1),
            WhenRaw::Date(d) => *d,
        }
    }
}

/// A `when` value from a picker, a token or an API call: `today`,
/// `tomorrow`, a weekday name, `YYYY-MM-DD`, a relative token; `none` /
/// empty clears. Mode always stores the resolved date (04 §5.3).
pub fn resolve_when_input(s: &str, today: Date, ws: WeekStart) -> Result<Option<Date>, String> {
    let t = s.trim().trim_start_matches('@');
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if let Some(wd) = weekday_from_name(t) {
        return Ok(Some(next_weekday(today, wd)));
    }
    if let Some(d) = resolve_date_token(&t.to_ascii_lowercase(), today, ws) {
        return Ok(Some(d));
    }
    Err(format!("{s:?} is not a date"))
}

/// The when bucket of 04 §5.3, in its order of precedence.
pub fn when_bucket(
    when_date: Option<&str>,
    closed: bool,
    today: &str,
    tomorrow: &str,
) -> &'static str {
    match when_date {
        None => "none",
        Some(d) if d < today && !closed => "overdue",
        Some(d) if d == today => "today",
        Some(d) if d == tomorrow => "tomorrow",
        Some(_) => "scheduled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        parse_date(s).unwrap()
    }

    #[test]
    fn dates_and_tokens() {
        assert!(parse_date("2026-02-30").is_none());
        assert!(parse_date("2026-1-01").is_none());
        assert_eq!(add_days("2026-10-31", 1).as_deref(), Some("2026-11-01"));
        // 2026-10-18 is a Sunday.
        let today = d("2026-10-18");
        assert_eq!(today.weekday(), Weekday::Sunday);
        let r = |t: &str| resolve_date_token(t, today, WeekStart::Monday).map(fmt_date);
        assert_eq!(r("week_start").as_deref(), Some("2026-10-12"));
        assert_eq!(r("week_end").as_deref(), Some("2026-10-18"));
        assert_eq!(r("week_start+7").as_deref(), Some("2026-10-19"));
        assert_eq!(r("today+7").as_deref(), Some("2026-10-25"));
        assert_eq!(r("today-1").as_deref(), Some("2026-10-17"));
        assert_eq!(r("month_start").as_deref(), Some("2026-10-01"));
        assert_eq!(r("month_end").as_deref(), Some("2026-10-31"));
        assert_eq!(r("next_month_start").as_deref(), Some("2026-11-01"));
        assert_eq!(r("next_month_end").as_deref(), Some("2026-11-30"));
        assert_eq!(r("2026-01-02").as_deref(), Some("2026-01-02"));
        assert_eq!(r("someday"), None);
        assert_eq!(
            resolve_date_token("week_start", today, WeekStart::Sunday)
                .map(fmt_date)
                .as_deref(),
            Some("2026-10-18")
        );
        assert_eq!(
            resolve_date_token("week_start", today, WeekStart::Saturday)
                .map(fmt_date)
                .as_deref(),
            Some("2026-10-17")
        );
        assert!(is_date_token("today+3"));
        assert!(!is_date_token("today+x"));
        assert_eq!(fmt_date(next_weekday(today, Weekday::Sunday)), "2026-10-18");
        assert_eq!(fmt_date(next_weekday(today, Weekday::Friday)), "2026-10-23");
        assert_eq!(fmt_date(month_end(d("2028-02-03"))), "2028-02-29");
    }

    #[test]
    fn instants_and_zones() {
        let berlin = FixedTz(120);
        assert_eq!(
            canonical_instant("2026-10-18T09:12:44.120Z", &berlin).as_deref(),
            Some("2026-10-18T09:12:44.120Z")
        );
        assert_eq!(
            canonical_instant("2026-10-18T11:12:44+02:00", &berlin).as_deref(),
            Some("2026-10-18T09:12:44.000Z")
        );
        assert_eq!(
            canonical_instant("2026-10-18 11:12", &berlin).as_deref(),
            Some("2026-10-18T09:12:00.000Z"),
            "no offset = local wall-clock time"
        );
        assert_eq!(
            canonical_instant("2026-10-18", &berlin).as_deref(),
            Some("2026-10-17T22:00:00.000Z"),
            "a bare date is local midnight"
        );
        assert_eq!(canonical_instant("yesterday", &berlin), None);
        assert_eq!(
            fmt_date(berlin.local_date(parse_instant("2026-10-18T23:30:00Z", &berlin).unwrap())),
            "2026-10-19"
        );
    }

    #[test]
    fn tags_status_when() {
        assert_eq!(tag_display("  #Big   Project "), Some("Big Project".into()));
        assert_eq!(tag_norm("#Gym").as_deref(), Some("gym"));
        assert_eq!(tag_display("a,b"), None);
        assert_eq!(tag_display(&"x".repeat(65)), None);
        let (tags, bad) = normalize_tags(["Groceries", "groceries", " errands", "[x]", ""]);
        assert_eq!(tags, vec!["Groceries", "errands"]);
        assert_eq!(bad, vec!["[x]"]);
        assert_eq!(split_tag_string("a, b c"), vec!["a", "b c"]);
        assert_eq!(split_tag_string("a b"), vec!["a", "b"]);
        assert!(is_slug("waiting-on"));
        assert!(!is_slug("Waiting"));
        assert!(custom_status_ok("urgent").is_err());
        assert_eq!(fold_status(" Doing "), "doing");
        assert!(single_grapheme("\u{25A3}") && single_grapheme("\u{1F3CB}\u{FE0F}"));
        assert!(single_grapheme("\u{1F468}\u{200D}\u{1F4BB}") && !single_grapheme("ab"));
        assert_eq!(parse_bool("YES"), Some(true));
        assert_eq!(parse_bool("maybe"), None);
        assert_eq!(WhenRaw::parse("Today"), Some(WhenRaw::Today));
        assert_eq!(
            WhenRaw::parse("2026-10-19T10:00:00Z"),
            Some(WhenRaw::Date(d("2026-10-19")))
        );
        assert_eq!(WhenRaw::parse("next week"), None);
        assert_eq!(WhenRaw::Tomorrow.resolve(d("2026-10-18")), d("2026-10-19"));
        let today = d("2026-10-18");
        assert_eq!(
            resolve_when_input("@fri", today, WeekStart::Monday).unwrap(),
            Some(d("2026-10-23"))
        );
        assert_eq!(
            resolve_when_input("none", today, WeekStart::Monday).unwrap(),
            None
        );
        assert!(resolve_when_input("2026-02-30", today, WeekStart::Monday).is_err());
        assert_eq!(
            when_bucket(Some("2026-10-17"), false, "2026-10-18", "2026-10-19"),
            "overdue"
        );
        assert_eq!(
            when_bucket(Some("2026-10-17"), true, "2026-10-18", "2026-10-19"),
            "scheduled"
        );
        assert_eq!(when_bucket(None, false, "2026-10-18", "2026-10-19"), "none");
    }
}
