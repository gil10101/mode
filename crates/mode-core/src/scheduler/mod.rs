//! Automations (03 §7B; 08 §3.2 `scheduler`; 09 §5.4A, §13 S8): schedule
//! triggers only in v1, three actions, run records, the one-minute tick and
//! the catch-up window.
//!
//! * [`schedule`]: `schedule_json` → a 5-field cron plan in an IANA zone
//!   (DST follows the zone), next fire times, trigger phrases and the
//!   plain-language echo.
//! * [`actions`]: `action_json` per action kind, validation, action phrases.
//! * [`store`]: list rows, run rows, saving drafts, pruning.
//! * [`engine`]: [`Scheduler`] (tick, catch-up, Run now, session-run
//!   monitoring with the token cap, `Scheduler::run(wake_rx)`), the
//!   [`Dispatch`] trait.
//! * [`dispatch`]: the app's dispatcher over the PTY host, notes and Chat.
//! * [`api`]: one function per command.

pub mod actions;
pub mod api;
pub mod dispatch;
pub mod engine;
pub mod schedule;
pub mod store;

pub use engine::{Dispatch, FailedRun, RunContext, Scheduler, SchedulerOptions, TickReport, Wake};
