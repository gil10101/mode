//! What every notes operation needs besides a connection: the vault root,
//! the device id, the clock (overridable for tests), the zone, the
//! `notes.*` settings it reads, and the trash. [`Env`] is cheap to clone
//! into a writer job; [`Cx`] is built inside the job around its connection.

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::Connection;
use time::Date;

use super::fields::{self, FixedTz, LocalTz, SqliteTz, WeekStart};
use super::fsio::{OsTrash, Trash};
use crate::db::{DbError, Result};
use crate::queries::Clock;
use crate::settings::{self, StatusDef};

/// Which zone local dates follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TzMode {
    /// The OS zone (SQLite `localtime`).
    #[default]
    System,
    /// A fixed offset in minutes east of UTC (tests).
    Fixed(i32),
}

/// Owned, `Send` context for a job.
#[derive(Clone)]
pub struct Env {
    pub root: PathBuf,
    pub device_id: String,
    /// A fixed clock (tests); `None` = now.
    pub clock: Option<Clock>,
    pub tz: TzMode,
    pub trash: Arc<dyn Trash>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("root", &self.root)
            .field("device_id", &self.device_id)
            .field("clock", &self.clock)
            .field("tz", &self.tz)
            .finish_non_exhaustive()
    }
}

impl Env {
    pub fn new(root: PathBuf, device_id: impl Into<String>) -> Env {
        Env {
            root,
            device_id: device_id.into(),
            clock: None,
            tz: TzMode::System,
            trash: Arc::new(OsTrash),
        }
    }
}

enum AnyTz<'c> {
    Sql(SqliteTz<'c>),
    Fixed(FixedTz),
}

/// A job's context around its connection.
pub struct Cx<'c> {
    pub conn: &'c Connection,
    pub env: &'c Env,
    pub clock: Clock,
    pub week_start: WeekStart,
    tz: AnyTz<'c>,
    statuses: std::cell::OnceCell<Vec<StatusDef>>,
}

impl<'c> Cx<'c> {
    pub fn new(conn: &'c Connection, env: &'c Env) -> Result<Cx<'c>> {
        let clock = match &env.clock {
            Some(c) => c.clone(),
            None => Clock::current(conn)?,
        };
        let week_start = settings::get_string_in(conn, "notes.week_start")?
            .map(|s| WeekStart::parse(&s))
            .unwrap_or_default();
        let tz = match env.tz {
            TzMode::System => AnyTz::Sql(SqliteTz(conn)),
            TzMode::Fixed(m) => AnyTz::Fixed(FixedTz(m)),
        };
        Ok(Cx {
            conn,
            env,
            clock,
            week_start,
            tz,
            statuses: std::cell::OnceCell::new(),
        })
    }

    pub fn tz(&self) -> &dyn LocalTz {
        match &self.tz {
            AnyTz::Sql(t) => t,
            AnyTz::Fixed(t) => t,
        }
    }

    pub fn device(&self) -> &str {
        &self.env.device_id
    }

    pub fn now(&self) -> &str {
        &self.clock.now
    }

    pub fn today(&self) -> Date {
        fields::parse_date(&self.clock.today).unwrap_or(Date::MIN)
    }

    /// `settings['notes.statuses']`.
    pub fn statuses(&self) -> &[StatusDef] {
        self.statuses.get_or_init(|| {
            settings::statuses_in(self.conn).unwrap_or_else(|_| {
                serde_json::from_value(settings::default_for("notes.statuses")).unwrap_or_default()
            })
        })
    }

    pub fn closed_statuses(&self) -> Vec<String> {
        let v: Vec<String> = self
            .statuses()
            .iter()
            .filter(|s| s.closed)
            .map(|s| s.slug.clone())
            .collect();
        if v.is_empty() { vec!["done".into()] } else { v }
    }

    pub fn is_closed(&self, status: &str) -> bool {
        self.closed_statuses().iter().any(|s| s == status)
    }

    pub fn status_known(&self, status: &str) -> bool {
        self.statuses().iter().any(|s| s.slug == status)
    }

    /// The compiler's context for this clock.
    pub fn filter_context(&self) -> crate::queries::FilterContext {
        self.clock.filter_context(self.closed_statuses())
    }
}

/// `DbError::Invalid` with a message for the user.
pub fn invalid(what: &'static str, detail: impl Into<String>) -> DbError {
    DbError::Invalid {
        what,
        detail: detail.into(),
    }
}
