// Clipboard and links for the Terminal tab (05 §8.3): OSC 52 writes are
// allowed, reads are refused; pasted text cannot end bracketed paste early;
// OSC 8 hyperlinks and plain URLs open through the opener, http(s) / mailto /
// file only.
import { base64ToText } from './base64'

/** Largest OSC 52 payload accepted (decoded bytes); larger writes are dropped. */
export const OSC52_MAX_BYTES = 1 << 20

export type Osc52 = { kind: 'write'; text: string } | { kind: 'read' } | { kind: 'ignore' }

/** `Pc ; Pd` — selection targets, then base64 text or `?` (a read request). */
export function parseOsc52(data: string): Osc52 {
  const i = data.indexOf(';')
  if (i < 0) return { kind: 'ignore' }
  const payload = data.slice(i + 1)
  if (payload === '?') return { kind: 'read' }
  if (!payload || payload.length > Math.ceil((OSC52_MAX_BYTES * 4) / 3) + 4) return { kind: 'ignore' }
  try {
    return { kind: 'write', text: base64ToText(payload) }
  } catch {
    return { kind: 'ignore' }
  }
}

/**
 * Pasted text without ESC: a paste carrying `ESC [201~` would end bracketed
 * paste early and run the rest as typed keys. Null when nothing changes.
 */
export function sanitizePaste(text: string): string | null {
  return text.includes('\x1b') ? text.replace(/\x1b/g, '') : null
}

export type LinkTarget = { kind: 'url'; url: string } | { kind: 'path'; path: string } | { kind: 'refused' }

/** What activating a terminal link does: open a URL, open a file, or nothing (javascript:, data:, …). */
export function linkTarget(uri: string): LinkTarget {
  let u: URL
  try {
    u = new URL(uri.trim())
  } catch {
    return { kind: 'refused' }
  }
  if (u.protocol === 'http:' || u.protocol === 'https:' || u.protocol === 'mailto:') return { kind: 'url', url: u.href }
  if (u.protocol === 'file:') {
    let path = decodeURIComponent(u.pathname)
    // file:///C:/x → C:/x on Windows paths.
    if (/^\/[A-Za-z]:\//.test(path)) path = path.slice(1)
    return path ? { kind: 'path', path } : { kind: 'refused' }
  }
  return { kind: 'refused' }
}
