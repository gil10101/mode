//! Unix transport: a domain socket (macOS, Linux). `connect` runs on a helper
//! thread so the 300 ms budget holds even when the listener's backlog is full.

#![allow(unsafe_code)]

use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

use mode_wire::Endpoint;

pub type Conn = UnixStream;

pub fn connect(endpoint: &Endpoint, timeout: Duration) -> Option<Conn> {
    let path = endpoint.socket_path();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(UnixStream::connect(&path));
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(stream)) => {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
            Some(stream)
        }
        _ => None,
    }
}

/// The `claude` process: exec-form hooks make the relay its direct child.
pub fn parent_pid() -> Option<u32> {
    // SAFETY: getppid has no preconditions and cannot fail.
    let ppid = unsafe { libc::getppid() };
    u32::try_from(ppid).ok().filter(|p| *p > 1)
}

/// The controlling terminal's device name, when stderr is a tty.
pub fn tty_name() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: fd 2 is always open; buf is a valid writable buffer of the given length.
    let rc = unsafe { libc::ttyname_r(2, buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|b| *b == 0)?;
    String::from_utf8(buf[..end].to_vec()).ok()
}
