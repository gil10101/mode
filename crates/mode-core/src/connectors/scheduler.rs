//! The poll scheduler (06 §2.3): which sources are due (interval per
//! provider or `account_sources.poll_seconds`, the Slack inactivity ladder,
//! the foreground boost), in what order (priority, then oldest
//! `last_polled_at`), under which budget (one token bucket per account and
//! service per minute: Gmail 100, Calendar 20, Slack
//! `polling.slack_budget_per_minute`), and what pauses it (offline, account
//! not `active`, a 429's `Retry-After`, the backoff of 06 §6.9). A source
//! that would exceed its bucket waits for the next minute; it is never
//! skipped silently (the Diagnostics line says "budget").

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use super::{ConnectorError, ConnectorSettings, Service, SourceKind};
use crate::db::parse_timestamp;
use crate::repo::{Account, AccountSource};

/// The scheduler ticks every second.
pub const TICK: std::time::Duration = std::time::Duration::from_secs(1);
pub const GMAIL_BUCKET: u32 = 100;
pub const CALENDAR_BUCKET: u32 = 20;
/// After this long without a poll the next one is a catch-up (06 §2.3).
pub const CATCH_UP_AFTER_MINUTES: i64 = 10;
/// The foreground boost never polls faster than this.
pub const FOREGROUND_MIN_SECS: i64 = 15;
/// Gmail backoff (06 §6.9): 1, 2, 4, 8, 16 s with full jitter, 5 tries.
pub const BACKOFF_MAX_TRIES: u32 = 5;

/// A per-minute token bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBucket {
    pub capacity: u32,
    used: u32,
    minute: i64,
}

impl TokenBucket {
    pub fn new(capacity: u32) -> TokenBucket {
        TokenBucket {
            capacity,
            used: 0,
            minute: i64::MIN,
        }
    }

    fn roll(&mut self, now: OffsetDateTime) {
        let m = now.unix_timestamp().div_euclid(60);
        if m != self.minute {
            self.minute = m;
            self.used = 0;
        }
    }

    pub fn remaining(&mut self, now: OffsetDateTime) -> u32 {
        self.roll(now);
        self.capacity.saturating_sub(self.used)
    }

    pub fn used(&mut self, now: OffsetDateTime) -> u32 {
        self.roll(now);
        self.used
    }

    /// Count `n` requests spent this minute.
    pub fn consume(&mut self, n: u32, now: OffsetDateTime) {
        self.roll(now);
        self.used = self.used.saturating_add(n);
    }
}

/// The bucket size of a service (06 §2.3).
pub fn bucket_capacity(service: Service, settings: &ConnectorSettings) -> u32 {
    match service {
        Service::Gmail => GMAIL_BUCKET,
        Service::Calendar => CALENDAR_BUCKET,
        Service::Slack => settings.slack_budget_per_minute.clamp(1, 10_000) as u32,
    }
}

/// The base interval of a source kind, in seconds.
pub fn base_interval(kind: SourceKind, settings: &ConnectorSettings) -> i64 {
    match kind {
        SourceKind::GmailInbox | SourceKind::GmailLabel => settings.gmail_seconds,
        SourceKind::Calendar => settings.calendar_seconds,
        SourceKind::SlackMentions => settings.slack_search_mentions_seconds,
        _ => settings.slack_seconds,
    }
}

/// The Slack inactivity ladder (06 §2.3): base if a message arrived in the
/// last 10 min, 2 min within the hour, 10 min within the day, 30 min beyond.
pub fn slack_ladder(
    base: i64,
    last_message_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> i64 {
    match last_message_at {
        Some(t) if now - t <= Duration::minutes(10) => base,
        Some(t) if now - t <= Duration::hours(1) => base.max(120),
        Some(t) if now - t <= Duration::days(1) => base.max(600),
        _ => base.max(1800),
    }
}

/// The interval a source runs at now.
pub fn effective_interval(
    source: &AccountSource,
    settings: &ConnectorSettings,
    last_message_at: Option<OffsetDateTime>,
    foreground: bool,
    now: OffsetDateTime,
) -> i64 {
    let Some(kind) = SourceKind::parse(&source.kind) else {
        return 300;
    };
    let base = source
        .poll_seconds
        .filter(|s| *s > 0)
        .unwrap_or_else(|| base_interval(kind, settings));
    let mut i = match kind {
        SourceKind::SlackDm | SourceKind::SlackMpim | SourceKind::SlackChannel => {
            slack_ladder(base, last_message_at, now)
        }
        _ => base,
    };
    let boosted = matches!(
        kind,
        SourceKind::GmailInbox
            | SourceKind::GmailLabel
            | SourceKind::SlackDm
            | SourceKind::SlackMpim
    );
    if foreground && boosted {
        i = (i / 2).max(FOREGROUND_MIN_SECS);
    }
    i
}

/// A source to poll now.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub account: Account,
    pub source: AccountSource,
    pub service: Service,
    /// More than 10 min since the last poll (sleep, offline).
    pub catch_up: bool,
}

/// Why nothing ran for an account (Diagnostics).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waiting {
    Offline,
    Paused,
    RateLimited,
    Budget,
}

/// The scheduler's in-memory state.
#[derive(Debug, Default)]
pub struct Scheduler {
    buckets: HashMap<(String, Service), TokenBucket>,
    paused_until: HashMap<String, OffsetDateTime>,
    backoff_tries: HashMap<String, u32>,
    failures: HashMap<String, u32>,
    last_ok: HashMap<String, (bool, bool)>,
    /// Newest message time per source (the ladder).
    last_message: HashMap<String, OffsetDateTime>,
    /// Account caps lowered by the user (06 §3.2; never above the default).
    caps: HashMap<(String, Service), u32>,
    pub offline: bool,
    pub foreground: bool,
    wake_pending: bool,
    pub waiting: HashMap<String, Waiting>,
}

impl Scheduler {
    pub fn new() -> Scheduler {
        Scheduler::default()
    }

    /// Lower an account's bucket (clamped to the provider default).
    pub fn set_cap(
        &mut self,
        account_id: &str,
        service: Service,
        cap: u32,
        settings: &ConnectorSettings,
    ) {
        let max = bucket_capacity(service, settings);
        self.caps
            .insert((account_id.to_string(), service), cap.clamp(1, max));
    }

    fn bucket(
        &mut self,
        account_id: &str,
        service: Service,
        settings: &ConnectorSettings,
    ) -> &mut TokenBucket {
        let cap = self
            .caps
            .get(&(account_id.to_string(), service))
            .copied()
            .unwrap_or_else(|| bucket_capacity(service, settings));
        let b = self
            .buckets
            .entry((account_id.to_string(), service))
            .or_insert_with(|| TokenBucket::new(cap));
        b.capacity = cap;
        b
    }

    /// Requests left this minute for an account's service.
    pub fn budget_for(
        &mut self,
        account_id: &str,
        service: Service,
        settings: &ConnectorSettings,
        now: OffsetDateTime,
    ) -> u32 {
        self.bucket(account_id, service, settings).remaining(now)
    }

    pub fn budget_used(
        &mut self,
        account_id: &str,
        service: Service,
        settings: &ConnectorSettings,
        now: OffsetDateTime,
    ) -> (u32, u32) {
        let b = self.bucket(account_id, service, settings);
        (b.used(now), b.capacity)
    }

    pub fn consume(
        &mut self,
        account_id: &str,
        service: Service,
        n: u32,
        settings: &ConnectorSettings,
        now: OffsetDateTime,
    ) {
        self.bucket(account_id, service, settings).consume(n, now);
    }

    /// Note the newest message of a source (resets the ladder).
    pub fn saw_message(&mut self, source_id: &str, at: OffsetDateTime) {
        let e = self.last_message.entry(source_id.to_string()).or_insert(at);
        if at > *e {
            *e = at;
        }
    }

    pub fn last_message(&self, source_id: &str) -> Option<OffsetDateTime> {
        self.last_message.get(source_id).copied()
    }

    /// The OS woke Mode: every source polls at once (06 §2.3 "app asleep").
    pub fn wake(&mut self) {
        self.wake_pending = true;
    }

    pub fn paused_until(&self, account_id: &str) -> Option<OffsetDateTime> {
        self.paused_until.get(account_id).copied()
    }

    pub fn consecutive_failures(&self, account_id: &str) -> u32 {
        self.failures.get(account_id).copied().unwrap_or(0)
    }

    pub fn last_polls(&self, account_id: &str) -> (bool, bool) {
        self.last_ok
            .get(account_id)
            .copied()
            .unwrap_or((true, true))
    }

    /// The due sources, ordered by (priority, oldest `last_polled_at`).
    pub fn plan(
        &mut self,
        now: OffsetDateTime,
        accounts: &[Account],
        sources: &[AccountSource],
        settings: &ConnectorSettings,
    ) -> Vec<Planned> {
        self.waiting.clear();
        let wake = std::mem::take(&mut self.wake_pending);
        let mut out: Vec<(u8, Option<String>, Planned)> = Vec::new();
        for a in accounts {
            if self.offline {
                self.waiting.insert(a.id.clone(), Waiting::Offline);
                continue;
            }
            if a.status != "active" {
                self.waiting.insert(a.id.clone(), Waiting::Paused);
                continue;
            }
            if let Some(t) = self.paused_until.get(&a.id)
                && *t > now
            {
                self.waiting.insert(a.id.clone(), Waiting::RateLimited);
                continue;
            }
            let services = super::json_strings(&a.services_json);
            for s in sources.iter().filter(|s| s.account_id == a.id && s.enabled) {
                let Some(kind) = SourceKind::parse(&s.kind) else {
                    continue;
                };
                let service = kind.service();
                if a.provider == "google" && !services.iter().any(|x| x == service.as_str()) {
                    continue;
                }
                let last = s.last_polled_at.as_deref().and_then(parse_timestamp);
                let interval = effective_interval(
                    s,
                    settings,
                    self.last_message.get(&s.id).copied(),
                    self.foreground,
                    now,
                );
                let due = wake || last.is_none_or(|t| now - t >= Duration::seconds(interval));
                if !due {
                    continue;
                }
                let catch_up =
                    last.is_some_and(|t| now - t > Duration::minutes(CATCH_UP_AFTER_MINUTES));
                out.push((
                    kind.priority(),
                    s.last_polled_at.clone(),
                    Planned {
                        account: a.clone(),
                        source: s.clone(),
                        service,
                        catch_up,
                    },
                ));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut planned = Vec::new();
        for (_, _, p) in out {
            if self.budget_for(&p.account.id, p.service, settings, now) == 0 {
                self.waiting.insert(p.account.id.clone(), Waiting::Budget);
                continue;
            }
            planned.push(p);
        }
        planned
    }

    /// Record a poll's result; returns the consecutive failure count.
    pub fn record(
        &mut self,
        account_id: &str,
        result: Result<(), &ConnectorError>,
        now: OffsetDateTime,
    ) -> u32 {
        let (prev, _) = self.last_polls(account_id);
        match result {
            Ok(()) => {
                self.failures.remove(account_id);
                self.backoff_tries.remove(account_id);
                self.last_ok.insert(account_id.to_string(), (true, prev));
                0
            }
            Err(ConnectorError::RateLimited { retry_after_secs }) => {
                self.paused_until.insert(
                    account_id.to_string(),
                    now + Duration::seconds(*retry_after_secs as i64),
                );
                self.consecutive_failures(account_id)
            }
            Err(ConnectorError::BudgetExhausted) => self.consecutive_failures(account_id),
            Err(e) => {
                self.last_ok.insert(account_id.to_string(), (false, prev));
                if matches!(e, ConnectorError::Server(..)) {
                    let tries = self
                        .backoff_tries
                        .entry(account_id.to_string())
                        .or_insert(0);
                    *tries += 1;
                    if *tries <= BACKOFF_MAX_TRIES {
                        let d = backoff_delay(*tries, jitter());
                        self.paused_until.insert(account_id.to_string(), now + d);
                    } else {
                        self.backoff_tries.remove(account_id);
                    }
                }
                if e.counts_as_failure() {
                    let n = self.failures.entry(account_id.to_string()).or_insert(0);
                    *n += 1;
                    *n
                } else {
                    self.consecutive_failures(account_id)
                }
            }
        }
    }

    /// Forget an account (removed).
    pub fn forget(&mut self, account_id: &str) {
        self.buckets.retain(|(a, _), _| a != account_id);
        self.caps.retain(|(a, _), _| a != account_id);
        self.paused_until.remove(account_id);
        self.failures.remove(account_id);
        self.backoff_tries.remove(account_id);
        self.last_ok.remove(account_id);
    }
}

/// `2^(try-1)` seconds with full jitter (`jitter` in 0–1).
pub fn backoff_delay(try_n: u32, jitter: f64) -> Duration {
    let base_ms = 1000i64 << (try_n.clamp(1, 5) - 1);
    Duration::milliseconds((base_ms as f64 * jitter.clamp(0.0, 1.0)).round() as i64)
}

fn jitter() -> f64 {
    let mut b = [0u8; 2];
    if getrandom::fill(&mut b).is_err() {
        return 0.5;
    }
    u16::from_le_bytes(b) as f64 / u16::MAX as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> OffsetDateTime {
        parse_timestamp(s).unwrap()
    }

    #[test]
    fn bucket_resets_each_minute() {
        let mut b = TokenBucket::new(40);
        let now = t("2026-10-22T10:00:05Z");
        b.consume(39, now);
        assert_eq!(b.remaining(now), 1);
        b.consume(5, now);
        assert_eq!(b.remaining(now), 0);
        assert_eq!(b.remaining(t("2026-10-22T10:01:00Z")), 40);
    }

    #[test]
    fn slack_ladder_steps() {
        let now = t("2026-10-22T10:00:00Z");
        assert_eq!(slack_ladder(30, Some(now - Duration::minutes(5)), now), 30);
        assert_eq!(
            slack_ladder(30, Some(now - Duration::minutes(30)), now),
            120
        );
        assert_eq!(slack_ladder(30, Some(now - Duration::hours(5)), now), 600);
        assert_eq!(slack_ladder(30, Some(now - Duration::days(3)), now), 1800);
        assert_eq!(slack_ladder(30, None, now), 1800);
    }

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(backoff_delay(1, 1.0), Duration::seconds(1));
        assert_eq!(backoff_delay(5, 1.0), Duration::seconds(16));
        assert_eq!(backoff_delay(9, 1.0), Duration::seconds(16));
        assert_eq!(backoff_delay(3, 0.0), Duration::ZERO);
    }
}
