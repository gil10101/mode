//! The PTY host proper (05 §8.1, §8.3; 08 §2.3, §5.4): one pseudo-terminal
//! per embedded session on `portable-pty` (openpty + fork/exec on macOS and
//! Linux, ConPTY on Windows). Per process three threads:
//!
//! * the **reader** blocks on the master and forwards raw bytes;
//! * the **pump** appends them to the [`Scrollback`] ring, coalesces them
//!   into chunks (one per ~16 ms window, ≤ 64 KB), reports output activity
//!   (the "output flowing" heuristic: working while bytes flow, idle after
//!   2 s of silence) and drives the periodic [`HostEvents::tick`];
//! * the **waiter** blocks on the child and reports its exit.
//!
//! The host never interprets the bytes (05 §8.4: no scraping); it only
//! counts them. Signals: SIGINT only for interrupts, SIGHUP (what a closed
//! terminal sends) as the last step of "Close terminal"; never SIGTERM
//! (ADR-0005), never `ChildKiller::kill` (it escalates to SIGKILL).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::Timings;

/// Largest chunk handed to the webview (08 §5.4).
pub const CHUNK_MAX: usize = 64 * 1024;
/// How long the pump waits for the reader's last bytes after the child exits.
const DRAIN_AFTER_EXIT: Duration = Duration::from_millis(250);
/// Bracketed paste markers (xterm DECSET 2004).
pub const PASTE_START: &[u8] = b"\x1b[200~";
pub const PASTE_END: &[u8] = b"\x1b[201~";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------- scrollback

/// The bytes kept for replay on attach (05 §8.3: 10 000 lines; a byte cap
/// bounds a TUI that redraws without newlines). Trimmed from the front at
/// line boundaries; offsets count bytes since the PTY spawned.
#[derive(Debug, Clone)]
pub struct Scrollback {
    buf: VecDeque<u8>,
    /// Stream offset of `buf[0]`.
    start: u64,
    /// `\n` in `buf`.
    lines: usize,
    max_lines: usize,
    max_bytes: usize,
}

impl Scrollback {
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Scrollback {
            buf: VecDeque::new(),
            start: 0,
            lines: 0,
            max_lines: max_lines.max(1),
            max_bytes: max_bytes.max(1),
        }
    }

    /// Append; returns the stream offset of the first appended byte.
    pub fn push(&mut self, bytes: &[u8]) -> u64 {
        let offset = self.end();
        self.buf.extend(bytes.iter().copied());
        self.lines += bytes.iter().filter(|&&b| b == b'\n').count();
        self.trim();
        offset
    }

    fn drop_front(&mut self, n: usize) {
        let n = n.min(self.buf.len());
        let newlines = self.buf.range(..n).filter(|&&b| b == b'\n').count();
        self.buf.drain(..n);
        self.start += n as u64;
        self.lines -= newlines;
    }

    fn trim(&mut self) {
        while self.lines > self.max_lines {
            match self.buf.iter().position(|&b| b == b'\n') {
                Some(pos) => self.drop_front(pos + 1),
                None => break,
            }
        }
        if self.buf.len() > self.max_bytes {
            self.drop_front(self.buf.len() - self.max_bytes);
            // Start on a line boundary when one is near.
            if let Some(pos) = self.buf.iter().take(4096).position(|&b| b == b'\n') {
                self.drop_front(pos + 1);
            }
        }
    }

    /// Offset after the last byte.
    pub fn end(&self) -> u64 {
        self.start + self.buf.len() as u64
    }

    pub fn start(&self) -> u64 {
        self.start
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn lines(&self) -> usize {
        self.lines
    }

    /// `(offset of the first byte, bytes)`.
    pub fn snapshot(&self) -> (u64, Vec<u8>) {
        (self.start, self.buf.iter().copied().collect())
    }
}

// ------------------------------------------------------------------- paste

/// The prompt box's bytes (05 §14): bracketed paste, so the CLI treats the
/// text as typed-in content, never as keys. Paste markers inside the text are
/// removed (they would end the paste early and turn the rest into
/// keystrokes); newlines become `\r` as a terminal's paste does.
pub fn bracketed(text: &str) -> Vec<u8> {
    let clean = text
        .replace("\x1b[200~", "")
        .replace("\x1b[201~", "")
        .replace("\r\n", "\r")
        .replace('\n', "\r");
    [PASTE_START, clean.as_bytes(), PASTE_END].concat()
}

/// Write the paste, wait `delay` (50 ms), then `\r` (05 §14).
pub fn deliver(w: &mut dyn Write, text: &str, delay: Duration) -> io::Result<()> {
    w.write_all(&bracketed(text))?;
    w.flush()?;
    std::thread::sleep(delay);
    w.write_all(b"\r")?;
    w.flush()
}

// ------------------------------------------------------------------ process

/// How the child ended: `code` is `None` when a signal killed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<String>,
}

impl From<portable_pty::ExitStatus> for ExitInfo {
    fn from(s: portable_pty::ExitStatus) -> Self {
        match s.signal() {
            Some(sig) => ExitInfo {
                code: None,
                signal: Some(sig.to_string()),
            },
            None => ExitInfo {
                code: Some(s.exit_code() as i32),
                signal: None,
            },
        }
    }
}

/// What the host tells its owner (the [`super::Pty`] service). Called from
/// the pump thread of the process.
pub trait HostEvents: Send + Sync {
    /// A coalesced chunk (≤ 64 KB) starting at stream `offset`.
    fn output(&self, proc: &Arc<PtyProc>, offset: u64, bytes: &[u8]);
    /// Output started flowing (`true`) or stopped for `idle_after` (`false`).
    /// The `true` edge comes before the bytes that caused it reach the
    /// scrollback, so output is never on screen for a row that has not yet
    /// been told it is working.
    fn activity(&self, proc: &Arc<PtyProc>, working: bool);
    /// About every `tick` while the process lives.
    fn tick(&self, proc: &Arc<PtyProc>);
    /// The child exited and its last bytes were flushed. The process still
    /// reads as alive during this call and is marked exited when it returns,
    /// so whoever waits on liveness finds the exit already announced; keep it
    /// short and put delayed work on a thread.
    fn exited(&self, proc: &Arc<PtyProc>, exit: &ExitInfo);
}

/// What to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    pub cols: u16,
    pub rows: u16,
}

/// One live (or exited, kept for replay) PTY.
pub struct PtyProc {
    pub launch_id: String,
    row_id: Mutex<String>,
    pid: Option<u32>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    size: Mutex<(u16, u16)>,
    scroll: Mutex<Scrollback>,
    exit: Mutex<Option<ExitInfo>>,
    last_output: Mutex<Option<Instant>>,
    /// A Terminal tab is attached: output is emitted (`pty_open` …
    /// `pty_detach`).
    pub attached: AtomicBool,
    /// Mode sent SIGINT (close, quit, Stop turn): exit 130 is not a failure.
    pub sigint_sent: AtomicBool,
    /// "Close terminal" is running.
    pub closing: AtomicBool,
    /// State is inferred from output (no hooks, 05 §8.1).
    pub infer: AtomicBool,
    /// The 30 s "no hook events" check ran.
    pub hooks_checked: AtomicBool,
    pub spawned_at: Instant,
    /// RFC 3339 spawn time (compared with `session_events.received_at`).
    pub spawned_ts: String,
}

impl std::fmt::Debug for PtyProc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyProc")
            .field("launch_id", &self.launch_id)
            .field("row_id", &self.row_id())
            .field("pid", &self.pid)
            .field("exit", &self.exit_info())
            .finish_non_exhaustive()
    }
}

impl PtyProc {
    pub fn row_id(&self) -> String {
        lock(&self.row_id).clone()
    }

    pub fn set_row_id(&self, id: &str) {
        *lock(&self.row_id) = id.to_string();
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn is_alive(&self) -> bool {
        lock(&self.exit).is_none()
    }

    pub fn exit_info(&self) -> Option<ExitInfo> {
        lock(&self.exit).clone()
    }

    pub fn size(&self) -> (u16, u16) {
        *lock(&self.size)
    }

    /// `(offset, bytes)` of the replay buffer.
    pub fn snapshot(&self) -> (u64, Vec<u8>) {
        lock(&self.scroll).snapshot()
    }

    /// Stream offset after the last byte read.
    pub fn stream_end(&self) -> u64 {
        lock(&self.scroll).end()
    }

    /// Time since the last output (since spawn when there was none).
    pub fn silent_for(&self) -> Duration {
        lock(&self.last_output).unwrap_or(self.spawned_at).elapsed()
    }

    fn dead() -> io::Error {
        io::Error::new(io::ErrorKind::BrokenPipe, "the terminal has exited")
    }

    /// Keystrokes and pastes, in order.
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        if !self.is_alive() {
            return Err(Self::dead());
        }
        let mut w = lock(&self.writer);
        let w = w.as_mut().ok_or_else(Self::dead)?;
        w.write_all(bytes)?;
        w.flush()
    }

    /// The prompt box through bracketed paste, then `\r` after `delay`; the
    /// writer stays locked so no keystroke lands between the two.
    pub fn paste(&self, text: &str, delay: Duration) -> io::Result<()> {
        if !self.is_alive() {
            return Err(Self::dead());
        }
        let mut w = lock(&self.writer);
        let w = w.as_mut().ok_or_else(Self::dead)?;
        deliver(w.as_mut(), text, delay)
    }

    /// `TIOCSWINSZ` / `ResizePseudoConsole`.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        if !self.is_alive() {
            return Err("the terminal has exited".into());
        }
        let m = lock(&self.master);
        let m = m.as_ref().ok_or("the terminal has exited")?;
        m.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;
        *lock(&self.size) = (cols, rows);
        Ok(())
    }

    /// SIGINT to `claude` (unix), Ctrl+C through the console (Windows).
    pub fn sigint(&self) -> io::Result<()> {
        if !self.is_alive() {
            return Ok(());
        }
        self.sigint_sent.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        {
            let pid = self.pid.ok_or_else(Self::dead)?;
            // SAFETY: plain kill(2) on the child's pid.
            let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGINT) };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            // ConPTY turns 0x03 into CTRL_C_EVENT (or the CLI reads it as
            // Ctrl+C in raw mode); there is no SIGINT on Windows.
            self.write(&[0x03])
        }
    }

    /// End-of-input at an empty prompt (Ctrl+D).
    pub fn eof(&self) -> io::Result<()> {
        self.write(&[0x04])
    }

    /// The hang-up a closed terminal sends: SIGHUP on unix; on Windows the
    /// pseudo console is closed. Never SIGTERM.
    pub fn hangup(&self) {
        if !self.is_alive() {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // SAFETY: plain kill(2) on the child's pid.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGHUP);
            }
        }
        #[cfg(not(unix))]
        {
            lock(&self.writer).take();
            lock(&self.master).take();
        }
    }

    fn set_exit(&self, e: &ExitInfo) {
        *lock(&self.exit) = Some(e.clone());
    }
}

/// Spawn options besides the command.
#[derive(Debug, Clone)]
pub struct ProcOpts {
    pub scrollback_lines: usize,
    pub scrollback_bytes: usize,
    pub timings: Timings,
    /// Infer state from output from the start (hooks not installed).
    pub infer: bool,
}

enum Msg {
    Data(Vec<u8>),
    Eof,
    Exit(ExitInfo),
}

/// Open a PTY of `cols × rows`, spawn the command in it and start the three
/// threads. The slave side is closed in this process so the reader sees EOF
/// when the child (and its children) let go of it.
pub fn spawn(
    spec: &SpawnSpec,
    launch_id: &str,
    row_id: &str,
    opts: &ProcOpts,
    events: Arc<dyn HostEvents>,
) -> Result<Arc<PtyProc>, String> {
    let system = native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not open a terminal: {e}"))?;
    let mut cmd = CommandBuilder::new(&spec.program);
    cmd.args(&spec.args);
    cmd.cwd(&spec.cwd);
    for k in &spec.env_remove {
        cmd.env_remove(k);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let master = pair.master;
    let reader = master.try_clone_reader().map_err(|e| e.to_string());
    let writer = master.take_writer().map_err(|e| e.to_string());
    let (reader, writer) = match (reader, writer) {
        (Ok(r), Ok(w)) => (r, w),
        (Err(e), _) | (_, Err(e)) => {
            // The child is running but unreachable: hang it up, never kill.
            #[cfg(unix)]
            if let Some(pid) = child.process_id() {
                // SAFETY: plain kill(2) on the child's pid.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGHUP);
                }
            }
            let _ = std::thread::Builder::new()
                .name("mode-pty-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                });
            return Err(e);
        }
    };
    let proc = Arc::new(PtyProc {
        launch_id: launch_id.to_string(),
        row_id: Mutex::new(row_id.to_string()),
        pid: child.process_id(),
        master: Mutex::new(Some(master)),
        writer: Mutex::new(Some(writer)),
        size: Mutex::new((spec.cols, spec.rows)),
        scroll: Mutex::new(Scrollback::new(
            opts.scrollback_lines,
            opts.scrollback_bytes,
        )),
        exit: Mutex::new(None),
        last_output: Mutex::new(None),
        attached: AtomicBool::new(false),
        sigint_sent: AtomicBool::new(false),
        closing: AtomicBool::new(false),
        infer: AtomicBool::new(opts.infer),
        hooks_checked: AtomicBool::new(false),
        spawned_at: Instant::now(),
        spawned_ts: crate::db::now_rfc3339(),
    });
    let (tx, rx) = mpsc::channel::<Msg>();
    let tx_wait = tx.clone();
    std::thread::Builder::new()
        .name("mode-pty-read".into())
        .spawn(move || read_loop(reader, tx))
        .map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("mode-pty-wait".into())
        .spawn(move || {
            let info = match child.wait() {
                Ok(status) => ExitInfo::from(status),
                Err(e) => {
                    tracing::warn!(target: "pty", error = %e, "wait on the terminal child failed");
                    ExitInfo {
                        code: None,
                        signal: Some("unknown".into()),
                    }
                }
            };
            let _ = tx_wait.send(Msg::Exit(info));
        })
        .map_err(|e| e.to_string())?;
    let p = Arc::clone(&proc);
    let t = opts.timings.clone();
    std::thread::Builder::new()
        .name("mode-pty-pump".into())
        .spawn(move || pump(p, rx, events, t))
        .map_err(|e| e.to_string())?;
    Ok(proc)
}

fn read_loop(mut reader: Box<dyn Read + Send>, tx: Sender<Msg>) {
    let mut buf = vec![0u8; CHUNK_MAX];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(Msg::Data(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // EIO on Linux once every slave fd is closed.
            Err(_) => break,
        }
    }
    let _ = tx.send(Msg::Eof);
}

fn pump(proc: Arc<PtyProc>, rx: Receiver<Msg>, events: Arc<dyn HostEvents>, t: Timings) {
    let mut pending: Vec<u8> = Vec::new();
    let mut pending_offset = 0u64;
    let mut first_at: Option<Instant> = None;
    let mut working = false;
    let mut reader_done = false;
    let mut exit: Option<(ExitInfo, Instant)> = None;
    let mut last_tick = Instant::now();
    let flush = |pending: &mut Vec<u8>, offset: u64| {
        let mut off = offset;
        for chunk in pending.chunks(CHUNK_MAX) {
            events.output(&proc, off, chunk);
            off += chunk.len() as u64;
        }
        pending.clear();
    };
    loop {
        let now = Instant::now();
        let mut wait = t.tick;
        if let Some(f) = first_at {
            wait = wait.min((f + t.coalesce).saturating_duration_since(now));
        }
        if working {
            let last = lock(&proc.last_output).unwrap_or(now);
            wait = wait.min((last + t.idle_after).saturating_duration_since(now));
        }
        if let Some((_, at)) = &exit {
            wait = wait.min((*at + DRAIN_AFTER_EXIT).saturating_duration_since(now));
        }
        match rx.recv_timeout(wait) {
            Ok(Msg::Data(b)) => {
                // The state edge first, the bytes second: a reader who finds
                // output on screen finds the row already working.
                *lock(&proc.last_output) = Some(Instant::now());
                if !working {
                    working = true;
                    events.activity(&proc, true);
                }
                let off = lock(&proc.scroll).push(&b);
                if pending.is_empty() {
                    pending_offset = off;
                    first_at = Some(Instant::now());
                }
                pending.extend_from_slice(&b);
                if pending.len() >= CHUNK_MAX {
                    flush(&mut pending, pending_offset);
                    first_at = None;
                }
            }
            Ok(Msg::Eof) => reader_done = true,
            Ok(Msg::Exit(e)) => exit = Some((e, Instant::now())),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                reader_done = true;
                if exit.is_none() {
                    exit = Some((
                        ExitInfo {
                            code: None,
                            signal: Some("unknown".into()),
                        },
                        Instant::now(),
                    ));
                }
            }
        }
        let due = first_at.is_some_and(|f| f.elapsed() >= t.coalesce);
        if !pending.is_empty() && (due || reader_done || exit.is_some()) {
            flush(&mut pending, pending_offset);
            first_at = None;
        }
        if let Some((e, at)) = &exit {
            if reader_done || at.elapsed() >= DRAIN_AFTER_EXIT {
                // Announce first, then mark: liveness flips only once the
                // exit has been published.
                events.exited(&proc, e);
                proc.set_exit(e);
                return;
            }
            continue;
        }
        if working && proc.silent_for() >= t.idle_after {
            working = false;
            events.activity(&proc, false);
        }
        if last_tick.elapsed() >= t.tick {
            last_tick = Instant::now();
            events.tick(&proc);
        }
    }
}

// --------------------------------------------------------------------- host

/// Every PTY of the app: live ones, and exited ones kept for replay until
/// the row is dismissed or a new PTY replaces them.
#[derive(Debug, Default)]
pub struct PtyHost {
    procs: Mutex<Vec<Arc<PtyProc>>>,
}

impl PtyHost {
    /// Keep a new process; older exited ones of the same row go.
    pub fn insert(&self, proc: Arc<PtyProc>) {
        let row = proc.row_id();
        let mut p = lock(&self.procs);
        p.retain(|x| x.is_alive() || x.row_id() != row);
        p.push(proc);
    }

    /// The row's PTY: the live one, else the newest exited one.
    pub fn by_row(&self, row_id: &str) -> Option<Arc<PtyProc>> {
        let p = lock(&self.procs);
        let mine = p.iter().filter(|x| x.row_id() == row_id);
        let mut newest = None;
        for x in mine {
            if x.is_alive() {
                return Some(Arc::clone(x));
            }
            newest = Some(Arc::clone(x));
        }
        newest
    }

    pub fn live(&self, row_id: &str) -> Option<Arc<PtyProc>> {
        self.by_row(row_id).filter(|p| p.is_alive())
    }

    pub fn all_live(&self) -> Vec<Arc<PtyProc>> {
        lock(&self.procs)
            .iter()
            .filter(|p| p.is_alive())
            .cloned()
            .collect()
    }

    /// Drop the exited PTYs of a row (the row was dismissed).
    pub fn forget(&self, row_id: &str) -> usize {
        let mut p = lock(&self.procs);
        let before = p.len();
        p.retain(|x| x.is_alive() || x.row_id() != row_id);
        before - p.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollback_keeps_the_last_lines_with_offsets() {
        let mut s = Scrollback::new(3, 1 << 20);
        assert_eq!(s.push(b"a\nb\n"), 0);
        assert_eq!(s.push(b"c\nd\ne"), 4);
        assert_eq!(s.lines(), 3, "a\\n dropped");
        let (off, bytes) = s.snapshot();
        assert_eq!(off, 2);
        assert_eq!(bytes, b"b\nc\nd\ne");
        assert_eq!(s.end(), 9);
        s.push(b"\nf\n");
        let (off, bytes) = s.snapshot();
        assert_eq!(bytes, b"d\ne\nf\n");
        assert_eq!(off + bytes.len() as u64, s.end());
    }

    #[test]
    fn scrollback_byte_cap_cuts_at_a_line_boundary() {
        let mut s = Scrollback::new(10_000, 10);
        s.push(b"0123456\nabcdefgh\nxy");
        let (off, bytes) = s.snapshot();
        assert!(bytes.len() <= 10);
        assert_eq!(bytes, b"xy", "cut to the newline after the byte cap");
        assert_eq!(off, 17);
        let mut s = Scrollback::new(10_000, 4);
        s.push(b"abcdefgh");
        assert_eq!(s.snapshot(), (4, b"efgh".to_vec()), "no newline: plain cut");
    }

    #[test]
    fn bracketed_paste_strips_markers_and_maps_newlines() {
        assert_eq!(bracketed("hi"), b"\x1b[200~hi\x1b[201~".to_vec());
        assert_eq!(
            bracketed("a\nb\r\nc\x1b[201~rm -rf /"),
            b"\x1b[200~a\rb\rcrm -rf /\x1b[201~".to_vec()
        );
    }

    /// Both orderings the pump promises: the `working` edge is delivered
    /// before its bytes are in the scrollback, and `exited` runs while the
    /// process still reads as alive.
    #[cfg(unix)]
    #[test]
    fn edges_precede_their_bytes_and_the_exit_precedes_the_flip() {
        use std::sync::mpsc;
        struct Rec {
            tx: Mutex<mpsc::Sender<&'static str>>,
        }
        impl Rec {
            fn say(&self, what: &'static str) {
                let _ = lock(&self.tx).send(what);
            }
        }
        impl HostEvents for Rec {
            fn output(&self, _: &Arc<PtyProc>, _: u64, _: &[u8]) {}
            fn activity(&self, proc: &Arc<PtyProc>, working: bool) {
                if working {
                    let (_, bytes) = proc.snapshot();
                    self.say(if bytes.is_empty() {
                        "working before the bytes"
                    } else {
                        "working after the bytes"
                    });
                }
            }
            fn tick(&self, _: &Arc<PtyProc>) {}
            fn exited(&self, proc: &Arc<PtyProc>, exit: &ExitInfo) {
                self.say(if proc.is_alive() && exit.code == Some(3) {
                    "exited while alive"
                } else {
                    "exited after the flip"
                });
            }
        }
        let (tx, rx) = mpsc::channel();
        let events: Arc<dyn HostEvents> = Arc::new(Rec { tx: Mutex::new(tx) });
        let spec = SpawnSpec {
            program: "sh".into(),
            args: vec!["-c".into(), "echo hi; exit 3".into()],
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            env_remove: Vec::new(),
            cols: 80,
            rows: 24,
        };
        let opts = ProcOpts {
            scrollback_lines: 100,
            scrollback_bytes: 1 << 16,
            timings: Timings::default(),
            infer: true,
        };
        let proc = spawn(&spec, "launch", "row", &opts, events).unwrap();
        let mut seen = Vec::new();
        while let Ok(what) = rx.recv_timeout(Duration::from_secs(10)) {
            seen.push(what);
            if what.starts_with("exited") {
                break;
            }
        }
        assert_eq!(seen, ["working before the bytes", "exited while alive"]);
        // The flip follows the announcement on the pump thread; this thread
        // only knows the announcement went out, so it waits for the flip.
        let until = Instant::now() + Duration::from_secs(10);
        while proc.is_alive() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!proc.is_alive(), "marked once `exited` returned");
        assert_eq!(proc.exit_info().and_then(|e| e.code), Some(3));
    }

    #[test]
    fn deliver_waits_before_the_carriage_return() {
        struct Rec(Vec<(Instant, Vec<u8>)>);
        impl Write for Rec {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.0.push((Instant::now(), b.to_vec()));
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut r = Rec(Vec::new());
        deliver(&mut r, "fix the tests", Duration::from_millis(50)).unwrap();
        assert_eq!(r.0.len(), 2);
        assert_eq!(r.0[0].1, b"\x1b[200~fix the tests\x1b[201~".to_vec());
        assert_eq!(r.0[1].1, b"\r".to_vec());
        assert!(r.0[1].0 - r.0[0].0 >= Duration::from_millis(50));
    }
}
