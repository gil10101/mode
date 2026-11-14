// The two byte streams of a Terminal tab: PTY output joined with the
// scrollback replay (`pty_open`) by stream offset, and keystrokes queued so
// `pty_write` calls reach the host one at a time and in order.
import { bytesToBase64, concatBytes } from './base64'

export interface OutputChunk {
  data: Uint8Array
  /** Stream offset of the first byte; absent = unknown (written as is). */
  offset?: number
}

/**
 * Joins live `pty.output` chunks with the replay. Chunks that arrive before
 * the replay are held; afterwards each chunk is trimmed by what has already
 * been written, so the attach neither loses nor doubles bytes.
 */
export class OutputJoin {
  /** Stream offset just past the last written byte; null until the replay is in. */
  private end: number | null = null
  private early: OutputChunk[] = []

  get ready(): boolean {
    return this.end !== null
  }

  /** The replay arrived: returns the byte runs to write, replay first. */
  replay(data: Uint8Array, offset: number): Uint8Array[] {
    this.end = offset
    const out: Uint8Array[] = []
    const first = this.take({ data, offset })
    if (first) out.push(first)
    for (const c of this.early) {
      const b = this.take(c)
      if (b) out.push(b)
    }
    this.early = []
    return out
  }

  /** A live chunk: the bytes to write now (null = nothing new, or held until the replay). */
  push(chunk: OutputChunk): Uint8Array | null {
    if (this.end === null) {
      this.early.push(chunk)
      return null
    }
    return this.take(chunk)
  }

  private take(c: OutputChunk): Uint8Array | null {
    if (!c.data.length) return null
    if (c.offset === undefined || this.end === null) return c.data
    const skip = this.end - c.offset
    if (skip >= c.data.length) return null
    const bytes = skip > 0 ? c.data.subarray(skip) : c.data
    // A gap (skip < 0) means bytes were lost upstream; write what we have and move on.
    this.end = c.offset + c.data.length
    return bytes
  }
}

export interface WriteQueue {
  push(bytes: Uint8Array): void
  /** Resolves once everything queued so far has been sent. */
  flushed(): Promise<void>
  dispose(): void
}

/**
 * Serializes keystrokes: one `send` in flight at a time; bytes typed while
 * it is in flight are coalesced into the next call (order is kept, latency
 * stays one round trip).
 */
export function createWriteQueue(send: (b64: string) => Promise<unknown>, onError?: (e: unknown) => void): WriteQueue {
  let queue: Uint8Array[] = []
  let running: Promise<void> | null = null
  let disposed = false

  const pump = async () => {
    while (queue.length && !disposed) {
      const bytes = concatBytes(queue)
      queue = []
      try {
        await send(bytesToBase64(bytes))
      } catch (e) {
        onError?.(e)
      }
    }
    running = null
  }

  return {
    push(bytes) {
      if (disposed || !bytes.length) return
      queue.push(bytes)
      if (!running) running = pump()
    },
    flushed() {
      return running ?? Promise.resolve()
    },
    dispose() {
      disposed = true
      queue = []
    },
  }
}
