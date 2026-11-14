// `peek` and `compact` (02 §5, §6): the mascot tab, and the pill with its
// phrase, count, one inline action and ⋮, drawn per placement.
import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { Mascot } from '../kit/avatars'
import { Button, MenuButton } from '../kit/button'
import { Diff } from '../kit/lists'
import { joinDot } from '../kit/text'
import { Peek, Pill } from '../kit/overlay'
import { Spinner } from '../kit/primitives'
import type { SessionState } from '../kit/types'
import { useOverlay } from './context'
import { NotchBot } from './NotchBot'
import { PeekNowPlaying } from './NowPlaying'
import type { PillModel } from './pill'

/** Drops trailing phrase parts while the rendered text overflows; the first two parts always stay (02 §5.2). */
export function useFitParts(parts: string[]) {
  const ref = useRef<HTMLSpanElement | null>(null)
  const key = parts.join('\u0001')
  const [fit, setFit] = useState<{ key: string; visible: number }>({ key, visible: parts.length })
  const visible = fit.key === key ? fit.visible : parts.length
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    if (el.scrollWidth > el.clientWidth + 1 && visible > 2) setFit({ key, visible: visible - 1 })
  })
  return { ref, visible }
}

export function PhraseParts({ parts, diff, visible }: { parts: string[]; diff?: PillModel['diff']; visible: number }) {
  const shown = parts.slice(0, visible)
  return <>{joinDot(...shown.map((p, i): ReactNode => (diff && i === diff.at ? <Diff key={i} add={diff.add} del={diff.del} /> : p)))}</>
}

function dotColor(badge: SessionState | undefined): string {
  if (badge === 'waiting') return 'var(--m-accent)'
  if (badge === 'failed') return 'var(--m-bad)'
  return 'var(--m-fg-3)'
}

/** The mascot with its badge, or the 6-px state dot when `overlay.show_mascot` is off. */
export function MascotOrDot({ size = 18 }: { size?: 18 | 20 }) {
  const { badge, showMascot } = useOverlay()
  if (!showMascot) return <span className="m-dot" style={{ background: dotColor(badge) }} aria-hidden="true" />
  return <Mascot size={size} badge={badge} alt="" />
}

/** The peek's mascot: the gaze bot (02 §6.1), or the state dot when `overlay.show_mascot` is off. */
function PeekMascot() {
  const { badge, showMascot, m, botRef } = useOverlay()
  if (!showMascot) return <MascotOrDot />
  return <NotchBot badge={badge} reduced={m.settings.reduced_motion} canvasRef={botRef} />
}

export function PeekView() {
  const { placement, m, pill, runner } = useOverlay()
  const pulse = m.arrival ? 'ov-pulse' : undefined
  const open = () => runner.dispatch({ type: 'click', target: 'peek' })
  // In the notch the overlay owns the wrapper and the frame (NotchFrame, 02 §12); the peek is the frame's content.
  if (placement === 'notch') {
    return (
      <div className="m-overlay-wings">
        {/* Now playing (02 §6.1): the artwork (or the equalizer) opposite the mascot. */}
        <div className="m-overlay-wing" style={{ padding: 0, justifyContent: 'center' }}>
          <PeekNowPlaying />
        </div>
        <div className="m-overlay-housing" />
        <div className="m-overlay-wing is-right">
          <PeekMascot />
        </div>
      </div>
    )
  }
  const items = pill ? 1 + pill.count : 0
  const label = placement === 'top-bar' && pill ? `${items} ${items === 1 ? 'item' : 'items'}` : undefined
  return (
    <span className="ov-frame" data-testid="overlay-frame" data-measure="">
      <Peek top={placement === 'top-bar'} mascot={<PeekMascot />} eq={<PeekNowPlaying />} label={label} onClick={open} className={pulse} />
    </span>
  )
}

export function notchVars(notch: { width: number; height: number }): CSSProperties {
  return { '--ov-notch-w': `${notch.width}px`, '--ov-notch-h': `${notch.height}px` } as CSSProperties
}

/** Bar heights (px) for an input level 0..1: 4 at silence, up to 14 for the middle bar. */
export function meterHeights(level: number): [number, number, number] {
  const l = Math.max(0, Math.min(1, level))
  return [0.6, 1, 0.8].map((f) => Math.round(4 + 10 * l * f)) as [number, number, number]
}

/** The dictation input level (02 §5.6): three 2-px bars in --m-accent; static glyph heights under reduced motion. */
export function LevelMeter({ level, reduced }: { level: number; reduced?: boolean }) {
  const h = meterHeights(level)
  return (
    <span className={`m-meter${reduced ? ' is-static' : ''}`} aria-hidden="true" data-testid="dictation-meter">
      {h.map((px, i) => (
        <i key={i} style={reduced ? undefined : { height: `${px}px` }} />
      ))}
    </span>
  )
}

function FlashText({ text, keepPill, onOpen }: { text: string; keepPill: boolean; onOpen: () => void }) {
  const muted = /terminal|already/i.test(text)
  return (
    <span className={`ov-flash${muted ? ' is-muted' : ''}`} data-testid="pill-flash">
      {keepPill ? `${text} · ` : text}
      {keepPill && (
        <a
          href="#"
          onClick={(e) => {
            e.preventDefault()
            e.stopPropagation()
            onOpen()
          }}
        >
          Open in app
        </a>
      )}
    </span>
  )
}

export function PillView() {
  const { placement, m, pill, runner, act, openMenu, decidedFor } = useOverlay()
  const { ref, visible } = useFitParts(pill?.parts ?? [])
  if (!pill) return null
  // A resolved flash belongs to a Q1 pill; the dictation pill draws over it (02 §5.6); an app-window flash is its own pill model (02 §5.4).
  const flash = pill.kind === 'dictation' || pill.kind === 'flash' ? null : m.flash
  const row = pill.row
  const busy = !!row && !!decidedFor(row.id)
  const expand = () => runner.dispatch({ type: 'click', target: 'pill', row: pill.ref })
  const phrase = flash ? <FlashText text={flash.text} keepPill={flash.keepPill} onOpen={() => act('open_in_app', row)} /> : <PhraseParts parts={pill.parts} diff={pill.diff} visible={visible} />
  const phraseProps = { ref, role: 'status', 'aria-live': pill.live, 'aria-label': flash ? flash.text : pill.label, 'data-testid': 'pill-text' } as const
  const showActions = !flash || flash.keepPill
  const stop = (e: { stopPropagation(): void }) => e.stopPropagation()
  const signal = pill.meter !== undefined ? <LevelMeter level={pill.meter} reduced={m.settings.reduced_motion} /> : pill.busy ? <Spinner /> : null
  const actions = showActions && (pill.primary || pill.menu.length > 0 || signal) && (
    <>
      {signal}
      {pill.primary && (
        <Button
          variant={pill.primary.accent ? 'primary' : 'default'}
          disabled={busy && (pill.primary.id === 'allow' || pill.primary.id === 'answer')}
          onClick={(e) => {
            stop(e)
            act(pill.primary!.id, row, pill.open ? { id: pill.open.id, view: pill.open.view } : { id: pill.ref ?? undefined, tab: pill.tab })
          }}
          data-testid="pill-primary"
        >
          {pill.primary.label}
        </Button>
      )}
      {pill.menu.length > 0 && (
        <MenuButton
          label="More"
          onClick={(e) => {
            stop(e)
            openMenu(
              e.currentTarget,
              pill.menu.map((a) => ({ id: a.id, label: a.label, disabled: busy && (a.id === 'deny' || a.id === 'always') })),
              (id) => act(id as never, row, { id: pill.ref ?? undefined, tab: pill.tab }),
            )
          }}
        />
      )}
    </>
  )
  const swapKey = pill.kind === 'flash' ? `flash:${pill.label}` : `${pill.kind}:${pill.ref}`
  const glyph = pill.glyph && <span className={`m-flash-glyph is-${pill.glyph}`} aria-hidden="true" data-testid="flash-glyph" />
  if (placement === 'notch') {
    return (
      <div className="m-overlay-wings">
        <div className="m-overlay-wing">
          <MascotOrDot />
          {glyph}
          <span className="m-ellipsis ov-swap" key={swapKey} {...phraseProps}>
            {phrase}
          </span>
        </div>
        <div className="m-overlay-housing" />
        <div className="m-overlay-wing is-right">
          {pill.count > 0 && <span className="m-count">{pill.count}</span>}
          {actions}
        </div>
      </div>
    )
  }
  return (
    <span className="ov-frame" data-testid="overlay-frame" data-measure="">
      <Pill
        key={swapKey}
        mascot={
          glyph ? (
            <>
              <MascotOrDot />
              {glyph}
            </>
          ) : (
            <MascotOrDot />
          )
        }
        phrase={phrase}
        phraseProps={phraseProps}
        count={pill.count > 0 ? pill.count : undefined}
        actions={actions || undefined}
        onClick={expand}
        className="ov-swap"
        testId="pill"
      />
    </span>
  )
}
