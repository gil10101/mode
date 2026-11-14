// The capture box (02 §7.9): the last element of `expanded`, the only text
// field in the overlay and the only moment it holds key input. Keys are
// handled by the root (the machine), this draws the box and its states.
import type { RefObject } from 'react'
import { Chip } from '../kit/controls'

export interface CaptureBoxProps {
  inputRef: RefObject<HTMLInputElement | null>
  value: string
  onChange: (value: string) => void
  onFocus: () => void
  onBlur: () => void
  /** Shows "Saved · Open" instead of the input. */
  saved: boolean
  onOpen: () => void
  /** "linked to …" chip (02 §7.9 context link). */
  linkedTo?: string | null
  onUnlink?: () => void
  /** The vault is missing: capture is disabled and the box says so (02 §14). */
  disabledReason?: string | null
  /** A drag is in progress: the drop target replaces the box (02 §8.5). */
  dropTarget?: boolean
}

export function CaptureBox({ inputRef, value, onChange, onFocus, onBlur, saved, onOpen, linkedTo, onUnlink, disabledReason, dropTarget }: CaptureBoxProps) {
  let body
  if (dropTarget) {
    body = (
      <div className="m-input is-focused" style={{ justifyContent: 'center' }}>
        <span className="m-placeholder">Drop to capture</span>
      </div>
    )
  } else if (saved) {
    body = (
      <div className="m-input" data-testid="capture-saved">
        <span className="m-capture-saved">
          {'Saved · '}
          <a
            href="#"
            onClick={(e) => {
              e.preventDefault()
              onOpen()
            }}
          >
            Open
          </a>
        </span>
      </div>
    )
  } else if (disabledReason) {
    body = (
      <div className="m-input" aria-disabled="true">
        <span className="m-placeholder">{disabledReason}</span>
      </div>
    )
  } else {
    body = (
      <input
        ref={inputRef}
        className="m-input"
        placeholder="Capture a note…"
        aria-label="Capture a note"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={onFocus}
        onBlur={onBlur}
        data-testid="capture-input"
      />
    )
  }
  return (
    <div className="m-capture">
      {linkedTo && (
        <Chip removable onRemove={onUnlink}>
          {`linked to ${linkedTo}`}
        </Chip>
      )}
      {body}
    </div>
  )
}
