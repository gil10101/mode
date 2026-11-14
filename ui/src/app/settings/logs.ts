// `logs/mode.log` lines as the Logs section shows them (03 §8.17): the
// tracing-subscriber text format (`<RFC 3339>  LEVEL target: message`) split
// into time, level, component and message; anything else is kept verbatim
// as an `info` line of the `core` component.

export type LogLevel = 'error' | 'warn' | 'info' | 'debug'

export const LOG_LEVELS: readonly LogLevel[] = ['error', 'warn', 'info', 'debug']

export const LOG_COMPONENTS = ['core', 'hooks', 'indexer', 'connectors', 'urgency', 'ai', 'ui'] as const
export type LogComponent = (typeof LOG_COMPONENTS)[number]

export interface LogLine {
  raw: string
  at: string | null
  level: LogLevel
  component: LogComponent
  target: string | null
  message: string
}

// `mode_core::diag`'s file format (tracing-subscriber's text format without colors): the time, the level
// right-aligned in five columns, the open spans (`name{fields}:`), the target, the message and its fields.
const LINE = /^(\d{4}-\d{2}-\d{2}T[\d:.]+(?:Z|[+-]\d{2}:?\d{2})?)\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s+(?:[^\s{}:]+\{[^}]*\}:+\s*)*([A-Za-z0-9_:]+?):?\s+(.*)$/

/** Which of the seven components a tracing target belongs to. */
export function componentOf(target: string): LogComponent {
  const t = target.toLowerCase()
  if (t.includes('hook')) return 'hooks'
  if (t.includes('notes') || t.includes('index') || t.includes('vault')) return 'indexer'
  if (t.includes('connector') || t.includes('gmail') || t.includes('slack') || t.includes('calendar') || t.includes('oauth')) return 'connectors'
  if (t.includes('urgency')) return 'urgency'
  if (/(^|::)ai(::|$)/.test(t)) return 'ai'
  if (t.includes('overlay') || t.includes('app_window') || t.includes('webview')) return 'ui'
  return 'core'
}

export function parseLogLine(raw: string): LogLine {
  const m = LINE.exec(raw)
  if (!m) return { raw, at: null, level: 'info', component: 'core', target: null, message: raw }
  const level = m[2] === 'TRACE' ? 'debug' : (m[2]!.toLowerCase() as LogLevel)
  return { raw, at: m[1]!, level, component: componentOf(m[3]!), target: m[3]!, message: m[4]! }
}

export interface LogFilter {
  /** The most verbose level shown (`info` shows error, warn and info). */
  level: LogLevel
  component: LogComponent | 'all'
  text: string
}

export function filterLogs(lines: readonly LogLine[], f: LogFilter): LogLine[] {
  const max = LOG_LEVELS.indexOf(f.level)
  const text = f.text.trim().toLowerCase()
  return lines.filter((l) => LOG_LEVELS.indexOf(l.level) <= max && (f.component === 'all' || l.component === f.component) && (!text || l.raw.toLowerCase().includes(text)))
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`
}
