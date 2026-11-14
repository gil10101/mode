//! The OS signals the connectors service and the alert pipeline read
//! (06 §2.3, 07 §8.4–§8.5, 08 §11):
//! - reachability every 30 s: resolve and TCP-connect the next hop of the
//!   connectors' HTTP path (the configured proxy, else the providers' API
//!   hosts), never a Mode server; two failed checks in a row → offline, one
//!   success → online (`Connectors::set_offline`, which pushes
//!   `health.changed` / `accounts.changed` and catches up on return);
//! - OS resume → `Connectors::wake` and the scheduler's `Wake::Resumed`
//!   (its catch-up window): a jump of the wall clock past the
//!   monotonic clock between two polls (a suspended machine: Linux and
//!   macOS monotonic clocks stop during sleep), a poll that fired far too
//!   late, or Windows' `WM_POWERBROADCAST` resume seen by the overlay's
//!   window subclass;
//! - every 30 s and after a resume: presentation, Focus Assist (Windows
//!   `SHQueryUserNotificationState`) and the locked screen (Windows input
//!   desktop) into `Connectors::set_os_state` (which pushes `quiet.changed`
//!   when anything changed); macOS Focus is read by the core itself;
//! - the app window's focus → `Connectors::set_foreground` (06 §2.3).

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tauri::WindowEvent;

use crate::platform::OverlayPlatform;
use crate::state::CoreState;

/// Reachability poll cadence.
pub const NETWORK_EVERY: Duration = Duration::from_secs(30);
/// OS state (Focus, lock, presentation) poll cadence (07 §8.5.2).
pub const OS_STATE_EVERY: Duration = Duration::from_secs(30);
/// One TCP connect attempt.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// The whole check (DNS included) before it counts as a failure.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// Consecutive failed checks before the app reports offline.
pub const OFFLINE_AFTER: u32 = 2;
/// The providers' API hosts (06 §6, §7, §8): the path the pollers take.
pub const PROVIDER_HOSTS: &[(&str, u16)] = &[("www.googleapis.com", 443), ("slack.com", 443)];

/// Offline after [`OFFLINE_AFTER`] failed checks, online after one success.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OfflineTracker {
    failures: u32,
    offline: bool,
}

impl OfflineTracker {
    /// Feed one check; `Some(offline)` when the state flips.
    pub fn observe(&mut self, reachable: bool) -> Option<bool> {
        if reachable {
            self.failures = 0;
            if self.offline {
                self.offline = false;
                return Some(false);
            }
            return None;
        }
        self.failures = self.failures.saturating_add(1);
        if !self.offline && self.failures >= OFFLINE_AFTER {
            self.offline = true;
            return Some(true);
        }
        None
    }

    pub fn is_offline(&self) -> bool {
        self.offline
    }
}

/// Detects a suspended machine between two polls.
#[derive(Debug, Clone)]
pub struct WakeDetector {
    mono: Instant,
    wall: SystemTime,
}

impl WakeDetector {
    pub fn new(mono: Instant, wall: SystemTime) -> WakeDetector {
        WakeDetector { mono, wall }
    }

    /// `true` when the time since the last tick shows the machine slept:
    /// the wall clock moved a minute more than the monotonic clock, or the
    /// tick came more than a minute later than `interval`.
    pub fn tick(&mut self, mono: Instant, wall: SystemTime, interval: Duration) -> bool {
        let dm = mono.saturating_duration_since(self.mono);
        let dw = wall.duration_since(self.wall).unwrap_or(Duration::ZERO);
        self.mono = mono;
        self.wall = wall;
        let slack = Duration::from_secs(60);
        dw > dm + slack || dm > interval + slack
    }
}

/// `host:port` of a proxy URL (`http://proxy:3128`, `socks5://h:1080`, `h:8080`).
fn proxy_host(v: &str) -> Option<(String, u16)> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    let (scheme, rest) = match v.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => ("http".to_string(), v),
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let (host, port) = if let Some(stripped) = hostport.strip_prefix('[') {
        let (h, tail) = stripped.split_once(']')?;
        (
            h.to_string(),
            tail.strip_prefix(':').and_then(|p| p.parse().ok()),
        )
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()),
            None => (hostport.to_string(), None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let default = match scheme.as_str() {
        "https" => 443,
        s if s.starts_with("socks") => 1080,
        _ => 80,
    };
    Some((host, port.unwrap_or(default)))
}

/// The next hop of the connectors' HTTP path: the proxy the HTTP client
/// would use (`HTTPS_PROXY`, `ALL_PROXY`, either case), else the providers.
pub fn next_hops(env: impl Fn(&str) -> Option<String>) -> Vec<(String, u16)> {
    for k in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        if let Some(h) = env(k).as_deref().and_then(proxy_host) {
            return vec![h];
        }
    }
    PROVIDER_HOSTS
        .iter()
        .map(|(h, p)| ((*h).to_string(), *p))
        .collect()
}

/// Resolve and TCP-connect any of `hops` (blocking).
pub fn reachable(hops: &[(String, u16)]) -> bool {
    hops.iter().any(|(h, p)| {
        (h.as_str(), *p)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .is_some_and(|addr| TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok())
    })
}

async fn check_once() -> bool {
    let hops = next_hops(|k| std::env::var(k).ok());
    let job = tauri::async_runtime::spawn_blocking(move || reachable(&hops));
    matches!(tokio::time::timeout(CHECK_TIMEOUT, job).await, Ok(Ok(true)))
}

/// Read the OS facts the platform glue can see and hand them to the
/// connectors service (only what is known; unknown keeps the last value).
pub fn feed_os_state(core: &CoreState, platform: &dyn OverlayPlatform) {
    let Some(c) = core.connectors.get() else {
        return;
    };
    let mut os = c.os_state();
    if let Some(f) = platform.focus() {
        os.focus = Some(f);
    }
    if let Some(l) = platform.screen_locked() {
        os.screen_locked = l;
    }
    if let Some(p) = platform.presentation() {
        os.presentation = p;
    }
    c.set_os_state(os);
}

/// Reachability, resume detection and the OS state, for the life of the app.
pub async fn signals_task(core: Arc<CoreState>, platform: Arc<dyn OverlayPlatform>) {
    let mut tracker = OfflineTracker::default();
    let mut wake = WakeDetector::new(Instant::now(), SystemTime::now());
    let mut last_os = Instant::now() - OS_STATE_EVERY;
    loop {
        let ok = check_once().await;
        if let Some(offline) = tracker.observe(ok) {
            tracing::info!(target: "mode::connectors", offline, "network reachability changed");
            if let Some(c) = core.connectors.get() {
                c.set_offline(offline);
            }
        }
        if last_os.elapsed() >= OS_STATE_EVERY {
            last_os = Instant::now();
            feed_os_state(&core, platform.as_ref());
        }
        tokio::time::sleep(NETWORK_EVERY).await;
        let resumed_hook = platform.take_resumed();
        if wake.tick(Instant::now(), SystemTime::now(), NETWORK_EVERY) || resumed_hook {
            tracing::info!(target: "mode::connectors", hook = resumed_hook, "OS resume: catch-up poll");
            if let Some(c) = core.connectors.get() {
                c.wake();
            }
            core.wake_scheduler(mode_core::scheduler::Wake::Resumed);
            last_os = Instant::now() - OS_STATE_EVERY;
        }
    }
}

/// The app window's events: focus drives the poll scheduler's foreground boost.
pub fn on_app_window_event(core: &CoreState, event: &WindowEvent) {
    if let WindowEvent::Focused(focused) = event
        && let Some(c) = core.connectors.get()
    {
        c.set_foreground(*focused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_needs_two_failures_and_one_success_ends_it() {
        let mut t = OfflineTracker::default();
        assert_eq!(t.observe(true), None);
        assert_eq!(t.observe(false), None, "one failure is a blip");
        assert_eq!(t.observe(true), None);
        assert_eq!(t.observe(false), None);
        assert_eq!(t.observe(false), Some(true));
        assert!(t.is_offline());
        assert_eq!(t.observe(false), None, "reported once");
        assert_eq!(t.observe(true), Some(false));
        assert!(!t.is_offline());
    }

    #[test]
    fn a_wall_clock_jump_or_a_late_tick_means_the_machine_slept() {
        let m0 = Instant::now();
        let w0 = SystemTime::now();
        let i = NETWORK_EVERY;
        let mut d = WakeDetector::new(m0, w0);
        assert!(!d.tick(m0 + i, w0 + i, i), "an ordinary tick");
        assert!(
            d.tick(m0 + i * 2, w0 + i * 2 + Duration::from_secs(3600), i),
            "an hour of wall time in 30 s of monotonic time"
        );
        assert!(
            d.tick(
                m0 + i * 2 + Duration::from_secs(600),
                w0 + Duration::from_secs(4300),
                i
            ),
            "the poll fired ten minutes late"
        );
        assert!(!d.tick(
            m0 + i * 3 + Duration::from_secs(600),
            w0 + Duration::from_secs(4330),
            i
        ));
        assert!(
            !d.tick(m0 + i * 4 + Duration::from_secs(600), w0, i),
            "the wall clock set back is no resume"
        );
    }

    #[test]
    fn the_next_hop_is_the_proxy_when_one_is_set() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_string())
            }
        };
        assert_eq!(
            next_hops(env(&[("HTTPS_PROXY", "http://user:pw@proxy.local:3128/")])),
            vec![("proxy.local".to_string(), 3128)]
        );
        assert_eq!(
            next_hops(env(&[("all_proxy", "socks5://[::1]")])),
            vec![("::1".to_string(), 1080)]
        );
        assert_eq!(
            next_hops(env(&[("https_proxy", "proxy:8080")])),
            vec![("proxy".to_string(), 8080)]
        );
        let direct = next_hops(env(&[("HTTPS_PROXY", "  ")]));
        assert_eq!(direct.len(), PROVIDER_HOSTS.len());
        assert_eq!(direct[0], ("www.googleapis.com".to_string(), 443));
        assert!(!reachable(&[]), "nothing to reach");
    }
}
