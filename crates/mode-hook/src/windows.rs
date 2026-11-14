//! Windows transport: the named pipe `\\.\pipe\mode[-dev]-<sid>`, opened with
//! the standard library (a byte-mode client needs no `windows` crate).
//! `ERROR_PIPE_BUSY` is retried until the 300 ms budget is spent.
//!
//! Type-checked on `x86_64-pc-windows-msvc`; untested on a Windows machine
//! until it runs on one.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

use mode_wire::Endpoint;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::GetConsoleWindow;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;

pub type Conn = File;

/// All pipe instances are busy; `WaitNamedPipe` would block, polling is simpler.
const ERROR_PIPE_BUSY: i32 = 231;

pub fn connect(endpoint: &Endpoint, timeout: Duration) -> Option<Conn> {
    let name = endpoint.pipe_name();
    let deadline = Instant::now() + timeout;
    loop {
        match OpenOptions::new().read(true).write(true).open(&name) {
            Ok(f) => return Some(f),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

/// The `claude` process id (05 §9.1): exec-form hooks make the relay its
/// direct child, so the parent in a Toolhelp32 process snapshot is `claude`.
pub fn parent_pid() -> Option<u32> {
    // SAFETY: the snapshot handle is owned here and closed on every path; the
    // entry struct is initialized with its size as the API requires.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let me = GetCurrentProcessId();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == me {
                    found = Some(entry.th32ParentProcessID);
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        found.filter(|p| *p > 0)
    }
}

/// Windows has no controlling tty; the console window handle (inherited from
/// `claude`) plays its part in `terminal_ref` (05 §9.1: `hwnd=0x3a0b2`).
/// `None` for a headless run without a console.
pub fn tty_name() -> Option<String> {
    // SAFETY: GetConsoleWindow has no preconditions.
    let hwnd = unsafe { GetConsoleWindow() };
    if hwnd.is_null() {
        None
    } else {
        Some(format!("hwnd=0x{:x}", hwnd as usize))
    }
}
