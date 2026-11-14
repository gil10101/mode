//! The urgency engine (07): every message the connectors store becomes one
//! tier (`vip`, `direct`, `ai-urgent`, `ai-normal`, `low`; `unscored` while
//! queued) with a why line, through a fixed pipeline: ingest (§3.1) →
//! deterministic stage (§3.2) → optional keyword and deadline rules (§3.3)
//! → AI scoring of the remainder (§3.4, [`scoring`]) → ranking (§3.5,
//! [`rank`]) → routing (§3.6, [`route`]). Important people (§5), snooze (§6),
//! de-duplication (§7) and alerts (§8–§9) sit beside it.
//!
//! Deterministic tiers are decided inside the connector's ingest
//! transaction ([`process_new`]); AI tiers arrive in a later batch
//! ([`apply_scores`]). Invariants of §2.2 are enforced here: a `vip` is
//! never lowered except by the user (I2), deterministic tiers never reach a
//! backend (I3), `vip` / `direct` / `ai-urgent` always carry a why (I5),
//! user overrides are sticky (I6).

pub mod alerts;
pub mod dedup;
pub mod rank;
pub mod route;
pub mod rules;
pub mod scoring;
pub mod snooze;
pub mod vips;
pub mod why;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::db::{Result, format_timestamp, parse_timestamp};
use crate::notes::ctx::TzMode;
use crate::notes::fields::{FixedTz, LocalTz, SqliteTz};
use alerts::{AlertEvent, MessageReceived};

/// The five tiers plus the transient `unscored` (07 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    #[serde(rename = "vip")]
    Vip,
    #[serde(rename = "direct")]
    Direct,
    #[serde(rename = "ai-urgent")]
    AiUrgent,
    #[serde(rename = "ai-normal")]
    AiNormal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "unscored")]
    Unscored,
}

impl Tier {
    pub const ALL: [Tier; 6] = [
        Tier::Vip,
        Tier::Direct,
        Tier::AiUrgent,
        Tier::AiNormal,
        Tier::Low,
        Tier::Unscored,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Vip => "vip",
            Tier::Direct => "direct",
            Tier::AiUrgent => "ai-urgent",
            Tier::AiNormal => "ai-normal",
            Tier::Low => "low",
            Tier::Unscored => "unscored",
        }
    }

    pub fn parse(s: &str) -> Option<Tier> {
        Tier::ALL.iter().copied().find(|t| t.as_str() == s)
    }

    /// Ranking (07 §3.5): `vip` > `direct` > `ai-urgent` > `ai-normal` =
    /// `unscored` > `low`; lower sorts first.
    pub fn rank(self) -> u8 {
        match self {
            Tier::Vip => 0,
            Tier::Direct => 1,
            Tier::AiUrgent => 2,
            Tier::AiNormal | Tier::Unscored => 3,
            Tier::Low => 4,
        }
    }

    /// The three UI levels (ADR-0032).
    pub fn level(self) -> &'static str {
        match self {
            Tier::Vip | Tier::Direct | Tier::AiUrgent => "urgent",
            Tier::Low => "low",
            Tier::AiNormal | Tier::Unscored => "normal",
        }
    }

    pub fn is_urgent(self) -> bool {
        self.level() == "urgent"
    }
}

/// `urgency_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TierSource {
    Rule,
    Ai,
    User,
}

impl TierSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TierSource::Rule => "rule",
            TierSource::Ai => "ai",
            TierSource::User => "user",
        }
    }
}

/// What a backfill is (07 §3.1): received more than 48 h before it was fetched.
pub const BACKFILL_HOURS: i64 = 48;

/// Everything the pipeline reads besides the rows: the clock, the zone and
/// the `urgency.*` / `notifications.*` / `ai.scoring_chars` settings.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub now: OffsetDateTime,
    pub tz: TzMode,
    pub keyword_rules: Vec<rules::KeywordRule>,
    pub direct_email_requires_known_contact: bool,
    pub max_alert_age_minutes: i64,
    pub pill_message_max_age_hours: i64,
    pub snooze_wake_on_reply: bool,
    pub scoring_chars: i64,
}

impl Ctx {
    pub fn load(conn: &Connection, now: OffsetDateTime, tz: TzMode) -> Result<Ctx> {
        use crate::settings::{get_as_in, get_bool_in, get_i64_in};
        Ok(Ctx {
            now,
            tz,
            keyword_rules: get_as_in::<Vec<rules::KeywordRule>>(conn, "urgency.keyword_rules")
                .unwrap_or_default(),
            direct_email_requires_known_contact: get_bool_in(
                conn,
                "urgency.direct_email_requires_known_contact",
            )?
            .unwrap_or(true),
            max_alert_age_minutes: get_i64_in(conn, "notifications.max_alert_age_minutes")?
                .unwrap_or(30),
            pill_message_max_age_hours: get_i64_in(conn, "urgency.pill_message_max_age_hours")?
                .unwrap_or(24),
            snooze_wake_on_reply: get_bool_in(conn, "notifications.snooze_wake_on_reply")?
                .unwrap_or(true),
            scoring_chars: get_i64_in(conn, "ai.scoring_chars")?
                .filter(|n| *n > 0)
                .unwrap_or(500),
        })
    }

    pub fn now_str(&self) -> String {
        format_timestamp(self.now)
    }
}

/// Run `f` with the context's zone (SQLite `localtime` or a fixed offset).
pub fn with_tz<T>(conn: &Connection, tz: TzMode, f: impl FnOnce(&dyn LocalTz) -> T) -> T {
    match tz {
        TzMode::System => f(&SqliteTz(conn)),
        TzMode::Fixed(m) => f(&FixedTz(m)),
    }
}

/// One message with the joined inputs of §3.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub account_id: String,
    pub provider: String,
    pub kind: String,
    pub thread_id: Option<String>,
    pub source_id: Option<String>,
    pub sender_contact_id: Option<String>,
    pub sender_name: String,
    pub sender_address: String,
    pub subject: Option<String>,
    pub snippet: String,
    pub channel_name: Option<String>,
    pub labels: Vec<String>,
    pub is_direct: bool,
    pub is_read: bool,
    pub has_attachments: bool,
    pub urgency_tier: String,
    pub urgency_source: Option<String>,
    pub dedup_key: String,
    pub received_at: String,
    pub fetched_at: String,
    pub account_email: Option<String>,
    pub account_handle: Option<String>,
    pub contact_vip: bool,
    pub contact_vip_note: Option<String>,
    pub contact_display_name: Option<String>,
    pub contact_message_count: i64,
    pub contact_rev: i64,
    pub source_vip: bool,
    pub source_name: Option<String>,
    /// Messages from the same contact received before this one.
    pub contact_earlier: i64,
    /// Messages from the same contact stored now (fewer than
    /// `message_count` once retention purged some).
    pub contact_stored: i64,
}

const CANDIDATE_SQL: &str = "
SELECT m.id, m.account_id, m.provider, m.kind, m.thread_id, m.source_id, m.sender_contact_id,
       m.sender_name, m.sender_address, m.subject, m.snippet, m.channel_name, m.labels_json,
       m.is_direct, m.is_read, m.has_attachments, m.urgency_tier, m.urgency_source, m.dedup_key,
       m.received_at, m.fetched_at, a.email, a.handle,
       CASE WHEN c.deleted_at IS NULL THEN COALESCE(c.vip, 0) ELSE 0 END,
       c.vip_note, c.display_name, COALESCE(c.message_count, 0), COALESCE(c.rev, 0),
       COALESCE(s.vip, 0), s.name,
       (SELECT COUNT(*) FROM messages e WHERE e.sender_contact_id = m.sender_contact_id
           AND e.id <> m.id AND e.received_at < m.received_at),
       (SELECT COUNT(*) FROM messages e WHERE e.sender_contact_id = m.sender_contact_id)
  FROM messages m
  JOIN accounts a ON a.id = m.account_id
  LEFT JOIN contacts c ON c.id = m.sender_contact_id
  LEFT JOIN account_sources s ON s.id = m.source_id
 WHERE m.id = ?1";

/// Load one candidate.
pub fn candidate(conn: &Connection, id: &str) -> Result<Option<Candidate>> {
    Ok(conn
        .query_row(CANDIDATE_SQL, [id], |r| {
            let labels: String = r.get(12)?;
            Ok(Candidate {
                id: r.get(0)?,
                account_id: r.get(1)?,
                provider: r.get(2)?,
                kind: r.get(3)?,
                thread_id: r.get(4)?,
                source_id: r.get(5)?,
                sender_contact_id: r.get(6)?,
                sender_name: r.get(7)?,
                sender_address: r.get(8)?,
                subject: r.get(9)?,
                snippet: r.get(10)?,
                channel_name: r.get(11)?,
                labels: serde_json::from_str(&labels).unwrap_or_default(),
                is_direct: r.get(13)?,
                is_read: r.get(14)?,
                has_attachments: r.get(15)?,
                urgency_tier: r.get(16)?,
                urgency_source: r.get(17)?,
                dedup_key: r.get(18)?,
                received_at: r.get(19)?,
                fetched_at: r.get(20)?,
                account_email: r.get(21)?,
                account_handle: r.get(22)?,
                contact_vip: r.get(23)?,
                contact_vip_note: r.get(24)?,
                contact_display_name: r.get(25)?,
                contact_message_count: r.get(26)?,
                contact_rev: r.get(27)?,
                source_vip: r.get(28)?,
                source_name: r.get(29)?,
                contact_earlier: r.get(30)?,
                contact_stored: r.get(31)?,
            })
        })
        .optional()?)
}

impl Candidate {
    /// §3.2 rule 1: sent by the account's own user.
    pub fn from_self(&self) -> bool {
        let me = match self.provider.as_str() {
            "slack" => self.account_handle.as_deref(),
            _ => self.account_email.as_deref(),
        };
        me.is_some_and(|m| !m.is_empty() && m.eq_ignore_ascii_case(&self.sender_address))
    }

    /// Received more than 48 h before it was fetched (backfill queue, no alerts).
    pub fn is_backfill(&self) -> bool {
        match (
            parse_timestamp(&self.received_at),
            parse_timestamp(&self.fetched_at),
        ) {
            (Some(r), Some(f)) => (f - r).whole_hours() > BACKFILL_HOURS,
            _ => false,
        }
    }

    /// The §3.2 "known contact" definition: at least one earlier message
    /// from any of the contact's identities (stored, or counted before
    /// retention purged it), an important person, or a contact made or edited by hand.
    pub fn known_sender(&self) -> bool {
        self.contact_vip
            || self.contact_rev > 1
            || self.contact_earlier >= 1
            || (self.contact_message_count > 1 && self.contact_message_count > self.contact_stored)
    }

    /// The §3.2 "automated sender" definition.
    pub fn automated(&self) -> bool {
        is_automated(&self.labels, &self.sender_address)
    }

    pub fn has_label(&self, l: &str) -> bool {
        self.labels.iter().any(|x| x == l)
    }
}

/// `labels_json` carries `list` / `bulk` / `auto` / `bot`, or the local part
/// matches the no-reply pattern (07 §3.2).
pub fn is_automated(labels: &[String], address: &str) -> bool {
    if labels
        .iter()
        .any(|l| matches!(l.as_str(), "list" | "bulk" | "auto" | "bot"))
    {
        return true;
    }
    let local = address.split('@').next().unwrap_or("").to_ascii_lowercase();
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"^(no-?reply|do-?not-?reply|notifications?|mailer-daemon|alerts?|bounces?|noreply-.*|.*-noreply)$",
        )
        .expect("static regex")
    })
    .is_match(&local)
}

/// A decision of §3.2 / §3.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    pub tier: Tier,
    pub why: Option<String>,
    pub source: TierSource,
    /// §3.2 rule 1.
    pub from_self: bool,
}

/// The deterministic stage (07 §3.2), first rule wins.
pub fn deterministic(c: &Candidate, ctx: &Ctx) -> Option<Decided> {
    let rule = |tier, why: Option<String>| Decided {
        tier,
        why,
        source: TierSource::Rule,
        from_self: false,
    };
    if c.from_self() {
        return Some(Decided {
            from_self: true,
            ..rule(Tier::Low, None)
        });
    }
    if c.contact_vip {
        let name = c
            .contact_display_name
            .clone()
            .unwrap_or_else(|| c.sender_name.clone());
        return Some(rule(
            Tier::Vip,
            Some(why::vip_contact(&name, c.contact_vip_note.as_deref())),
        ));
    }
    if c.source_vip {
        let ch = c
            .channel_name
            .clone()
            .or_else(|| c.source_name.clone())
            .unwrap_or_default();
        return Some(rule(Tier::Vip, Some(why::vip_channel(&ch))));
    }
    if c.provider == "slack" && c.is_direct {
        let w = match c.kind.as_str() {
            "dm" => why::dm(&c.sender_name, false),
            "group_dm" => why::dm(&c.sender_name, true),
            _ => why::mention(c.channel_name.as_deref().unwrap_or("a channel")),
        };
        return Some(rule(Tier::Direct, Some(w)));
    }
    if c.provider == "google"
        && c.is_direct
        && !c.automated()
        && (c.known_sender() || !ctx.direct_email_requires_known_contact)
    {
        return Some(rule(Tier::Direct, Some(why::direct_email(&c.sender_name))));
    }
    None
}

/// What [`process_new`] did.
#[derive(Debug, Clone, Default)]
pub struct IngestOutcome {
    pub decided: Vec<(String, Tier)>,
    /// Still `unscored`: the scorer's queue.
    pub queued: Vec<String>,
    /// Copied from a twin (§7.2).
    pub twins: Vec<String>,
    pub alerts: Vec<AlertEvent>,
    /// Snoozes that woke on a reply (§6.3).
    pub woke: Vec<String>,
    pub thread_ids: Vec<String>,
}

fn write_tier(
    conn: &Connection,
    id: &str,
    tier: Tier,
    why: Option<&str>,
    source: Option<TierSource>,
    model: Option<&str>,
    now: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE messages SET urgency_tier = ?2, urgency_why = ?3, urgency_source = ?4,
                urgency_model = ?5, scored_at = ?6, updated_at = ?6 WHERE id = ?1",
        params![
            id,
            tier.as_str(),
            why,
            source.map(|s| s.as_str()),
            model,
            now
        ],
    )?;
    Ok(())
}

/// The `message.received` payload for a just-tiered message.
pub fn received_event(c: &Candidate, tier: Tier, why: Option<&str>) -> AlertEvent {
    AlertEvent::MessageReceived(MessageReceived {
        message_id: c.id.clone(),
        thread_id: c.thread_id.clone(),
        dedup_key: c.dedup_key.clone(),
        account_id: c.account_id.clone(),
        provider: c.provider.clone(),
        kind: c.kind.clone(),
        sender_contact_id: c.sender_contact_id.clone(),
        sender_name: c.sender_name.clone(),
        channel_name: c.channel_name.clone(),
        subject: c.subject.clone(),
        snippet: c.snippet.clone(),
        received_at: c.received_at.clone(),
        urgency_tier: tier.as_str().to_string(),
        urgency_why: why.map(str::to_string),
        resurfaced: false,
    })
}

/// §3.1–§3.3 for freshly inserted messages, inside the ingest transaction:
/// twin copy, deterministic tier, keyword rules, wake-on-reply, thread
/// recompute, and `message.received` for every tier assigned now (not from
/// self, not a backfill, not a twin after the first).
pub fn process_new(
    conn: &Connection,
    device_id: &str,
    ids: &[String],
    ctx: &Ctx,
) -> Result<IngestOutcome> {
    let now = ctx.now_str();
    let mut out = IngestOutcome::default();
    for id in ids {
        let Some(c) = candidate(conn, id)? else {
            continue;
        };
        if let Some(t) = &c.thread_id
            && !out.thread_ids.contains(t)
        {
            out.thread_ids.push(t.clone());
        }
        // Wake on reply (§6.3): an inbound message into a snoozed thread.
        if !c.from_self()
            && let Some(thread) = &c.thread_id
        {
            out.woke
                .extend(snooze::wake_on_reply(conn, device_id, thread, &c.id, &now)?);
        }
        if c.urgency_tier != Tier::Unscored.as_str() {
            continue; // a re-fetch keeps its tier (§3.1)
        }
        if let Some(t) = dedup::tiered_twin(conn, &c.id)? {
            conn.execute(
                "UPDATE messages SET urgency_tier = ?2, urgency_why = ?3, urgency_source = ?4,
                        urgency_model = ?5, scored_at = ?6, updated_at = ?7 WHERE id = ?1",
                params![c.id, t.tier, t.why, t.source, t.model, t.scored_at, now],
            )?;
            out.twins.push(c.id.clone());
            continue;
        }
        let decided = deterministic(&c, ctx).or_else(|| {
            let tz_ctx = ctx.clone();
            with_tz(conn, tz_ctx.tz, |tz| {
                rules::evaluate(&ctx.keyword_rules, &c, ctx.now, tz)
            })
            .map(|(tier, why)| Decided {
                tier,
                why: Some(why),
                source: TierSource::Rule,
                from_self: false,
            })
        });
        match decided {
            Some(d) => {
                write_tier(
                    conn,
                    &c.id,
                    d.tier,
                    d.why.as_deref(),
                    Some(d.source),
                    None,
                    &now,
                )?;
                if d.from_self && !c.is_read {
                    conn.execute(
                        "UPDATE messages SET is_read = 1, read_at = ?2, read_origin = 'provider'
                          WHERE id = ?1",
                        params![c.id, now],
                    )?;
                }
                out.decided.push((c.id.clone(), d.tier));
                if !d.from_self && !c.is_backfill() {
                    out.alerts
                        .push(received_event(&c, d.tier, d.why.as_deref()));
                }
            }
            None => out.queued.push(c.id.clone()),
        }
    }
    rank::recompute_threads(conn, &out.thread_ids, &now)?;
    Ok(out)
}

/// One AI verdict to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scored {
    pub message_id: String,
    pub tier: Tier,
    pub why: String,
    /// `<model id>+<prompt version>`.
    pub model: String,
}

/// Write a batch of AI verdicts (07 §3.4.4) in one transaction, copy them
/// to unscored twins, recompute thread tiers, and return `message.received`
/// for each (not for backfills). Rows that left `unscored` meanwhile (a
/// user override, an important-person change) are left alone.
pub fn apply_scores(conn: &Connection, scored: &[Scored], ctx: &Ctx) -> Result<Vec<AlertEvent>> {
    let now = ctx.now_str();
    let mut events = Vec::new();
    let mut threads = Vec::new();
    for s in scored {
        let Some(c) = candidate(conn, &s.message_id)? else {
            continue;
        };
        if c.urgency_tier != Tier::Unscored.as_str() {
            continue;
        }
        write_tier(
            conn,
            &c.id,
            s.tier,
            Some(&s.why),
            Some(TierSource::Ai),
            Some(&s.model),
            &now,
        )?;
        let twins = conn.execute(
            "UPDATE messages SET urgency_tier = ?2, urgency_why = ?3, urgency_source = 'ai',
                    urgency_model = ?4, scored_at = ?5, updated_at = ?5
              WHERE dedup_key = ?1 AND id <> ?6 AND urgency_tier = 'unscored'",
            params![c.dedup_key, s.tier.as_str(), s.why, s.model, now, c.id],
        )?;
        let _ = twins;
        if let Some(t) = &c.thread_id {
            threads.push(t.clone());
        }
        if !c.is_backfill() && !c.from_self() {
            events.push(received_event(&c, s.tier, Some(&s.why)));
        }
    }
    rank::recompute_threads(conn, &threads, &now)?;
    Ok(events)
}

/// Mark rows with a fallback (07 §3.4.6): `ai-normal`, `not scored (…)`,
/// `urgency_source = 'rule'`. No alert (an `ai-normal` fallback is never
/// urgent and the premade rule excludes it; the event is still returned so
/// a rule that selects `ai-normal` can see it).
pub fn apply_fallback(
    conn: &Connection,
    ids: &[String],
    why_text: &str,
    ctx: &Ctx,
) -> Result<Vec<AlertEvent>> {
    let now = ctx.now_str();
    let mut events = Vec::new();
    let mut threads = Vec::new();
    for id in ids {
        let Some(c) = candidate(conn, id)? else {
            continue;
        };
        if c.urgency_tier != Tier::Unscored.as_str() {
            continue;
        }
        write_tier(
            conn,
            id,
            Tier::AiNormal,
            Some(why_text),
            Some(TierSource::Rule),
            None,
            &now,
        )?;
        conn.execute(
            "UPDATE messages SET urgency_tier = 'ai-normal', urgency_why = ?2, urgency_source = 'rule',
                    scored_at = ?3, updated_at = ?3
              WHERE dedup_key = ?1 AND id <> ?4 AND urgency_tier = 'unscored'",
            params![c.dedup_key, why_text, now, id],
        )?;
        if let Some(t) = &c.thread_id {
            threads.push(t.clone());
        }
        if !c.is_backfill() && !c.from_self() {
            events.push(received_event(&c, Tier::AiNormal, Some(why_text)));
        }
    }
    rank::recompute_threads(conn, &threads, &now)?;
    Ok(events)
}

/// A user override from the badge menu (07 §3.5): `urgency_source = 'user'`,
/// `set by you`, copied to every twin, sticky. Never emits an alert.
pub fn set_override(conn: &Connection, message_id: &str, tier: Tier, now: &str) -> Result<usize> {
    if tier == Tier::Unscored {
        return Err(crate::db::DbError::Invalid {
            what: "tier",
            detail: "unscored is not a tier a person can choose".into(),
        });
    }
    let group = dedup::group_of(conn, message_id)?;
    let mut n = 0;
    let mut threads = Vec::new();
    for id in &group {
        n += conn.execute(
            "UPDATE messages SET urgency_tier = ?2, urgency_why = ?3, urgency_source = 'user',
                    urgency_model = NULL, scored_at = ?4, updated_at = ?4 WHERE id = ?1",
            params![id, tier.as_str(), why::SET_BY_YOU, now],
        )?;
        let t: Option<String> = conn
            .query_row("SELECT thread_id FROM messages WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        threads.extend(t);
    }
    rank::recompute_threads(conn, &threads, now)?;
    Ok(n)
}

/// Re-score (07 §3.4.8): re-queue the last 24 h of `ai` rows and `rule`
/// rows with a fallback why, plus `unscored`; never `user`, `vip`,
/// `direct`. Keyword-rule rows of the window are re-run through the
/// deterministic stage and rules first. Returns the rows re-queued or
/// re-decided. No alert ever fires for them (§8.1).
pub fn rescore(conn: &Connection, ctx: &Ctx) -> Result<usize> {
    let since = format_timestamp(ctx.now - time::Duration::hours(24));
    let ids: Vec<String> = {
        let mut st = conn.prepare(
            "SELECT id FROM messages
              WHERE received_at >= ?1
                AND urgency_tier NOT IN ('vip','direct')
                AND (urgency_source IS NULL OR urgency_source IN ('ai','rule'))",
        )?;
        let rows = st.query_map([&since], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let now = ctx.now_str();
    let mut n = 0;
    let mut threads = Vec::new();
    for id in ids {
        let Some(c) = candidate(conn, &id)? else {
            continue;
        };
        if c.from_self() {
            continue;
        }
        let decided = deterministic(&c, ctx).or_else(|| {
            with_tz(conn, ctx.tz, |tz| {
                rules::evaluate(&ctx.keyword_rules, &c, ctx.now, tz)
            })
            .map(|(tier, why)| Decided {
                tier,
                why: Some(why),
                source: TierSource::Rule,
                from_self: false,
            })
        });
        match decided {
            Some(d) => write_tier(
                conn,
                &id,
                d.tier,
                d.why.as_deref(),
                Some(d.source),
                None,
                &now,
            )?,
            None => {
                conn.execute(
                    "UPDATE messages SET urgency_tier = 'unscored', urgency_why = NULL,
                            urgency_source = NULL, urgency_model = NULL, scored_at = NULL,
                            updated_at = ?2 WHERE id = ?1",
                    params![id, now],
                )?;
            }
        }
        if let Some(t) = c.thread_id {
            threads.push(t);
        }
        n += 1;
    }
    rank::recompute_threads(conn, &threads, &now)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_round_trip_and_rank() {
        for t in Tier::ALL {
            assert_eq!(Tier::parse(t.as_str()), Some(t));
            assert_eq!(serde_json::to_value(t).unwrap(), t.as_str());
        }
        assert!(Tier::Vip.rank() < Tier::Direct.rank());
        assert!(Tier::Direct.rank() < Tier::AiUrgent.rank());
        assert_eq!(Tier::AiNormal.rank(), Tier::Unscored.rank());
        assert!(Tier::AiNormal.rank() < Tier::Low.rank());
        assert_eq!(Tier::Direct.level(), "urgent");
        assert_eq!(Tier::Unscored.level(), "normal");
    }

    #[test]
    fn automated_senders() {
        assert!(is_automated(&[], "no-reply@x.com"));
        assert!(is_automated(&[], "Notifications@github.com"));
        assert!(is_automated(&[], "billing-noreply@x.com"));
        assert!(is_automated(&["list".into()], "ana@x.com"));
        assert!(!is_automated(&[], "ana@x.com"));
        assert!(!is_automated(&[], "alertsteam@x.com"));
    }
}
