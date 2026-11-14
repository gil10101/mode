//! The current user's SID string on Windows (05 §3.2: the pipe is
//! `\\.\pipe\mode-<sid>`; the core's DACL names the same SID). Read from the
//! process token with `GetTokenInformation`, so no configuration is needed.

#![allow(unsafe_code)]

use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::core::PWSTR;

/// `S-1-5-21-…-1001` for the user running this process, or `None` when the
/// token cannot be read.
pub fn current_user_sid() -> Option<String> {
    // SAFETY: every handle and buffer below is owned by this function; the
    // token is closed on every path and the SID string is freed with LocalFree
    // as the API requires.
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut len);
        if len == 0 {
            CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let user = buf.as_ptr().cast::<TOKEN_USER>();
        let mut wide: PWSTR = ptr::null_mut();
        if ConvertSidToStringSidW((*user).User.Sid, &mut wide) == 0 || wide.is_null() {
            return None;
        }
        let mut n = 0usize;
        while *wide.add(n) != 0 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(wide, n));
        LocalFree(wide.cast());
        Some(sid)
    }
}
