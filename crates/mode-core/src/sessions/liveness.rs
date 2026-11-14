//! Process liveness for external sessions (05 §9.1, §4.2 "Liveness probe"):
//! the relay reports `ppid` (the `claude` process), the engine keeps it in
//! memory per session, persists it with the machine's boot identity (0004) and
//! probes it every 15 s. The boot identity lets a restarted Mode tell a pid it
//! can still probe from one that belongs to a previous boot (05 §4.2 "Mode
//! starts").

use std::collections::HashMap;
use std::time::Duration;

/// How often the engine probes known pids (05 §4.2, assumption 8).
pub const PROBE_INTERVAL: Duration = Duration::from_secs(15);

/// Two boot identities within this many seconds are the same boot (the
/// Windows identity is a tick-derived boot time, not an id).
pub const BOOT_TOLERANCE_SECS: i64 = 60;

/// Whether a process with this pid exists (best effort).
pub fn pid_alive(pid: u32) -> bool {
    platform::pid_alive(pid)
}

/// An identity of the current machine boot: a boot UUID where the OS keeps
/// one (Linux, macOS), else `boot:<unix seconds>` derived from the uptime.
/// Never empty.
pub fn boot_id() -> String {
    platform::boot_id().unwrap_or_else(|| {
        boot_time()
            .map(|t| format!("boot:{}", t.unix_timestamp()))
            .unwrap_or_else(|| "boot:unknown".into())
    })
}

/// When the machine booted, if the OS says (used to close rows whose last
/// event predates this boot).
pub fn boot_time() -> Option<time::OffsetDateTime> {
    platform::boot_time()
}

/// Whether two boot identities name the same boot: equal ids, or tick-derived
/// times within [`BOOT_TOLERANCE_SECS`].
pub fn same_boot(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (
        a.strip_prefix("boot:").and_then(|n| n.parse::<i64>().ok()),
        b.strip_prefix("boot:").and_then(|n| n.parse::<i64>().ok()),
    ) {
        (Some(x), Some(y)) => (x - y).abs() <= BOOT_TOLERANCE_SECS,
        _ => false,
    }
}

#[cfg(unix)]
mod platform {
    pub fn boot_id() -> Option<String> {
        #[cfg(target_os = "linux")]
        {
            let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
            let id = id.trim();
            (!id.is_empty()).then(|| id.to_string())
        }
        #[cfg(target_os = "macos")]
        {
            sysctl_string("kern.bootsessionuuid")
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }

    pub fn boot_time() -> Option<time::OffsetDateTime> {
        #[cfg(target_os = "linux")]
        {
            let stat = std::fs::read_to_string("/proc/stat").ok()?;
            let secs: i64 = stat
                .lines()
                .find_map(|l| l.strip_prefix("btime "))?
                .trim()
                .parse()
                .ok()?;
            time::OffsetDateTime::from_unix_timestamp(secs).ok()
        }
        #[cfg(target_os = "macos")]
        {
            let mut tv = libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            };
            let mut len = std::mem::size_of::<libc::timeval>();
            let name = c"kern.boottime";
            // SAFETY: a sysctl read into a correctly sized timeval.
            let rc = unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    (&mut tv as *mut libc::timeval).cast(),
                    &mut len,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if rc != 0 {
                return None;
            }
            // `tv_sec` is `time_t`, an i64 on macOS.
            time::OffsetDateTime::from_unix_timestamp(tv.tv_sec).ok()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }

    #[cfg(target_os = "macos")]
    fn sysctl_string(name: &str) -> Option<String> {
        let cname = std::ffi::CString::new(name).ok()?;
        let mut len: usize = 0;
        // SAFETY: a size query, then a read into a buffer of that size.
        unsafe {
            if libc::sysctlbyname(
                cname.as_ptr(),
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            ) != 0
                || len == 0
            {
                return None;
            }
            let mut buf = vec![0u8; len];
            if libc::sysctlbyname(
                cname.as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            ) != 0
            {
                return None;
            }
            buf.truncate(len);
            let s = String::from_utf8_lossy(&buf);
            let s = s.trim_end_matches('\0').trim();
            (!s.is_empty()).then(|| s.to_string())
        }
    }

    pub fn pid_alive(pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        if pid <= 0 {
            return false;
        }
        // SAFETY: `kill` with signal 0 performs no action beyond the check.
        let rc = unsafe { libc::kill(pid, 0) };
        if rc == 0 {
            return true;
        }
        // EPERM: the process exists but belongs to someone else.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// Windows keeps no boot id in user space; the identity is the boot time
    /// derived from the tick count, compared with a tolerance.
    pub fn boot_id() -> Option<String> {
        boot_time().map(|t| format!("boot:{}", t.unix_timestamp()))
    }

    pub fn boot_time() -> Option<time::OffsetDateTime> {
        // SAFETY: GetTickCount64 has no preconditions.
        let up_ms = unsafe { GetTickCount64() };
        let now = time::OffsetDateTime::now_utc();
        Some(now - time::Duration::milliseconds(i64::try_from(up_ms).unwrap_or(0)))
    }

    pub fn pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        // SAFETY: plain Win32 calls with checked handles.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub fn boot_id() -> Option<String> {
        None
    }

    pub fn boot_time() -> Option<time::OffsetDateTime> {
        None
    }

    pub fn pid_alive(_pid: u32) -> bool {
        true
    }
}

/// The pid and tty of a live external session, as the relay reported them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRef {
    pub pid: u32,
    pub tty: Option<String>,
}

/// Session id → process, kept in memory (05 §9.1 "stored in memory per session").
#[derive(Debug, Default)]
pub struct Processes {
    by_session: HashMap<String, ProcessRef>,
}

impl Processes {
    /// Remember a session's process; `true` when the pid is new or changed
    /// (the caller persists it then).
    pub fn record(&mut self, session_id: &str, pid: Option<u32>, tty: Option<&str>) -> bool {
        let Some(pid) = pid else {
            return false;
        };
        let changed = self.by_session.get(session_id).is_none_or(|p| p.pid != pid);
        self.by_session.insert(
            session_id.to_string(),
            ProcessRef {
                pid,
                tty: tty.map(str::to_string),
            },
        );
        changed
    }

    pub fn get(&self, session_id: &str) -> Option<&ProcessRef> {
        self.by_session.get(session_id)
    }

    pub fn forget(&mut self, session_id: &str) {
        self.by_session.remove(session_id);
    }

    /// The sessions whose process is gone; they are forgotten in the same call.
    pub fn reap(&mut self, alive: impl Fn(u32) -> bool) -> Vec<String> {
        let dead: Vec<String> = self
            .by_session
            .iter()
            .filter(|(_, p)| !alive(p.pid))
            .map(|(s, _)| s.clone())
            .collect();
        for s in &dead {
            self.by_session.remove(s);
        }
        dead
    }

    pub fn len(&self) -> usize {
        self.by_session.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_session.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_identity_is_stable_and_tolerant_only_for_tick_times() {
        let id = boot_id();
        assert!(!id.is_empty());
        assert_eq!(id, boot_id());
        assert!(same_boot("abc", "abc"));
        assert!(!same_boot("abc", "abd"));
        assert!(same_boot("boot:1000", "boot:1030"));
        assert!(!same_boot("boot:1000", "boot:1100"));
        assert!(!same_boot("boot:1000", "abc"));
    }

    #[test]
    fn record_reports_a_new_or_changed_pid_only() {
        let mut p = Processes::default();
        assert!(!p.record("s", None, None));
        assert!(p.record("s", Some(7), None));
        assert!(!p.record("s", Some(7), Some("/dev/ttys1")));
        assert!(p.record("s", Some(8), None));
        assert_eq!(p.get("s").map(|r| r.pid), Some(8));
    }

    #[test]
    fn own_process_is_alive_and_an_absurd_pid_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
        #[cfg(unix)]
        assert!(!pid_alive(u32::MAX / 2), "no such pid on any sane system");
    }

    #[test]
    fn processes_reap_the_dead() {
        let mut p = Processes::default();
        p.record("a", Some(10), Some("/dev/ttys1"));
        p.record("b", Some(20), None);
        p.record("c", None, None);
        assert_eq!(p.len(), 2);
        assert_eq!(p.get("a").unwrap().tty.as_deref(), Some("/dev/ttys1"));
        let dead = p.reap(|pid| pid == 10);
        assert_eq!(dead, vec!["b".to_string()]);
        assert_eq!(p.len(), 1);
        p.forget("a");
        assert!(p.is_empty());
    }
}
