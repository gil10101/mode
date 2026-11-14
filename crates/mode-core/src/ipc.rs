//! Hook IPC server (08 §5.2; 05 §3.2–§3.5, §7.4).
//!
//! Listens on the endpoint (`UnixListener` on unix, a named pipe on Windows),
//! reads NDJSON frames from each relay, writes the `ack` **before** anything
//! else (target ≤ 50 ms), hands every `hook` frame to the core through the
//! 1 000-frame memory ring (oldest dropped when the consumer is slow) and
//! keeps the connection of a decision event open in the pending registry
//! until [`Server::decide`], [`Server::no_opinion`], a relay `cancel`, EOF, a
//! later frame that supersedes it (`PostToolUse` for the same tool use,
//! `SessionEnd`) or the watchdog resolves it. The watchdog answers
//! `no_opinion` just before the relay's own deadline (`decision_wait_ms`), so
//! an unanswered card always ends in Claude Code's terminal prompt without
//! the relay having to time out.
//!
//! Security (05 §3.5): unix sockets are `0600` in a `0700` directory and the
//! peer uid must equal ours (`SO_PEERCRED` / `LOCAL_PEERCRED`); the Windows
//! pipe is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`,
//! `PIPE_REJECT_REMOTE_CLIENTS` and a DACL for the current user's SID only.
//!
//! Logging (08 §12): `tool_input` / `tool_response` are logged as sizes only.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mode_wire::{
    AckFrame, DecisionFrame, Endpoint, Frame, HookFrame, MAX_FRAME_BYTES, NoOpinionFrame, events,
};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::redact;

/// Capacity of the memory ring (08 §5.2: a 1 000-frame memory ring).
pub const DEFAULT_CAPACITY: usize = 1000;
/// The channel between the ring's drain task and the consumer; the ring is the buffer.
const HANDOFF_CAPACITY: usize = 1;
/// Decision wait assumed for a relay that did not say (`120 s − 2 s`, 05 §3.3).
pub const DEFAULT_DECISION_WAIT: Duration = Duration::from_secs(118);
/// The watchdog answers `no_opinion` this long before the relay's deadline.
pub const WATCHDOG_MARGIN: Duration = Duration::from_millis(750);
/// The relay version this core expects on every frame (08 §5.2 "Version check").
pub const EXPECTED_RELAY_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Listener backlog (08 §5.2).
#[cfg(unix)]
const BACKLOG: i32 = 128;

/// A hook frame the core has acknowledged and must now process.
#[derive(Debug)]
pub struct Inbound {
    pub frame: HookFrame,
    pub received_at: Instant,
    /// `relay_version` differs from this core's: the install is `stale`
    /// (08 §5.2); the frame is processed anyway.
    pub relay_stale: bool,
}

// ---------------------------------------------------------------- peer check

/// Who is on the other end of a hook connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    Uid(u32),
    Sid(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PeerError {
    #[error("peer credentials unavailable: {0}")]
    Unavailable(String),
    #[error("peer {peer:?} is not this user ({own:?})")]
    Mismatch { peer: Peer, own: Peer },
}

/// The rule of 05 §3.2: a connection is served only when the peer is the
/// same OS user as the core. Pure, so it is unit-tested with fake credentials.
pub fn check_peer(peer: Result<Peer, String>, own: &Peer) -> Result<(), PeerError> {
    match peer {
        Err(e) => Err(PeerError::Unavailable(e)),
        Ok(p) if p == *own => Ok(()),
        Ok(p) => Err(PeerError::Mismatch {
            peer: p,
            own: own.clone(),
        }),
    }
}

/// The SDDL of the Windows pipe: a protected DACL granting generic all to one
/// SID and nothing to anyone else (05 §3.2 "A DACL granting only the current
/// user's SID").
pub fn pipe_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

// ------------------------------------------------------------------ metrics

/// Live counters for Diagnostics (03 §8.16; 08 §12 "every section reads live counters").
#[derive(Debug, Default)]
pub struct Metrics {
    frames: AtomicU64,
    acks: AtomicU64,
    decisions: AtomicU64,
    no_opinions: AtomicU64,
    timeouts: AtomicU64,
    cancels: AtomicU64,
    eof_before_decision: AtomicU64,
    superseded: AtomicU64,
    rejected_peers: AtomicU64,
    malformed: AtomicU64,
    stale_relay: AtomicU64,
    truncated: AtomicU64,
    ring_dropped: AtomicU64,
    round_trips: AtomicU64,
    max_ack_latency_us: AtomicU64,
    last_frame: Mutex<Option<Instant>>,
}

impl Metrics {
    fn bump(c: &AtomicU64) -> u64 {
        c.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let last = *self.last_frame.lock().unwrap_or_else(|p| p.into_inner());
        MetricsSnapshot {
            frames: self.frames.load(Ordering::Relaxed),
            acks: self.acks.load(Ordering::Relaxed),
            decisions: self.decisions.load(Ordering::Relaxed),
            no_opinions: self.no_opinions.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            cancels: self.cancels.load(Ordering::Relaxed),
            eof_before_decision: self.eof_before_decision.load(Ordering::Relaxed),
            superseded: self.superseded.load(Ordering::Relaxed),
            rejected_peers: self.rejected_peers.load(Ordering::Relaxed),
            malformed: self.malformed.load(Ordering::Relaxed),
            stale_relay: self.stale_relay.load(Ordering::Relaxed),
            truncated: self.truncated.load(Ordering::Relaxed),
            ring_dropped: self.ring_dropped.load(Ordering::Relaxed),
            round_trips: self.round_trips.load(Ordering::Relaxed),
            max_ack_latency_us: self.max_ack_latency_us.load(Ordering::Relaxed),
            last_frame_age_ms: last.map(|t| t.elapsed().as_millis() as u64),
        }
    }
}

/// A copy of [`Metrics`] for the Diagnostics view and `core_status`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct MetricsSnapshot {
    pub frames: u64,
    pub acks: u64,
    pub decisions: u64,
    pub no_opinions: u64,
    pub timeouts: u64,
    pub cancels: u64,
    pub eof_before_decision: u64,
    pub superseded: u64,
    pub rejected_peers: u64,
    pub malformed: u64,
    pub stale_relay: u64,
    pub truncated: u64,
    pub ring_dropped: u64,
    pub round_trips: u64,
    pub max_ack_latency_us: u64,
    pub last_frame_age_ms: Option<u64>,
}

// ----------------------------------------------------------------- registry

/// An open decision, as listed for the overlay and the app window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingDecision {
    /// The frame id ([`Server::decide`] takes it).
    pub id: String,
    pub session_id: String,
    pub tool_use_id: Option<String>,
    pub event: String,
    pub tool_name: Option<String>,
    /// Until the watchdog answers `no_opinion`.
    pub remaining_ms: u64,
}

#[derive(Debug)]
enum Outcome {
    Decision(Value),
    NoOpinion,
    /// The watchdog fired.
    Timeout,
    /// Answered elsewhere: `PostToolUse` for the tool use, or `SessionEnd`.
    Superseded,
}

/// How a pending decision ended, for the session engine (05 §7.4: a card
/// grays on `cancel` / deadline / EOF, "answered in terminal" on supersede).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedKind {
    /// Mode answered (`decide` / `decide_tool`).
    Decision,
    /// Mode chose Claude Code's own prompt (`no_opinion`).
    NoOpinion,
    /// The watchdog answered `no_opinion` just before the relay's deadline.
    Timeout,
    /// A later frame closed it (`PostToolUse` for the tool use, `SessionEnd`).
    Superseded,
    /// The relay sent `cancel` (its own deadline or a signal).
    Cancelled,
    /// The relay went away before any reply (Claude Code exited, or the hook was killed).
    Eof,
}

/// A pending decision that is no longer pending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolved {
    /// The frame id.
    pub id: String,
    pub session_id: String,
    pub tool_use_id: Option<String>,
    pub event: String,
    pub tool_name: Option<String>,
    pub kind: ResolvedKind,
}

struct Entry {
    tx: oneshot::Sender<Outcome>,
    watchdog: JoinHandle<()>,
    session_id: String,
    tool_use_id: Option<String>,
    event: String,
    tool_name: Option<String>,
    tool_input: Option<Value>,
    deadline: Instant,
}

type Registry = Mutex<HashMap<String, Entry>>;

fn lock_registry(r: &Registry) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
    r.lock().unwrap_or_else(|p| p.into_inner())
}

/// Remove an entry and stop its watchdog.
fn take_entry(r: &Registry, id: &str) -> Option<Entry> {
    let entry = lock_registry(r).remove(id)?;
    entry.watchdog.abort();
    Some(entry)
}

// --------------------------------------------------------------------- ring

/// The 1 000-frame memory ring: pushes never block a connection task; when
/// the consumer falls behind, the oldest frame is dropped (counted).
struct Ring {
    queue: Mutex<VecDeque<Inbound>>,
    capacity: usize,
    notify: Notify,
    metrics: Arc<Metrics>,
}

impl Ring {
    fn push(&self, item: Inbound) {
        {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            q.push_back(item);
            while q.len() > self.capacity {
                if let Some(old) = q.pop_front() {
                    let n = Metrics::bump(&self.metrics.ring_dropped);
                    tracing::warn!(target: "hooks", id = %old.frame.id, event = %old.frame.event, dropped = n, "memory ring full: oldest frame dropped");
                }
            }
        }
        self.notify.notify_one();
    }

    async fn drain(self: Arc<Self>, tx: mpsc::Sender<Inbound>) {
        loop {
            self.notify.notified().await;
            loop {
                let next = self
                    .queue
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .pop_front();
                match next {
                    Some(item) => {
                        if tx.send(item).await.is_err() {
                            return;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

// -------------------------------------------------------------- round trips

/// Synthetic frames from the installer (05 §3.6 S10), by session id.
#[derive(Default)]
struct RoundTrips {
    seen: Mutex<HashMap<String, Instant>>,
    notify: Notify,
}

impl RoundTrips {
    fn record(&self, session_id: &str) {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session_id.to_string(), Instant::now());
        self.notify.notify_waiters();
    }

    async fn wait(&self, session_id: &str, timeout: Duration) -> Option<Instant> {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(t) = self
                .seen
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(session_id)
            {
                return Some(t);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let _ = tokio::time::timeout(remaining, notified).await;
        }
    }
}

// ------------------------------------------------------------------- server

/// Everything a connection task needs.
struct Shared {
    registry: Registry,
    metrics: Arc<Metrics>,
    ring: Arc<Ring>,
    round_trips: RoundTrips,
    default_wait: Duration,
    /// Every resolution of a pending decision, for [`Server::resolutions`].
    outcomes: broadcast::Sender<Resolved>,
}

impl Shared {
    fn announce(&self, entry: &Entry, id: &str, kind: ResolvedKind) {
        let _ = self.outcomes.send(Resolved {
            id: id.to_string(),
            session_id: entry.session_id.clone(),
            tool_use_id: entry.tool_use_id.clone(),
            event: entry.event.clone(),
            tool_name: entry.tool_name.clone(),
            kind,
        });
    }
}

impl Shared {
    /// 08 §5.2: `PostToolUse` for the same tool use and `SessionEnd` close an
    /// open decision ("answered in terminal"). A 2.1.305 `PermissionRequest`
    /// carries no `tool_use_id`, so it is matched by session, tool and input.
    fn supersede(&self, hook: &HookFrame) {
        let payload = &hook.payload;
        let ids: Vec<String> = match hook.event.as_str() {
            events::POST_TOOL_USE | events::POST_TOOL_USE_FAILURE => lock_registry(&self.registry)
                .iter()
                .filter(|(_, e)| e.session_id == payload.session_id)
                .filter(|(_, e)| match (&e.tool_use_id, &payload.tool_use_id) {
                    (Some(a), Some(b)) => a == b,
                    (None, _) => {
                        e.event == events::PERMISSION_REQUEST
                            && e.tool_name == payload.tool_name
                            && e.tool_input.as_ref() == payload.get("tool_input")
                    }
                    (Some(_), None) => false,
                })
                .map(|(id, _)| id.clone())
                .collect(),
            events::SESSION_END => lock_registry(&self.registry)
                .iter()
                .filter(|(_, e)| e.session_id == payload.session_id)
                .map(|(id, _)| id.clone())
                .collect(),
            _ => Vec::new(),
        };
        for id in ids {
            if let Some(entry) = take_entry(&self.registry, &id) {
                Metrics::bump(&self.metrics.superseded);
                tracing::info!(target: "hooks", id = %id, by = %hook.event, "pending decision superseded");
                self.announce(&entry, &id, ResolvedKind::Superseded);
                let _ = entry.tx.send(Outcome::Superseded);
            }
        }
    }

    fn register(
        self: &Arc<Self>,
        hook: &HookFrame,
        received: Instant,
    ) -> oneshot::Receiver<Outcome> {
        let wait = hook
            .decision_wait_ms
            .map(Duration::from_millis)
            .unwrap_or(self.default_wait);
        let margin = if wait > WATCHDOG_MARGIN * 2 {
            WATCHDOG_MARGIN
        } else {
            wait / 2
        };
        let deadline = received + wait - margin;
        let (tx, rx) = oneshot::channel();
        let watchdog = tokio::spawn({
            let shared = Arc::clone(self);
            let id = hook.id.clone();
            async move {
                tokio::time::sleep_until(deadline.into()).await;
                // Not `take_entry`: a task must not abort itself mid-way.
                let entry = lock_registry(&shared.registry).remove(&id);
                if let Some(entry) = entry {
                    Metrics::bump(&shared.metrics.timeouts);
                    tracing::warn!(target: "hooks", id = %id, session = %entry.session_id, "no decision before the relay's deadline: answering no_opinion");
                    shared.announce(&entry, &id, ResolvedKind::Timeout);
                    let _ = entry.tx.send(Outcome::Timeout);
                }
            }
        });
        lock_registry(&self.registry).insert(
            hook.id.clone(),
            Entry {
                tx,
                watchdog,
                session_id: hook.payload.session_id.clone(),
                tool_use_id: hook.payload.tool_use_id.clone(),
                event: hook.event.clone(),
                tool_name: hook.payload.tool_name.clone(),
                tool_input: hook.payload.get("tool_input").cloned(),
                deadline,
            },
        );
        rx
    }
}

/// The running listener. Dropping it stops accepting and (on unix) unlinks the socket.
pub struct Server {
    endpoint: Endpoint,
    shared: Arc<Shared>,
    accept: JoinHandle<()>,
    drain: JoinHandle<()>,
    #[cfg(unix)]
    socket_path: std::path::PathBuf,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("endpoint", &self.endpoint.describe())
            .field("metrics", &self.metrics())
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Bind the endpoint and start accepting. Must be called inside a tokio runtime.
    pub async fn bind(endpoint: Endpoint) -> io::Result<(Server, mpsc::Receiver<Inbound>)> {
        Self::bind_with_capacity(endpoint, DEFAULT_CAPACITY).await
    }

    /// `capacity` is the size of the memory ring.
    pub async fn bind_with_capacity(
        endpoint: Endpoint,
        capacity: usize,
    ) -> io::Result<(Server, mpsc::Receiver<Inbound>)> {
        let metrics = Arc::new(Metrics::default());
        let ring = Arc::new(Ring {
            queue: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            notify: Notify::new(),
            metrics: Arc::clone(&metrics),
        });
        let (outcomes, _) = broadcast::channel(256);
        let shared = Arc::new(Shared {
            registry: Mutex::new(HashMap::new()),
            metrics,
            ring: Arc::clone(&ring),
            round_trips: RoundTrips::default(),
            default_wait: DEFAULT_DECISION_WAIT,
            outcomes,
        });
        let (tx, rx) = mpsc::channel(HANDOFF_CAPACITY);
        let bound = platform::bind(&endpoint)?;
        #[cfg(unix)]
        let socket_path = bound.1.clone();
        let drain = tokio::spawn(ring.drain(tx));
        let accept = tokio::spawn(platform::accept_loop(bound, Arc::clone(&shared)));
        tracing::info!(target: "hooks", endpoint = %endpoint.describe(), ring = capacity, "hook IPC listening");
        Ok((
            Server {
                endpoint,
                shared,
                accept,
                drain,
                #[cfg(unix)]
                socket_path,
            },
            rx,
        ))
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The live counters.
    pub fn metrics(&self) -> MetricsSnapshot {
        self.shared.metrics.snapshot()
    }

    /// Resolve a pending decision with the `output` the relay prints verbatim.
    /// `false` when no relay is waiting on `id` (already answered, canceled or gone).
    pub async fn decide(&self, id: &str, output: Value) -> bool {
        self.resolve(id, Outcome::Decision(output))
    }

    /// [`Server::decide`] from synchronous code (the session engine answers
    /// `always` rules inside the frame it is applying).
    pub fn try_decide(&self, id: &str, output: Value) -> bool {
        self.resolve(id, Outcome::Decision(output))
    }

    /// Tell the relay to exit without output so Claude Code's own prompt appears.
    pub async fn no_opinion(&self, id: &str) -> bool {
        self.resolve(id, Outcome::NoOpinion)
    }

    /// [`Server::no_opinion`] from synchronous code.
    pub fn try_no_opinion(&self, id: &str) -> bool {
        self.resolve(id, Outcome::NoOpinion)
    }

    /// Whether a relay is still waiting on frame `id`.
    pub fn is_pending(&self, id: &str) -> bool {
        lock_registry(&self.shared.registry).contains_key(id)
    }

    /// Every resolution of a pending decision, as it happens: the engine
    /// grays a card on `Timeout` / `Canceled` / `Eof` (05 §7.4).
    pub fn resolutions(&self) -> broadcast::Receiver<Resolved> {
        self.shared.outcomes.subscribe()
    }

    /// [`Server::decide`] by `(session_id, tool_use_id)` (08 §5.2).
    pub async fn decide_tool(&self, session_id: &str, tool_use_id: &str, output: Value) -> bool {
        match self.find_tool(session_id, tool_use_id) {
            Some(id) => self.resolve(&id, Outcome::Decision(output)),
            None => false,
        }
    }

    /// [`Server::no_opinion`] by `(session_id, tool_use_id)`.
    pub async fn no_opinion_tool(&self, session_id: &str, tool_use_id: &str) -> bool {
        match self.find_tool(session_id, tool_use_id) {
            Some(id) => self.resolve(&id, Outcome::NoOpinion),
            None => false,
        }
    }

    fn find_tool(&self, session_id: &str, tool_use_id: &str) -> Option<String> {
        lock_registry(&self.shared.registry)
            .iter()
            .find(|(_, e)| {
                e.session_id == session_id && e.tool_use_id.as_deref() == Some(tool_use_id)
            })
            .map(|(id, _)| id.clone())
    }

    fn resolve(&self, id: &str, outcome: Outcome) -> bool {
        match take_entry(&self.shared.registry, id) {
            Some(entry) => {
                let kind = match &outcome {
                    Outcome::Decision(_) => ResolvedKind::Decision,
                    Outcome::NoOpinion => ResolvedKind::NoOpinion,
                    Outcome::Timeout => ResolvedKind::Timeout,
                    Outcome::Superseded => ResolvedKind::Superseded,
                };
                self.shared.announce(&entry, id, kind);
                entry.tx.send(outcome).is_ok()
            }
            None => false,
        }
    }

    /// Every decision a relay is still waiting on, oldest deadline first.
    pub async fn pending(&self) -> Vec<PendingDecision> {
        let now = Instant::now();
        let mut out: Vec<(Instant, PendingDecision)> = lock_registry(&self.shared.registry)
            .iter()
            .map(|(id, e)| {
                (
                    e.deadline,
                    PendingDecision {
                        id: id.clone(),
                        session_id: e.session_id.clone(),
                        tool_use_id: e.tool_use_id.clone(),
                        event: e.event.clone(),
                        tool_name: e.tool_name.clone(),
                        remaining_ms: e.deadline.saturating_duration_since(now).as_millis() as u64,
                    },
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
        out.into_iter().map(|(_, p)| p).collect()
    }

    /// Frame ids whose relay is still waiting for a decision.
    pub async fn pending_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock_registry(&self.shared.registry)
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// Wait for the synthetic `SessionStart` of an installer round trip
    /// (05 §3.6 S10). Returns when the frame was received: the instant it
    /// arrived, or `None` after `timeout`.
    pub async fn await_round_trip(&self, session_id: &str, timeout: Duration) -> Option<Instant> {
        self.shared.round_trips.wait(session_id, timeout).await
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.accept.abort();
        self.drain.abort();
        for (_, entry) in lock_registry(&self.shared.registry).drain() {
            entry.watchdog.abort();
        }
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

/// One relay connection, from its first frame to EOF.
async fn handle_connection<S>(stream: S, shared: Arc<Shared>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let metrics = Arc::clone(&shared.metrics);
    let (reader, mut writer) = tokio::io::split(stream);
    // A relay sends one hook frame (+ at most one cancel); cap the connection at the frame limit.
    let mut lines =
        BufReader::with_capacity(64 * 1024, reader.take(MAX_FRAME_BYTES as u64 + 64 * 1024))
            .lines();

    loop {
        let frame = match lines.next_line().await {
            Ok(Some(line)) => match Frame::from_line(&line) {
                Ok(f) => f,
                Err(e) => {
                    Metrics::bump(&metrics.malformed);
                    tracing::warn!(target: "hooks", error = %e, bytes = line.len(), "dropping relay connection: malformed frame");
                    return;
                }
            },
            Ok(None) => return,
            Err(e) => {
                tracing::debug!(target: "hooks", error = %e, "relay connection read error");
                return;
            }
        };

        match frame {
            Frame::Hook(hook) => {
                let hook = *hook;
                let id = hook.id.clone();
                let received = Instant::now();
                Metrics::bump(&metrics.frames);
                *metrics.last_frame.lock().unwrap_or_else(|p| p.into_inner()) = Some(received);

                // 1. ack before any other work.
                let Ok(ack) = Frame::Ack(AckFrame::new(&id)).to_line() else {
                    return;
                };
                if writer.write_all(ack.as_bytes()).await.is_err() {
                    return;
                }
                Metrics::bump(&metrics.acks);
                let ack_us = received.elapsed().as_micros() as u64;
                metrics
                    .max_ack_latency_us
                    .fetch_max(ack_us, Ordering::Relaxed);

                // 2. bookkeeping that never touches payload contents.
                let stale = hook.relay_version != EXPECTED_RELAY_VERSION;
                if stale {
                    Metrics::bump(&metrics.stale_relay);
                    tracing::warn!(target: "hooks", id = %id, relay_version = %hook.relay_version, expected = EXPECTED_RELAY_VERSION, "relay version differs: hook install is stale");
                }
                if hook.truncated {
                    Metrics::bump(&metrics.truncated);
                }
                let sizes = redact::sizes(&hook.payload);
                tracing::debug!(
                    target: "hooks",
                    id = %id,
                    event = %hook.event,
                    session = %hook.payload.session_id,
                    tool = hook.payload.tool_name.as_deref().unwrap_or("-"),
                    tool_use_id = hook.payload.tool_use_id.as_deref().unwrap_or("-"),
                    tool_input_bytes = sizes.tool_input,
                    tool_response_bytes = sizes.tool_response,
                    prompt_bytes = sizes.prompt,
                    wants_decision = hook.wants_decision,
                    truncated = hook.truncated,
                    ack_us,
                    "hook frame"
                );

                if hook.payload.is_round_trip() {
                    Metrics::bump(&metrics.round_trips);
                    shared.round_trips.record(&hook.payload.session_id);
                    tracing::info!(target: "hooks", id = %id, session = %hook.payload.session_id, ack_us, "round trip frame");
                    continue;
                }

                // 3. server-side rules, then the registry, then the ring.
                shared.supersede(&hook);
                let waiter = hook
                    .wants_decision
                    .then(|| shared.register(&hook, received));
                shared.ring.push(Inbound {
                    frame: hook,
                    received_at: received,
                    relay_stale: stale,
                });

                // 4. hold the connection until the decision, a cancel or EOF.
                if let Some(mut rx) = waiter {
                    loop {
                        tokio::select! {
                            outcome = &mut rx => {
                                let reply = match outcome {
                                    Ok(Outcome::Decision(output)) => {
                                        Metrics::bump(&metrics.decisions);
                                        Frame::Decision(DecisionFrame::new(&id, output))
                                    }
                                    Ok(Outcome::NoOpinion) => {
                                        Metrics::bump(&metrics.no_opinions);
                                        Frame::NoOpinion(NoOpinionFrame::new(&id))
                                    }
                                    Ok(Outcome::Timeout) | Ok(Outcome::Superseded) => {
                                        Frame::NoOpinion(NoOpinionFrame::new(&id))
                                    }
                                    Err(_) => return,
                                };
                                if let Ok(line) = reply.to_line() {
                                    let _ = writer.write_all(line.as_bytes()).await;
                                    let _ = writer.flush().await;
                                }
                                break;
                            }
                            next = lines.next_line() => match next {
                                Ok(Some(line)) => match Frame::from_line(&line) {
                                    Ok(Frame::Cancel(c)) if c.id == id => {
                                        if let Some(entry) = take_entry(&shared.registry, &id) {
                                            Metrics::bump(&metrics.cancels);
                                            shared.announce(&entry, &id, ResolvedKind::Cancelled);
                                        }
                                        tracing::info!(target: "hooks", id = %id, reason = ?c.reason, "relay canceled");
                                        break;
                                    }
                                    Ok(other) => tracing::debug!(target: "hooks", kind = ?std::mem::discriminant(&other), "ignored frame while a decision is pending"),
                                    Err(e) => {
                                        Metrics::bump(&metrics.malformed);
                                        tracing::warn!(target: "hooks", error = %e, "malformed frame while a decision is pending");
                                    }
                                },
                                _ => {
                                    if let Some(entry) = take_entry(&shared.registry, &id) {
                                        Metrics::bump(&metrics.eof_before_decision);
                                        shared.announce(&entry, &id, ResolvedKind::Eof);
                                        tracing::info!(target: "hooks", id = %id, "relay went away before a decision");
                                    }
                                    return;
                                }
                            }
                        }
                    }
                }
            }
            Frame::Cancel(c) => {
                if let Some(entry) = take_entry(&shared.registry, &c.id) {
                    Metrics::bump(&metrics.cancels);
                    shared.announce(&entry, &c.id, ResolvedKind::Cancelled);
                }
            }
            other => {
                tracing::debug!(target: "hooks", kind = ?std::mem::discriminant(&other), "unexpected frame from a relay")
            }
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::PathBuf;
    use tokio::net::UnixListener;

    pub type Bound = (UnixListener, PathBuf);

    /// Bind the socket: `0700` directory, `0600` socket, stale file replaced,
    /// a live socket (another instance) refused.
    pub fn bind(endpoint: &Endpoint) -> io::Result<Bound> {
        let path = endpoint.socket_path();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)?;
            }
        }
        if path.exists() {
            match std::os::unix::net::UnixStream::connect(&path) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("another Mode instance is listening on {}", path.display()),
                    ));
                }
                Err(_) => {
                    tracing::info!(target: "hooks", path = %path.display(), "removing stale socket");
                    std::fs::remove_file(&path)?;
                }
            }
        }
        let listener = bind_with_backlog(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok((listener, path))
    }

    /// `std` binds with a backlog of 128 already; made explicit so 08 §5.2 holds
    /// whatever the standard library's default becomes.
    fn bind_with_backlog(path: &std::path::Path) -> io::Result<UnixListener> {
        let std_listener = std::os::unix::net::UnixListener::bind(path)?;
        std_listener.set_nonblocking(true)?;
        let _ = BACKLOG;
        UnixListener::from_std(std_listener)
    }

    pub async fn accept_loop((listener, _path): Bound, shared: Arc<Shared>) {
        // SAFETY: getuid has no preconditions and cannot fail.
        let own = Peer::Uid(unsafe { libc::getuid() });
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let peer = stream
                        .peer_cred()
                        .map(|c| Peer::Uid(c.uid()))
                        .map_err(|e| e.to_string());
                    if let Err(e) = check_peer(peer, &own) {
                        Metrics::bump(&shared.metrics.rejected_peers);
                        tracing::warn!(target: "hooks", error = %e, "dropping hook connection");
                        continue;
                    }
                    tokio::spawn(handle_connection(stream, Arc::clone(&shared)));
                }
                Err(e) => {
                    tracing::warn!(target: "hooks", error = %e, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    /// The DACL every pipe instance is created with: the current user's SID only.
    pub struct PipeSecurity {
        descriptor: PSECURITY_DESCRIPTOR,
        attributes: SECURITY_ATTRIBUTES,
    }

    // SAFETY: the descriptor is written once by the system and only read afterwards.
    unsafe impl Send for PipeSecurity {}
    unsafe impl Sync for PipeSecurity {}

    impl PipeSecurity {
        pub fn for_user(sid: &str) -> io::Result<Self> {
            let sddl: Vec<u16> = pipe_sddl(sid)
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            // SAFETY: `sddl` is NUL-terminated; the descriptor is LocalAlloc'd by
            // the system and freed in Drop.
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 || descriptor.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                descriptor,
                attributes: SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor,
                    bInheritHandle: 0,
                },
            })
        }

        fn attributes_ptr(&self) -> *mut c_void {
            std::ptr::from_ref(&self.attributes).cast_mut().cast()
        }
    }

    impl Drop for PipeSecurity {
        fn drop(&mut self) {
            // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
            unsafe {
                LocalFree(self.descriptor);
            }
        }
    }

    pub type Bound = (String, NamedPipeServer, Arc<PipeSecurity>);

    fn create(name: &str, security: &PipeSecurity, first: bool) -> io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options.reject_remote_clients(true);
        if first {
            options.first_pipe_instance(true);
        }
        // SAFETY: the attributes point at a live SECURITY_ATTRIBUTES owned by `security`.
        unsafe { options.create_with_security_attributes_raw(name, security.attributes_ptr()) }
    }

    /// Create the first pipe instance; `first_pipe_instance` fails when another
    /// process already owns the name (a second Mode instance).
    pub fn bind(endpoint: &Endpoint) -> io::Result<Bound> {
        let name = endpoint.pipe_name();
        let sid = mode_wire::sid::current_user_sid().ok_or_else(|| {
            io::Error::other("cannot read the current user's SID for the pipe DACL")
        })?;
        let security = Arc::new(PipeSecurity::for_user(&sid)?);
        let first = create(&name, &security, true)?;
        Ok((name, first, security))
    }

    pub async fn accept_loop((name, mut server, security): Bound, shared: Arc<Shared>) {
        loop {
            if let Err(e) = server.connect().await {
                tracing::warn!(target: "hooks", error = %e, "named pipe connect failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            // Hand the connected instance to its task, then open the next instance.
            let next = loop {
                match create(&name, &security, false) {
                    Ok(s) => break s,
                    Err(e) => {
                        tracing::warn!(target: "hooks", error = %e, "could not create the next pipe instance; retrying");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            };
            let connected = std::mem::replace(&mut server, next);
            tokio::spawn(handle_connection(connected, Arc::clone(&shared)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mode_wire::{
        CancelFrame, CancelReason, HookEvent, PROTOCOL_VERSION, PermissionDecision,
        ROUND_TRIP_SESSION_PREFIX,
    };
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[cfg(unix)]
    type Client = tokio::net::UnixStream;
    #[cfg(windows)]
    type Client = tokio::net::windows::named_pipe::NamedPipeClient;

    #[cfg(unix)]
    fn test_endpoint(dir: &tempfile::TempDir) -> Endpoint {
        Endpoint::at(dir.path().join("ipc.sock").display().to_string())
    }

    #[cfg(windows)]
    fn test_endpoint(_dir: &tempfile::TempDir) -> Endpoint {
        use std::sync::atomic::AtomicU32;
        static N: AtomicU32 = AtomicU32::new(0);
        Endpoint::at(format!(
            r"\\.\pipe\mode-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[cfg(unix)]
    async fn connect(endpoint: &Endpoint) -> Client {
        Client::connect(endpoint.socket_path()).await.unwrap()
    }

    #[cfg(windows)]
    async fn connect(endpoint: &Endpoint) -> Client {
        use tokio::net::windows::named_pipe::ClientOptions;
        loop {
            match ClientOptions::new().open(endpoint.pipe_name()) {
                Ok(c) => return c,
                Err(e) if e.raw_os_error() == Some(231) => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(e) => panic!("open pipe: {e}"),
            }
        }
    }

    fn hook_frame_for(id: &str, session: &str, event: &str, tool: Option<&str>) -> HookFrame {
        let mut payload = HookEvent::new(event, session);
        if let Some(t) = tool {
            payload.set("tool_name", t);
            payload.set("tool_input", serde_json::json!({"command": "npm test"}));
        }
        HookFrame {
            v: PROTOCOL_VERSION,
            id: id.into(),
            event: event.into(),
            relay_version: EXPECTED_RELAY_VERSION.into(),
            wants_decision: payload.wants_decision(),
            sent_at: "2026-10-18T14:02:11.004Z".into(),
            ppid: None,
            tty: None,
            env: Default::default(),
            launch_id: None,
            truncated: false,
            decision_wait_ms: payload.wants_decision().then_some(118_000),
            payload,
        }
    }

    fn hook_frame(id: &str, event: &str, tool: Option<&str>) -> Frame {
        Frame::hook(hook_frame_for(id, "sess-1", event, tool))
    }

    async fn send(w: &mut (impl AsyncWrite + Unpin), f: &Frame) {
        w.write_all(f.to_line().unwrap().as_bytes()).await.unwrap();
        w.flush().await.unwrap();
    }

    type Lines = tokio::io::Lines<BufReader<tokio::io::ReadHalf<Client>>>;

    async fn recv(lines: &mut Lines) -> Option<Frame> {
        tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("frame within 5 s")
            .unwrap()
            .map(|l| Frame::from_line(&l).unwrap())
    }

    async fn open(endpoint: &Endpoint) -> (tokio::io::WriteHalf<Client>, Lines) {
        let client = connect(endpoint).await;
        let (r, w) = tokio::io::split(client);
        (w, BufReader::new(r).lines())
    }

    async fn settle(server: &Server, f: impl Fn(&MetricsSnapshot) -> bool) {
        for _ in 0..100 {
            if f(&server.metrics()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[test]
    fn peer_check_with_fake_credentials() {
        let own = Peer::Uid(1000);
        assert_eq!(check_peer(Ok(Peer::Uid(1000)), &own), Ok(()));
        assert_eq!(
            check_peer(Ok(Peer::Uid(1001)), &own),
            Err(PeerError::Mismatch {
                peer: Peer::Uid(1001),
                own: Peer::Uid(1000)
            })
        );
        assert_eq!(
            check_peer(Err("ENOTSUP".into()), &own),
            Err(PeerError::Unavailable("ENOTSUP".into()))
        );
        let sid = Peer::Sid("S-1-5-21-1-2-3-1001".into());
        assert_eq!(check_peer(Ok(sid.clone()), &sid), Ok(()));
        assert!(check_peer(Ok(Peer::Sid("S-1-5-18".into())), &sid).is_err());
        assert_eq!(
            pipe_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)"
        );
    }

    #[tokio::test]
    async fn acks_within_50ms_delivers_the_frame_and_relays_the_decision() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(&dir);
        let (server, mut inbound) = Server::bind(endpoint.clone()).await.unwrap();
        let (mut w, mut lines) = open(&endpoint).await;

        let sent = Instant::now();
        send(
            &mut w,
            &hook_frame("id-1", "PermissionRequest", Some("Bash")),
        )
        .await;
        let ack = recv(&mut lines).await.unwrap();
        let ack_latency = sent.elapsed();
        assert_eq!(ack, Frame::Ack(AckFrame::new("id-1")));
        assert!(
            ack_latency < Duration::from_millis(50),
            "ack took {ack_latency:?}"
        );

        let got = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.frame.id, "id-1");
        assert_eq!(got.frame.event, "PermissionRequest");
        assert!(got.frame.wants_decision);
        assert!(!got.relay_stale);
        assert_eq!(server.pending_ids().await, vec!["id-1".to_string()]);
        let pending = server.pending().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].session_id, "sess-1");
        assert_eq!(pending[0].tool_name.as_deref(), Some("Bash"));
        assert!(pending[0].remaining_ms > 110_000 && pending[0].remaining_ms <= 118_000 - 750);

        assert!(!server.decide("unknown", Value::Null).await);
        let output = PermissionDecision::allow().output();
        assert!(server.decide("id-1", output.clone()).await);
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::Decision(DecisionFrame::new("id-1", output))
        );
        assert!(server.pending_ids().await.is_empty());
        assert!(
            !server.decide("id-1", Value::Null).await,
            "a decision is delivered once"
        );
        let m = server.metrics();
        assert_eq!((m.frames, m.acks, m.decisions, m.timeouts), (1, 1, 1, 0));
        assert!(m.last_frame_age_ms.is_some());
        assert!(m.max_ack_latency_us < 50_000);
    }

    #[tokio::test]
    async fn no_opinion_is_relayed_and_cancel_clears_pending() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(&dir);
        let (server, mut inbound) = Server::bind(endpoint.clone()).await.unwrap();
        let (mut w, mut lines) = open(&endpoint).await;
        send(
            &mut w,
            &hook_frame("id-2", "PreToolUse", Some("AskUserQuestion")),
        )
        .await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert!(server.no_opinion("id-2").await);
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::NoOpinion(NoOpinionFrame::new("id-2"))
        );

        let (mut w2, mut lines2) = open(&endpoint).await;
        send(
            &mut w2,
            &hook_frame("id-3", "PermissionRequest", Some("Bash")),
        )
        .await;
        assert!(matches!(recv(&mut lines2).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(server.pending_ids().await, vec!["id-3".to_string()]);
        send(
            &mut w2,
            &Frame::Cancel(CancelFrame::new("id-3", CancelReason::Deadline)),
        )
        .await;
        settle(&server, |m| m.cancels == 1).await;
        assert!(
            server.pending_ids().await.is_empty(),
            "cancel removes the pending decision"
        );
        assert!(
            !server.decide("id-3", Value::Null).await,
            "a decision after cancel is ignored"
        );
        let m = server.metrics();
        assert_eq!((m.no_opinions, m.cancels), (1, 1));
    }

    #[tokio::test]
    async fn non_decision_frames_and_eof_leave_nothing_pending() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(&dir);
        let (server, mut inbound) = Server::bind(endpoint.clone()).await.unwrap();
        let (mut w, mut lines) = open(&endpoint).await;
        send(&mut w, &hook_frame("id-4", "SessionStart", None)).await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        let got = inbound.recv().await.unwrap();
        assert!(!got.frame.wants_decision);
        assert!(server.pending_ids().await.is_empty());

        // A decision event whose relay disappears (EOF) is dropped from the registry.
        send(
            &mut w,
            &hook_frame("id-5", "PermissionRequest", Some("Bash")),
        )
        .await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(server.pending_ids().await, vec!["id-5".to_string()]);
        drop(w);
        drop(lines);
        settle(&server, |m| m.eof_before_decision == 1).await;
        assert!(
            server.pending_ids().await.is_empty(),
            "EOF clears the pending decision"
        );
    }

    #[tokio::test]
    async fn watchdog_answers_no_opinion_just_before_the_relay_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(&dir);
        let (server, mut inbound) = Server::bind(endpoint.clone()).await.unwrap();
        let (mut w, mut lines) = open(&endpoint).await;
        let mut frame = hook_frame_for("id-6", "sess-1", "PermissionRequest", Some("Bash"));
        frame.decision_wait_ms = Some(1_500); // relay gives up at 1.5 s → watchdog at 0.75 s
        let started = Instant::now();
        send(&mut w, &Frame::hook(frame)).await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(
            server.pending().await[0].remaining_ms.clamp(500, 750),
            server.pending().await[0].remaining_ms
        );
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::NoOpinion(NoOpinionFrame::new("id-6"))
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(700) && elapsed < Duration::from_millis(1_400),
            "watchdog fired at {elapsed:?}"
        );
        assert!(server.pending_ids().await.is_empty());
        assert_eq!(server.metrics().timeouts, 1);
        assert!(
            !server.decide("id-6", Value::Null).await,
            "nothing left to decide"
        );

        // A very short wait still fires (half the wait rather than a negative margin).
        let mut frame = hook_frame_for("id-7", "sess-1", "PermissionRequest", Some("Bash"));
        frame.decision_wait_ms = Some(400);
        send(&mut w, &Frame::hook(frame)).await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::NoOpinion(NoOpinionFrame::new("id-7"))
        );
        assert_eq!(server.metrics().timeouts, 2);
    }

    #[tokio::test]
    async fn decisions_by_tool_use_id_and_superseding_frames() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(&dir);
        let (server, mut inbound) = Server::bind(endpoint.clone()).await.unwrap();

        // A question keyed by tool_use_id, answered through (session, tool_use_id).
        let (mut w, mut lines) = open(&endpoint).await;
        let mut q = hook_frame_for("id-8", "sess-2", "PreToolUse", Some("AskUserQuestion"));
        q.payload.set("tool_use_id", "toolu_q");
        send(&mut w, &Frame::hook(q)).await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert!(
            !server
                .decide_tool("sess-2", "toolu_other", Value::Null)
                .await
        );
        assert!(!server.no_opinion_tool("sess-9", "toolu_q").await);
        assert!(
            server
                .decide_tool("sess-2", "toolu_q", serde_json::json!({"ok": true}))
                .await
        );
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::Decision(DecisionFrame::new("id-8", serde_json::json!({"ok": true})))
        );

        // A question answered in the terminal: PostToolUse for the tool use closes it.
        let (mut w, mut lines) = open(&endpoint).await;
        let mut q = hook_frame_for("id-9", "sess-2", "PreToolUse", Some("AskUserQuestion"));
        q.payload.set("tool_use_id", "toolu_q2");
        send(&mut w, &Frame::hook(q)).await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        let (mut w2, mut lines2) = open(&endpoint).await;
        let mut post = hook_frame_for("id-10", "sess-2", "PostToolUse", Some("AskUserQuestion"));
        post.payload.set("tool_use_id", "toolu_q2");
        send(&mut w2, &Frame::hook(post)).await;
        assert!(matches!(recv(&mut lines2).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::NoOpinion(NoOpinionFrame::new("id-9"))
        );
        assert_eq!(server.metrics().superseded, 1);

        // A 2.1.305 PermissionRequest (no tool_use_id) answered in the terminal:
        // the PostToolUse of the same tool and input closes it; a different input does not.
        let (mut w, mut lines) = open(&endpoint).await;
        send(
            &mut w,
            &hook_frame("id-11", "PermissionRequest", Some("Bash")),
        )
        .await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        let (mut w3, mut lines3) = open(&endpoint).await;
        let mut other = hook_frame_for("id-12", "sess-1", "PostToolUse", Some("Bash"));
        other
            .payload
            .set("tool_input", serde_json::json!({"command": "ls"}));
        send(&mut w3, &Frame::hook(other)).await;
        assert!(matches!(recv(&mut lines3).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(
            server.pending_ids().await,
            vec!["id-11".to_string()],
            "different input: still open"
        );
        let mut same = hook_frame_for("id-13", "sess-1", "PostToolUse", Some("Bash"));
        same.payload.set("tool_use_id", "toolu_x");
        send(&mut w3, &Frame::hook(same)).await;
        assert!(matches!(recv(&mut lines3).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::NoOpinion(NoOpinionFrame::new("id-11"))
        );

        // SessionEnd closes every open decision of the session.
        let (mut w, mut lines) = open(&endpoint).await;
        send(
            &mut w,
            &hook_frame("id-14", "PermissionRequest", Some("Bash")),
        )
        .await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        send(&mut w3, &hook_frame("id-15", "SessionEnd", None)).await;
        assert!(matches!(recv(&mut lines3).await, Some(Frame::Ack(_))));
        inbound.recv().await.unwrap();
        assert_eq!(
            recv(&mut lines).await.unwrap(),
            Frame::NoOpinion(NoOpinionFrame::new("id-14"))
        );
        assert_eq!(server.metrics().superseded, 3);
        assert!(server.pending_ids().await.is_empty());
    }

    #[tokio::test]
    async fn ring_drops_the_oldest_frames_when_the_consumer_is_slow() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(&dir);
        let (server, mut inbound) = Server::bind_with_capacity(endpoint.clone(), 3)
            .await
            .unwrap();
        let (mut w, mut lines) = open(&endpoint).await;
        for i in 0..10 {
            send(&mut w, &hook_frame(&format!("id-{i:02}"), "Stop", None)).await;
            assert!(
                matches!(recv(&mut lines).await, Some(Frame::Ack(_))),
                "every frame is acked"
            );
        }
        settle(&server, |m| m.frames == 10 && m.ring_dropped >= 5).await;
        let mut received = Vec::new();
        while let Ok(Some(i)) =
            tokio::time::timeout(Duration::from_millis(200), inbound.recv()).await
        {
            received.push(i.frame.id);
        }
        let m = server.metrics();
        assert_eq!(
            received.len() as u64 + m.ring_dropped,
            10,
            "{received:?} + {}",
            m.ring_dropped
        );
        assert!(m.ring_dropped >= 5, "{}", m.ring_dropped);
        assert_eq!(
            received.last().map(String::as_str),
            Some("id-09"),
            "the newest survives"
        );
        assert!(
            received.windows(2).all(|p| p[0] < p[1]),
            "order kept: {received:?}"
        );
    }

    #[tokio::test]
    async fn round_trip_frames_are_recorded_not_forwarded_and_stale_relays_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(&dir);
        let (server, mut inbound) = Server::bind(endpoint.clone()).await.unwrap();
        let (mut w, mut lines) = open(&endpoint).await;
        let session = format!("{ROUND_TRIP_SESSION_PREFIX}abc");
        let waiter = {
            let session = session.clone();
            let shared = Arc::clone(&server.shared);
            tokio::spawn(async move {
                shared
                    .round_trips
                    .wait(&session, Duration::from_secs(5))
                    .await
            })
        };
        let mut synthetic = hook_frame_for("id-rt", &session, "SessionStart", None);
        synthetic.relay_version = "0.0.9".into();
        send(&mut w, &Frame::hook(synthetic)).await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        assert!(waiter.await.unwrap().is_some(), "the round trip was seen");
        assert!(
            server
                .await_round_trip(&session, Duration::from_millis(50))
                .await
                .is_none(),
            "consumed once"
        );
        assert!(
            server
                .await_round_trip("never", Duration::from_millis(50))
                .await
                .is_none()
        );

        send(&mut w, &hook_frame("id-real", "Stop", None)).await;
        assert!(matches!(recv(&mut lines).await, Some(Frame::Ack(_))));
        let got = inbound.recv().await.unwrap();
        assert_eq!(
            got.frame.id, "id-real",
            "the synthetic frame never reaches the consumer"
        );
        let m = server.metrics();
        assert_eq!((m.frames, m.round_trips, m.stale_relay), (2, 1, 1));

        // Malformed input drops the connection and is counted.
        w.write_all(b"not a frame\n").await.unwrap();
        w.flush().await.unwrap();
        settle(&server, |m| m.malformed == 1).await;
        assert_eq!(server.metrics().malformed, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stale_socket_is_replaced_and_live_socket_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(
            dir.path()
                .join("sub")
                .join("ipc.sock")
                .display()
                .to_string(),
        );
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(endpoint.socket_path(), b"stale").unwrap();
        let (server, _rx) = Server::bind(endpoint.clone()).await.unwrap();
        let mode = std::fs::metadata(endpoint.socket_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let second = Server::bind(endpoint.clone()).await;
        assert!(
            matches!(&second, Err(e) if e.kind() == io::ErrorKind::AddrInUse),
            "{second:?}"
        );
        drop(server);
        assert!(!endpoint.socket_path().exists(), "drop unlinks the socket");

        // A missing parent directory is created 0700.
        let nested = Endpoint::at(
            dir.path()
                .join("new")
                .join("deep")
                .join("ipc.sock")
                .display()
                .to_string(),
        );
        let (server, _rx) = Server::bind(nested.clone()).await.unwrap();
        let dir_mode = std::fs::metadata(dir.path().join("new").join("deep"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        drop(server);
    }
}
