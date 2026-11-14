// The Terminal layer and prompt delivery contract (05 §8, §8.5, §14; 03 §3.7,
// §3.8), as `mode_core::pty::api` serializes it (app crate:
// `commands_pty.rs`, app window only). Field names are snake_case, as in
// contract.ts. Every command rejects with `CmdError` (contract.ts); before
// the PTY host starts they answer `{ code: 'unavailable' }`.
//
// Framing: every byte that crosses the bridge is standard base64 (RFC 4648,
// with padding) of raw bytes, never re-encoded text. PTY output is the
// master side's bytes exactly as read (a chunk may split a UTF-8 sequence;
// the emulator joins them). Keystrokes are the emulator's bytes (UTF-8 for
// typed text, raw bytes for binary mouse reports).
//
// Ordering: `offset` is the stream position (bytes since the PTY spawned) of
// a chunk's first byte. The UI subscribes to `pty.output` before it calls
// `pty_open`, writes the replay, then drops whatever part of a live chunk the
// replay already covered, so nothing is lost or doubled across the attach.

/* ---- events: core → app window ------------------------------------------ */

export const TERMINAL_EVENTS = {
  /** Bytes from the PTY of an embedded session. */
  output: 'pty.output',
  /** A PTY was spawned (launch, resume, "Open a terminal") with its initial size. */
  started: 'pty.started',
  /** The `claude` process in the PTY exited; the PTY is gone (driver `pty` → `none`). */
  exited: 'pty.exited',
  /** A headless `stream-json` turn started (driver `none` → `headless`, 05 §8.5). */
  turnStarted: 'turn.started',
  /** The headless turn ended: `result` arrived, or SIGINT stopped it (driver `headless` → `none`). */
  turnFinished: 'turn.finished',
  /** Streamed assistant text of a headless turn (`{ session_id, turn_id, text, parent_tool_use_id? }`). */
  turnDelta: 'turn.delta',
} as const

export interface PtyOutput {
  session_id: string
  /** base64 of the raw bytes. */
  data: string
  /** Stream offset of the first byte; optional (without it the chunk is written as is). */
  offset?: number
}

export interface PtyStarted {
  session_id: string
  cols: number
  rows: number
}

export interface PtyExited {
  session_id: string
  /** Exit status; `null` when killed by a signal. */
  code: number | null
}

export interface TurnStarted {
  session_id: string
  turn_id: string
}

export type TurnOutcome = 'finished' | 'stopped' | 'failed'

export interface TurnFinished {
  session_id: string
  turn_id: string
  outcome: TurnOutcome
}

/* ---- commands: app window → core ---------------------------------------- */

export interface PtyOpenArgs {
  session_id: string
  /** The Terminal tab's size; the host resizes the PTY to it before replaying. */
  cols: number
  rows: number
}

/**
 * `pty_open`: attach to the session's live PTY and replay its scrollback
 * buffer. Rejects with `{ code, message }` when the session has no PTY.
 */
export interface PtyOpenResult {
  /** The size the PTY now has. */
  cols: number
  rows: number
  /** base64 of the buffered bytes to replay (may be empty). */
  data: string
  /** Stream offset of the first replayed byte. */
  offset: number
  /** The process already exited (the buffer is kept until the row is dismissed); `exit_code` says how. */
  exited?: boolean
  exit_code?: number | null
}

/** A session with a live process (`pty_list`; the quit confirmation's list, 03 §2.1). */
export interface LiveSession {
  session_id: string
  display_name: string
  /** `pty` | `headless`. */
  driver: 'pty' | 'headless' | string
  state: string
}

/** Where `claude` resolves to now (`claude_locate`). */
export interface ClaudeLocation {
  found: boolean
  path?: string
  /** `setting` | `path` | `where` | `fallback` | `config`. */
  source?: string
  message?: string
}

/** A `session_prompt` refusal (05 §14): `{ code: 'refused', message, action? }`; `action` names the button to offer. */
export interface PromptRefusal {
  code: string
  message: string
  action?: 'jump' | 'resume'
}

/** How `session_prompt` delivered the text (05 §14): PTY stdin through bracketed paste, or a headless turn. */
export interface PromptDelivered {
  delivered_via: 'pty' | 'headless'
  /** Set for a headless turn (`turn.started` / `turn.finished` carry the same id). */
  turn_id?: string
}

export interface TerminalCommands {
  pty_open: { args: PtyOpenArgs; result: PtyOpenResult }
  /** Keystrokes and pastes; `data` is base64. Writes are sent one at a time, in order. */
  pty_write: { args: { session_id: string; data: string }; result: void }
  /** Debounced 100 ms (10 §13); the host forwards `TIOCSWINSZ` / `ResizePseudoConsole`. */
  pty_resize: { args: { session_id: string; cols: number; rows: number }; result: void }
  /** "Close terminal": SIGINT, wait for the turn to end, then detach; `pty.exited` follows. Never SIGTERM (05 §8.3). */
  pty_close: { args: { session_id: string }; result: void }
  /** Stop receiving `pty.output` for this session (the Terminal tab unmounted); the PTY and its buffer stay. */
  pty_detach: { args: { session_id: string }; result: void }
  /**
   * The prompt box. Refused (`{ code: 'refused', message, action? }`; the UI shows `message`) when
   * the box would be disabled (05 §14): a card is open, a turn is running,
   * Claude is working, the session is external, untracked or failed.
   */
  session_prompt: { args: { session_id: string; text: string }; result: PromptDelivered }
  /** "Stop turn" while a headless turn runs: SIGINT; `turn.finished` with `stopped` follows. */
  turn_stop: { args: { session_id: string }; result: void }
  /** The sessions a quit would interrupt. */
  pty_list: { args: undefined; result: LiveSession[] }
  /** SIGINT every live PTY and headless turn and wait (`wait_ms`, 10 s by default). */
  pty_quit_all: { args: { wait_ms?: number } | undefined; result: { signalled: string[]; remaining: string[] } }
  claude_locate: { args: undefined; result: ClaudeLocation }
}

/** Kept with the PTY by the host and by the emulator (03 §3.8). */
export const SCROLLBACK_LINES = 10_000
