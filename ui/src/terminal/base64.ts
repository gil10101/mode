// The byte framing of the PTY bridge (contract-terminal.ts): standard base64
// of raw bytes in both directions. Typed text is UTF-8; xterm's binary
// strings (`onBinary`, one char per byte) are taken byte for byte.

const CHUNK = 0x8000

/** Raw bytes → base64. Chunked so large pastes do not overflow the argument list. */
export function bytesToBase64(bytes: Uint8Array): string {
  let bin = ''
  for (let i = 0; i < bytes.length; i += CHUNK) bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK))
  return btoa(bin)
}

/** base64 → raw bytes; whitespace is ignored, anything else invalid throws. */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64.replace(/\s+/g, ''))
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

const encoder = new TextEncoder()
const decoder = new TextDecoder()

/** Typed text (a JS string) → UTF-8 bytes. */
export function textToBytes(text: string): Uint8Array {
  return encoder.encode(text)
}

/** A binary string (each char code 0–255 is one byte) → bytes. */
export function binaryToBytes(bin: string): Uint8Array {
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i) & 0xff
  return out
}

/** UTF-8 bytes → text (invalid sequences become U+FFFD). */
export function bytesToText(bytes: Uint8Array): string {
  return decoder.decode(bytes)
}

export function textToBase64(text: string): string {
  return bytesToBase64(textToBytes(text))
}

export function base64ToText(b64: string): string {
  return bytesToText(base64ToBytes(b64))
}

export function concatBytes(chunks: readonly Uint8Array[]): Uint8Array {
  if (chunks.length === 1) return chunks[0]
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0))
  let at = 0
  for (const c of chunks) {
    out.set(c, at)
    at += c.length
  }
  return out
}
