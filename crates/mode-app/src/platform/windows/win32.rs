//! Raw Win32 glue for the overlay (08 §6.3) and jump to terminal (05 §9.3),
//! written against the `windows` crate with raw `HWND`s; no Tauri types.
//!
//! tao already sets `WS_EX_NOACTIVATE` for `focusable: false` and
//! `WS_EX_TOPMOST` for `alwaysOnTop`; Mode adds `WS_EX_TOOLWINDOW` so the
//! overlay never appears in Alt-Tab or the taskbar (`skipTaskbar` only
//! deletes the taskbar tab, which can come back after an explorer restart),
//! answers `WM_MOUSEACTIVATE` with `MA_NOACTIVATE` through a comctl32
//! subclass, and implements `set_activating` for the capture box.
//!
//! Type-checked against the real crate on `x86_64-pc-windows-msvc`; untested
//! on a Windows 11 machine until the owner runs it.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_SWITCHDESKTOP, OpenInputDesktop, SwitchDesktop,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_QUIET_TIME,
    QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx, GW_OWNER, GWL_EXSTYLE,
    GetCursorPos, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    IsIconic, IsWindow, IsWindowVisible, MA_NOACTIVATE, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, SPI_GETCLIENTAREAANIMATION, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW, WM_MOUSEACTIVATE,
    WM_POWERBROADCAST, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

/// While the capture box holds key input the subclass lets clicks activate.
static ACTIVATING: AtomicBool = AtomicBool::new(false);
/// The subclass saw a resume from sleep (`WM_POWERBROADCAST`).
static RESUMED: AtomicBool = AtomicBool::new(false);
const SUBCLASS_ID: usize = 0x4d4f_4445; // "MODE"

/// Set or clear extended-style bits and let the frame pick the change up
/// without activating, moving or resizing the window.
///
/// # Safety
/// `raw` must be a live top-level window handle; call on its thread.
unsafe fn update_ex_style(raw: isize, set: isize, clear: isize) -> windows::core::Result<()> {
    let h = hwnd(raw);
    // SAFETY: by the caller's contract hwnd is live; these calls only read
    // and write its extended style bits.
    unsafe {
        let current = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let wanted = (current | set) & !clear;
        if wanted != current {
            SetWindowLongPtrW(h, GWL_EXSTYLE, wanted);
            SetWindowPos(
                h,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            )?;
        }
    }
    Ok(())
}

/// OR the tool-window and no-activate bits onto the extended style of the
/// window whose handle is `raw` (from `WebviewWindow::hwnd()`).
///
/// # Safety
/// `raw` must be a live top-level window handle owned by tao, and the call
/// must happen on that window's thread.
pub unsafe fn make_tool_window(raw: isize) -> windows::core::Result<()> {
    // SAFETY: by the caller's contract.
    unsafe { update_ex_style(raw, (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as isize, 0) }
}

unsafe extern "system" fn no_activate_proc(
    h: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_MOUSEACTIVATE && !ACTIVATING.load(Ordering::Relaxed) {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    // Every top-level window gets the power broadcasts; the overlay's
    // subclass notes a resume for the connectors' catch-up poll.
    if msg == WM_POWERBROADCAST
        && (wparam.0 == PBT_APMRESUMEAUTOMATIC as usize
            || wparam.0 == PBT_APMRESUMESUSPEND as usize)
    {
        RESUMED.store(true, Ordering::Relaxed);
    }
    // SAFETY: forwarding the message this subclass received.
    unsafe { DefSubclassProc(h, msg, wparam, lparam) }
}

/// Answer `WM_MOUSEACTIVATE` with `MA_NOACTIVATE` (08 §6.3 "Hover
/// activation"; tao does not). WebView2's child window cannot be subclassed;
/// whether the parent's answer suffices is an owner check.
///
/// # Safety
/// As [`make_tool_window`].
pub unsafe fn install_no_activate(raw: isize) -> bool {
    // SAFETY: by the caller's contract; the subclass proc lives for the process.
    unsafe { SetWindowSubclass(hwnd(raw), Some(no_activate_proc), SUBCLASS_ID, 0) }.as_bool()
}

/// `set_activating(true)` (02 §9.2): clear `WS_EX_NOACTIVATE`, remember the
/// foreground window, bring the overlay to the foreground. Returns the
/// previous foreground window.
///
/// # Safety
/// As [`make_tool_window`].
pub unsafe fn begin_key(raw: isize) -> Option<isize> {
    // SAFETY: by the caller's contract.
    unsafe {
        let previous = GetForegroundWindow();
        ACTIVATING.store(true, Ordering::Relaxed);
        let _ = update_ex_style(raw, 0, WS_EX_NOACTIVATE.0 as isize);
        let _ = SetForegroundWindow(hwnd(raw));
        (!previous.is_invalid() && previous != hwnd(raw)).then_some(previous.0 as isize)
    }
}

/// `set_activating(false)` (02 §9.3): give the foreground back to the
/// previous window while Mode still owns it (the foreground-lock rule allows
/// that), then restore `WS_EX_NOACTIVATE`. A previous window that is gone
/// gets no call.
///
/// # Safety
/// As [`make_tool_window`].
pub unsafe fn end_key(raw: isize, previous: Option<isize>) {
    // SAFETY: by the caller's contract; `IsWindow` guards a stale handle.
    unsafe {
        if let Some(prev) = previous
            && GetForegroundWindow() == hwnd(raw)
            && IsWindow(Some(hwnd(prev))).as_bool()
        {
            let _ = SetForegroundWindow(hwnd(prev));
        }
        let _ = update_ex_style(raw, WS_EX_NOACTIVATE.0 as isize, 0);
        ACTIVATING.store(false, Ordering::Relaxed);
    }
}

/// Cursor position (physical; the process is per-monitor DPI aware v2) and
/// whether any mouse button is held. Any thread.
pub fn cursor_and_buttons() -> Option<(f64, f64, bool)> {
    let mut p = POINT::default();
    // SAFETY: plain reads of global input state into a local.
    unsafe {
        GetCursorPos(&mut p).ok()?;
        let held = |vk: i32| (GetAsyncKeyState(vk) as u16 & 0x8000) != 0;
        let buttons =
            held(VK_LBUTTON.0 as i32) || held(VK_RBUTTON.0 as i32) || held(VK_MBUTTON.0 as i32);
        Some((f64::from(p.x), f64::from(p.y), buttons))
    }
}

/// `SPI_GETCLIENTAREAANIMATION` off → reduced motion (02 §12).
pub fn reduced_motion() -> Option<bool> {
    let mut enabled = BOOL(1);
    // SAFETY: the out-pointer is a local BOOL, as the action expects.
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut BOOL as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
    }
    Some(!enabled.as_bool())
}

/// Presentation mode or exclusive full screen (02 §6.6): only
/// `QUNS_PRESENTATION_MODE` and `QUNS_RUNNING_D3D_FULL_SCREEN`, so a
/// borderless full-screen terminal keeps the overlay.
pub fn presentation() -> Option<bool> {
    // SAFETY: no arguments; returns a value.
    let state = unsafe { SHQueryUserNotificationState() }.ok()?;
    Some(state == QUNS_PRESENTATION_MODE || state == QUNS_RUNNING_D3D_FULL_SCREEN)
}

/// Focus Assist / Do Not Disturb (07 §8.5.2): `QUNS_BUSY` and
/// `QUNS_QUIET_TIME` (presentation and full screen are reported apart).
pub fn focus_assist() -> Option<bool> {
    // SAFETY: no arguments; returns a value.
    let state = unsafe { SHQueryUserNotificationState() }.ok()?;
    Some(state == QUNS_BUSY || state == QUNS_QUIET_TIME)
}

/// The workstation is locked: the input desktop cannot be opened or
/// switched to (it is the secure desktop then).
pub fn screen_locked() -> bool {
    // SAFETY: the handle is closed before returning; no other state.
    unsafe {
        match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_SWITCHDESKTOP) {
            Ok(h) => {
                let switched = SwitchDesktop(h).is_ok();
                let _ = CloseDesktop(h);
                !switched
            }
            Err(_) => true,
        }
    }
}

/// Whether a resume was seen since the last call.
pub fn take_resumed() -> bool {
    RESUMED.swap(false, Ordering::Relaxed)
}

/// One process of a snapshot: pid, parent pid, exe name (lowercase).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    pub exe: String,
}

/// Every process (Toolhelp32 snapshot).
pub fn processes() -> Vec<ProcessEntry> {
    let mut out = Vec::new();
    // SAFETY: the snapshot handle is closed below; the entry is a local with
    // `dwSize` set as the API requires.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(e.szExeFile.len());
            out.push(ProcessEntry {
                pid: e.th32ProcessID,
                parent: e.th32ParentProcessID,
                exe: String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase(),
            });
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

struct Collect {
    windows: Vec<(isize, u32)>,
}

unsafe extern "system" fn collect_proc(h: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `Collect` passed by `top_level_windows` for the
    // duration of EnumWindows.
    let c = unsafe { &mut *(lparam.0 as *mut Collect) };
    // SAFETY: h is a live window handed to the callback.
    unsafe {
        if IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).is_err() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            c.windows.push((h.0 as isize, pid));
        }
    }
    BOOL(1)
}

/// Visible, unowned top-level windows with their process ids.
pub fn top_level_windows() -> Vec<(isize, u32)> {
    let mut c = Collect {
        windows: Vec::new(),
    };
    // SAFETY: the callback only touches `c`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect_proc), LPARAM(&mut c as *mut Collect as isize));
    }
    c.windows
}

/// Bring a window forward (05 §9.3): restore it when minimized, then
/// `SetForegroundWindow`; when the foreground lock refuses, flash its
/// taskbar button. Returns whether it became the foreground window.
pub fn focus_window(raw: isize) -> bool {
    let h = hwnd(raw);
    // SAFETY: `IsWindow` guards a stale handle; the rest only acts on it.
    unsafe {
        if !IsWindow(Some(h)).as_bool() {
            return false;
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if SetForegroundWindow(h).as_bool() {
            return true;
        }
        let info = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd: h,
            dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
            uCount: 3,
            dwTimeout: 0,
        };
        let _ = FlashWindowEx(&info);
        false
    }
}
