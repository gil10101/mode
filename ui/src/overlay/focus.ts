// The focus rule of 02 §9: nothing in the overlay can take focus except the
// capture box. After every render each focusable element is set to
// tabindex -1; mousedown outside the capture box is prevented so buttons never
// focus on click, and any stray focus is blurred.
import { useLayoutEffect, type FocusEvent, type MouseEvent, type RefObject } from 'react'

const FOCUSABLE = 'a,button,input,select,textarea,[tabindex]'

export function useFocusGuard(root: RefObject<HTMLElement | null>, capture: RefObject<HTMLInputElement | null>) {
  useLayoutEffect(() => {
    const el = root.current
    if (!el) return
    el.querySelectorAll<HTMLElement>(FOCUSABLE).forEach((node) => {
      if (node !== capture.current) node.tabIndex = -1
    })
  })
  return {
    onMouseDownCapture(e: MouseEvent) {
      const target = e.target as Node
      if (!capture.current || !capture.current.contains(target)) e.preventDefault()
    },
    onFocusCapture(e: FocusEvent) {
      if (e.target !== capture.current) (e.target as HTMLElement).blur?.()
    },
  }
}

/** The elements of `root` that can still take focus (tests). */
export function focusable(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((n) => n.tabIndex >= 0 && !(n as HTMLButtonElement).disabled)
}
