//! The health model (06 §2.4): the stored `accounts.status` machine
//! (`active`, `paused`, `needs_reauth`, `revoked`, `error`) with its
//! transitions, the derived `health()` per account (`ok`, `degraded`,
//! `rate_limited`, `needs_reauth`, `revoked`, `offline`, `error`) with the
//! Diagnostics line, the Integrations card aggregation (worst state wins,
//! §9A), and the `health.changed` fields `offline`, `paused_accounts`,
//! `broken_accounts` the app merges into its push.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::ConnectorError;
use crate::db::{Result, format_timestamp, parse_timestamp};
use crate::repo::{self, Account};

/// Five consecutive non-auth failures → `error` (06 §2.4).
pub const ERROR_AFTER: u32 = 5;

/// `accounts.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Paused,
    NeedsReauth,
    Revoked,
    Error,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Paused => "paused",
            Status::NeedsReauth => "needs_reauth",
            Status::Revoked => "revoked",
            Status::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "active" => Status::Active,
            "paused" => Status::Paused,
            "needs_reauth" => Status::NeedsReauth,
            "revoked" => Status::Revoked,
            "error" => Status::Error,
            _ => return None,
        })
    }
}

/// What can happen to an account (the arrows of 06 §2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    UserPause,
    UserResume,
    /// Refresh failed with `invalid_grant`, a scope went missing, Testing expiry.
    AuthFailed(String),
    /// Provider reports the token revoked / `account_inactive`.
    Revoked(String),
    /// The n-th consecutive non-auth failure.
    Failure {
        consecutive: u32,
        detail: String,
    },
    PollSucceeded,
    UserRetry,
    Reauthorised,
}

/// The next status, or `None` when the arrow does not exist.
pub fn next(cur: Status, t: &Transition) -> Option<Status> {
    use Status::*;
    match (cur, t) {
        (Active, Transition::UserPause) => Some(Paused),
        (Paused, Transition::UserResume) => Some(Active),
        (Active, Transition::AuthFailed(_)) => Some(NeedsReauth),
        (Active | Error, Transition::Revoked(_)) => Some(Revoked),
        (NeedsReauth | Revoked | Error | Paused | Active, Transition::Reauthorised) => Some(Active),
        (Active, Transition::Failure { consecutive, .. }) if *consecutive >= ERROR_AFTER => {
            Some(Error)
        }
        (Error, Transition::PollSucceeded | Transition::UserRetry) => Some(Active),
        (Error, Transition::AuthFailed(_)) => Some(NeedsReauth),
        _ => None,
    }
}

/// Apply a transition to the row; returns the new status when it changed.
pub fn transition(
    conn: &Connection,
    account_id: &str,
    t: &Transition,
    now: OffsetDateTime,
) -> Result<Option<Status>> {
    let Some(a) = repo::get_by_id::<Account>(conn, account_id)? else {
        return Ok(None);
    };
    let cur = Status::parse(&a.status).unwrap_or(Status::Active);
    let now_s = format_timestamp(now);
    let detail = match t {
        Transition::AuthFailed(d) | Transition::Revoked(d) => Some(d.clone()),
        Transition::Failure { detail, .. } => Some(detail.clone()),
        _ => None,
    };
    match next(cur, t) {
        Some(n) => {
            let clear = matches!(
                t,
                Transition::PollSucceeded
                    | Transition::UserRetry
                    | Transition::Reauthorised
                    | Transition::UserResume
            );
            conn.execute(
                "UPDATE accounts SET status = ?2, status_detail = CASE WHEN ?4 THEN NULL ELSE COALESCE(?3, status_detail) END,
                        last_error_at = CASE WHEN ?3 IS NOT NULL THEN ?5 ELSE last_error_at END, updated_at = ?5
                  WHERE id = ?1",
                params![account_id, n.as_str(), detail, clear, now_s],
            )?;
            Ok((n != cur).then_some(n))
        }
        None => {
            if let Some(d) = detail {
                conn.execute(
                    "UPDATE accounts SET status_detail = ?2, last_error_at = ?3, updated_at = ?3 WHERE id = ?1",
                    params![account_id, d, now_s],
                )?;
            }
            Ok(None)
        }
    }
}

/// Map a sync error onto a transition (06 §2.1 "Errors" column).
pub fn transition_for(err: &ConnectorError, consecutive: u32) -> Option<Transition> {
    match err {
        ConnectorError::InvalidGrant(d) => Some(Transition::AuthFailed(d.clone())),
        ConnectorError::Unauthorized => Some(Transition::AuthFailed(
            "the provider rejected the token".into(),
        )),
        ConnectorError::Revoked(d) => Some(Transition::Revoked(d.clone())),
        e if e.counts_as_failure() => Some(Transition::Failure {
            consecutive,
            detail: crate::redact::scrub(&e.to_string()).into_owned(),
        }),
        _ => None,
    }
}

/// Record a successful poll (`last_sync_at`; `error` → `active`).
pub fn record_success(conn: &Connection, account_id: &str, now: OffsetDateTime) -> Result<()> {
    conn.execute(
        "UPDATE accounts SET last_sync_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![account_id, format_timestamp(now)],
    )?;
    transition(conn, account_id, &Transition::PollSucceeded, now)?;
    Ok(())
}

// ------------------------------------------------------------ derived

/// `health()` (06 §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    Degraded,
    RateLimited,
    Offline,
    Error,
    NeedsReauth,
    Revoked,
    Paused,
}

/// In-memory facts the scheduler knows about an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runtime {
    pub offline: bool,
    pub rate_limited_until: Option<String>,
    pub last_poll_failed: bool,
    pub previous_poll_failed: bool,
    pub writeback_retrying: Option<(i64, i64)>,
    pub next_poll_in_secs: Option<i64>,
    pub budget_used: i64,
    pub budget_cap: i64,
    pub interval_secs: i64,
    pub provider_label: String,
}

/// One account's health with its Diagnostics line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub account_id: String,
    pub display_name: String,
    pub provider: String,
    pub status: String,
    pub health: Health,
    pub line: String,
    pub last_sync_at: Option<String>,
    pub budget_used: i64,
    pub budget_cap: i64,
}

fn ago(now: OffsetDateTime, at: &str) -> String {
    match parse_timestamp(at) {
        Some(t) => {
            let s = (now - t).whole_seconds().max(0);
            if s < 60 {
                format!("{s} s ago")
            } else if s < 3600 {
                format!("{} m ago", s / 60)
            } else if s < 86_400 {
                format!("{} h ago", s / 3600)
            } else {
                format!("{} d ago", s / 86_400)
            }
        }
        None => "never".into(),
    }
}

/// Derive `health()` and the Diagnostics line.
pub fn report(a: &Account, rt: &Runtime, now: OffsetDateTime) -> HealthReport {
    let status = Status::parse(&a.status).unwrap_or(Status::Active);
    let (health, line) = match status {
        Status::Paused => (Health::Paused, "paused".to_string()),
        Status::NeedsReauth => (Health::NeedsReauth, "re-authorize".to_string()),
        Status::Revoked => (Health::Revoked, "access revoked · re-authorize".to_string()),
        Status::Error => (
            Health::Error,
            a.status_detail.clone().unwrap_or_else(|| "error".into()),
        ),
        Status::Active if rt.offline => (Health::Offline, "offline · pollers paused".to_string()),
        Status::Active => {
            if let Some(until) = rt.rate_limited_until.as_deref().and_then(parse_timestamp)
                && until > now
            {
                (
                    Health::RateLimited,
                    format!(
                        "rate limited by {} · resumes in {} s",
                        rt.provider_label,
                        (until - now).whole_seconds()
                    ),
                )
            } else if let Some((n, of)) = rt.writeback_retrying {
                (
                    Health::Degraded,
                    format!("retrying write-back ({n} of {of})"),
                )
            } else if rt.last_poll_failed || rt.previous_poll_failed {
                (Health::Degraded, "last poll failed · retrying".to_string())
            } else {
                let fresh = a
                    .last_sync_at
                    .as_deref()
                    .and_then(parse_timestamp)
                    .is_some_and(|t| (now - t).whole_seconds() <= 2 * rt.interval_secs.max(1));
                let mut line = format!(
                    "last sync {}",
                    a.last_sync_at
                        .as_deref()
                        .map(|t| ago(now, t))
                        .unwrap_or_else(|| "pending".into())
                );
                if let Some(n) = rt.next_poll_in_secs {
                    line.push_str(&format!(" · next in {} s", n.max(0)));
                }
                if rt.budget_cap > 0 {
                    line.push_str(&format!(" · budget {}/{}", rt.budget_used, rt.budget_cap));
                }
                (
                    if fresh || a.last_sync_at.is_none() {
                        Health::Ok
                    } else {
                        Health::Degraded
                    },
                    line,
                )
            }
        }
    };
    HealthReport {
        account_id: a.id.clone(),
        display_name: a.display_name.clone(),
        provider: a.provider.clone(),
        status: a.status.clone(),
        health,
        line,
        last_sync_at: a.last_sync_at.clone(),
        budget_used: rt.budget_used,
        budget_cap: rt.budget_cap,
    }
}

/// The Integrations card status line (06 §9A): worst state wins, newest sync shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardStatus {
    /// `connected` | `not_connected` | `needs_sign_in` | `error` | `offline` | `rate_limited`.
    pub state: String,
    pub glyph: String,
    pub line: String,
    pub accounts: usize,
}

pub fn card_status(reports: &[HealthReport], now: OffsetDateTime) -> CardStatus {
    if reports.is_empty() {
        return CardStatus {
            state: "not_connected".into(),
            glyph: "○".into(),
            line: "not connected".into(),
            accounts: 0,
        };
    }
    let worst = reports.iter().map(|r| r.health).max().unwrap_or(Health::Ok);
    let newest = reports
        .iter()
        .filter_map(|r| r.last_sync_at.as_deref())
        .max()
        .map(|t| ago(now, t));
    let n = reports.len();
    let accounts = if n == 1 {
        "1 account".to_string()
    } else {
        format!("{n} accounts")
    };
    let (state, glyph, line) = match worst {
        Health::NeedsReauth | Health::Revoked => {
            ("needs_sign_in", "✕", "needs sign-in".to_string())
        }
        Health::Error => (
            "error",
            "✕",
            reports
                .iter()
                .find(|r| r.health == Health::Error)
                .map(|r| r.line.clone())
                .unwrap_or_default(),
        ),
        Health::Offline => ("offline", "◉", "offline".to_string()),
        Health::RateLimited => ("rate_limited", "◉", format!("{accounts} · rate limited")),
        _ => (
            "connected",
            "◉",
            match newest {
                Some(a) => format!("{accounts} · synced {a}"),
                None => accounts,
            },
        ),
    };
    CardStatus {
        state: state.into(),
        glyph: glyph.into(),
        line,
        accounts: n,
    }
}

/// The `health.changed` fields this package owns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthFields {
    pub offline: bool,
    pub paused_accounts: Vec<String>,
    /// `needs_reauth` / `revoked` / `error`, by label.
    pub broken_accounts: Vec<String>,
}

pub fn health_fields(conn: &Connection, offline: bool) -> Result<HealthFields> {
    let accounts = repo::query::<Account>(conn, "1 ORDER BY position, display_name", &[])?;
    Ok(HealthFields {
        offline,
        paused_accounts: accounts
            .iter()
            .filter(|a| a.status == "paused")
            .map(|a| a.display_name.clone())
            .collect(),
        broken_accounts: accounts
            .iter()
            .filter(|a| matches!(a.status.as_str(), "needs_reauth" | "revoked" | "error"))
            .map(|a| a.display_name.clone())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::*;

    #[test]
    fn the_state_diagram_of_06_2_4() {
        assert_eq!(next(Active, &Transition::UserPause), Some(Paused));
        assert_eq!(next(Paused, &Transition::UserResume), Some(Active));
        assert_eq!(
            next(Active, &Transition::AuthFailed("x".into())),
            Some(NeedsReauth)
        );
        assert_eq!(next(NeedsReauth, &Transition::Reauthorised), Some(Active));
        assert_eq!(
            next(Active, &Transition::Revoked("x".into())),
            Some(Revoked)
        );
        assert_eq!(next(Revoked, &Transition::Reauthorised), Some(Active));
        let f = |n| Transition::Failure {
            consecutive: n,
            detail: "5xx".into(),
        };
        assert_eq!(next(Active, &f(4)), None);
        assert_eq!(next(Active, &f(5)), Some(Error));
        assert_eq!(next(Error, &Transition::PollSucceeded), Some(Active));
        assert_eq!(next(Error, &Transition::UserRetry), Some(Active));
        assert_eq!(next(Paused, &Transition::PollSucceeded), None);
        assert_eq!(next(NeedsReauth, &Transition::UserResume), None);
    }
}
