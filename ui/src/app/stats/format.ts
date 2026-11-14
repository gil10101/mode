// Figures of the Stats view (03 §7A): spend in dollars (the one place currency
// appears, ADR-0051), counts with thousands separators, shares, durations,
// local hours and short model names.

/** `$0.42`, `$38.20`, `$1,204` (cents under a thousand, whole dollars above); `—` when unpriced. */
export function money(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—'
  const abs = Math.abs(n)
  if (abs < 1000) return `$${n.toFixed(2)}`
  return `$${Math.round(n).toLocaleString('en-US')}`
}

/** `1,284`. */
export function count(n: number): string {
  return Math.round(n).toLocaleString('en-US')
}

/** `92%` of a 0 ..= 1 share. */
export function pct(x: number | null | undefined): string {
  if (x === null || x === undefined || !Number.isFinite(x)) return '—'
  return `${Math.round(x * 100)}%`
}

/** `5h 12m`, `48m`, `30s`. */
export function duration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds))
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h ${m % 60}m`
  const d = Math.floor(h / 24)
  return `${d}d ${h % 24}h`
}

/** `15:00` for a local hour. */
export function hourLabel(h: number): string {
  return `${String(h).padStart(2, '0')}:00`
}

/** `opus-5-5` from `claude-opus-5-5-20260101`: the family without the vendor prefix and the release date. */
export function shortModel(id: string): string {
  let m = id.trim()
  for (const p of ['anthropic.', 'us.', 'eu.', 'global.', 'apac.']) if (m.startsWith(p)) m = m.slice(p.length)
  m = m.replace(/-v\d+:\d+$/, '').replace(/@\d{8}$/, '').replace(/-\d{8}$/, '')
  return m.replace(/^claude-/, '')
}

/** `Claude Code` / `Mode` / `Ollama` / `LM Studio` for a `usage_samples.source`. */
export function sourceLabel(source: string): string {
  switch (source) {
    case 'claude_code':
      return 'Claude Code'
    case 'claude_api':
    case 'openai_compat':
      return 'Mode'
    case 'ollama':
      return 'Ollama'
    case 'lm_studio':
      return 'LM Studio'
    default:
      return source
  }
}
