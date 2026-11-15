// `expanded` (02 §7): the head (mascot with its badge, phrase, "New: …"
// strip, health dot, pin, ⋮), the Now playing strip while a player runs,
// the tabs with their counts (Music only while Now playing is on), the active
// tab's rows and the capture box, in the kit's frame for the placement.
import { useMemo } from 'react'
import { MenuButton } from '../kit/button'
import { Tab, Tabs } from '../kit/controls'
import { Icon } from '../kit/icons'
import { Overlay as Frame } from '../kit/overlay'
import { Spinner } from '../kit/primitives'
import { visibleTabs, type OverlayTab } from '../state/contract'
import { tabCounts, useOverlayStore } from '../state/overlay-store'
import { CaptureBox } from './CaptureBox'
import { MascotOrDot } from './Compact'
import { useOverlay } from './context'
import { NowPlayingSlot } from './NowPlaying'
import { MusicTab } from './MusicTab'
import { CalendarTab, InboxTab, NotesTab } from './OtherTabs'
import { ReplyCard } from './ReplyCard'
import { SessionsTab } from './SessionsTab'

const TAB_LABEL: Record<OverlayTab, string> = { sessions: 'Sessions', inbox: 'Inbox', notes: 'Notes', calendar: 'Calendar', music: 'Music' }

function healthTitle(h: { offline: boolean; paused_accounts: string[]; hooks: string; read_only: boolean } | null): string | null {
  if (!h) return null
  if (h.read_only) return 'Mode is read-only · open the app for details'
  if (h.offline) return 'Offline · showing cached messages'
  if (h.paused_accounts.length) return `${h.paused_accounts.join(', ')} paused`
  if (h.hooks === 'stale' || h.hooks === 'missing' || h.hooks === 'conflict') return 'Hooks need attention'
  return null
}

function HeadControls({ phraseId, leadTaken }: { phraseId?: string; leadTaken?: boolean }) {
  const { m, runner, act, openMenu, setTab } = useOverlay()
  const health = useOverlayStore((s) => s.health)
  const title = healthTitle(health)
  // The first control pushes itself to the right edge (no wrapper, so the class tree matches the wireframe).
  let placed = !!leadTaken
  const lead = () => {
    if (placed) return undefined
    placed = true
    return { marginLeft: 'auto' } as const
  }
  return (
    <>
      {m.strip && (
        <span
          className="m-tag"
          role="status"
          data-testid="new-strip"
          style={lead()}
          onClick={(e) => {
            e.stopPropagation()
            setTab(m.strip!.tab)
            runner.dispatch({ type: 'focus_row', row: m.strip!.ref })
          }}
        >
          {`New · ${m.strip.label}`}
        </span>
      )}
      {title && <span className="m-dot ov-health" title={title} role="img" aria-label={title} data-testid="health-dot" style={lead()} />}
      {m.pinned && (
        <span className="m-btn m-btn-icon is-quiet ov-pin" role="button" aria-label="Pinned" title="Pinned" onClick={() => runner.dispatch({ type: 'pin' })} data-testid="pin" style={lead()}>
          <Icon id="pin" />
        </span>
      )}
      <MenuButton
        label="More"
        aria-describedby={phraseId}
        style={lead()}
        onClick={(e) =>
          openMenu(
            e.currentTarget,
            [
              { id: 'pin', label: m.pinned ? 'Unpin' : 'Pin' },
              { id: 'expand_app', label: 'Open in app' },
              { id: 'settings', label: 'Settings' },
            ],
            (id) => (id === 'pin' ? runner.dispatch({ type: 'pin' }) : act(id as 'expand_app' | 'settings')),
          )
        }
      />
    </>
  )
}

export function Expanded() {
  const ctx = useOverlay()
  const { placement, m, pill, setTab, capture, captureRef, runner, act, linkedRow } = ctx
  const rows = useOverlayStore((s) => s.rows)
  const inbox = useOverlayStore((s) => s.inbox)
  const notes = useOverlayStore((s) => s.notes)
  const agenda = useOverlayStore((s) => s.agenda)
  const counts = useMemo(() => tabCounts({ rows, inbox, notes, agenda }), [rows, inbox, notes, agenda])
  const captureSaved = useOverlayStore((s) => s.captureSaved)
  const health = useOverlayStore((s) => s.health)
  // The head names what is happening, with the mascot before it. With nothing to name (at rest, and on the
  // reply card, whose head holds only the spinner while the request runs: 02 §5.7, §7.1) both stay out.
  const reply = m.reply ?? null
  const phrase = reply ? null : m.flash ? m.flash.text : pill ? pill.parts.slice(0, 2).join(' · ') : null
  const busy = reply && !reply.done ? <Spinner /> : null
  const phraseId = phrase === null ? undefined : 'ov-head-phrase'

  const head =
    placement === 'notch' ? (
      <div className="m-overlay-wings">
        <div className="m-overlay-wing">
          {phrase !== null && <MascotOrDot />}
          {phrase !== null && (
            <span className="m-ellipsis" id={phraseId} data-testid="head-phrase">
              {phrase}
            </span>
          )}
          {busy}
        </div>
        <div className="m-overlay-housing" />
        <div className="m-overlay-wing is-right">
          <HeadControls phraseId={phraseId} />
        </div>
      </div>
    ) : (
      <div className="m-overlay-head">
        {phrase !== null && <MascotOrDot />}
        {phrase !== null && (
          <span className="m-ellipsis" style={{ fontWeight: 500 }} id={phraseId} data-testid="head-phrase">
            {phrase}
          </span>
        )}
        {busy}
        {!reply && pill && pill.count > 0 && (
          <span className="m-count" style={{ marginLeft: 'auto' }}>
            {pill.count}
          </span>
        )}
        <HeadControls phraseId={phraseId} leadTaken={!reply && !!pill && pill.count > 0} />
      </div>
    )

  const settings = useOverlayStore((s) => s.settings)
  const content = m.tab === 'sessions' ? <SessionsTab /> : m.tab === 'inbox' ? <InboxTab /> : m.tab === 'notes' ? <NotesTab /> : m.tab === 'music' ? <MusicTab /> : <CalendarTab />

  const footer = (
    <CaptureBox
      inputRef={captureRef}
      value={capture.value}
      onChange={capture.set}
      onFocus={() => runner.dispatch({ type: 'capture_focus' })}
      onBlur={() => runner.dispatch({ type: 'capture_blur' })}
      saved={!!captureSaved && captureSaved.until > ctx.now}
      onOpen={() => act('open_tab', undefined, { tab: 'notes', id: captureSaved?.note_id })}
      linkedTo={linkedRow?.display_name ?? null}
      onUnlink={capture.unlink}
      disabledReason={health?.vault_missing ? 'Choose a vault to capture notes' : health?.read_only ? 'Mode is read-only' : null}
    />
  )

  const tabs = (
    <Tabs label="Overlay tabs">
      {visibleTabs(settings).map((t) => (
        <Tab key={t} active={m.tab === t} count={counts[t] > 0 ? counts[t] : undefined} onClick={() => setTab(t)}>
          {TAB_LABEL[t]}
        </Tab>
      ))}
    </Tabs>
  )

  // A voice request's answer replaces the tabs, the rows and the capture box (02 §5.7, §7.1).
  if (reply) {
    const card = <ReplyCard reply={reply} />
    if (placement === 'notch')
      return (
        <>
          {head}
          {card}
        </>
      )
    return (
      <Frame placement={placement} state="expanded" phrase={phrase} head={head} className="ov-frame" testId="overlay-frame">
        {card}
      </Frame>
    )
  }

  // In the notch the overlay owns the wrapper and the frame (NotchFrame, 02 §12); the panel is the frame's content.
  // The Now playing strip sits under the header, above the tabs (02 §7.1).
  if (placement === 'notch')
    return (
      <>
        {head}
        <NowPlayingSlot />
        {tabs}
        {content}
        {footer}
      </>
    )
  return (
    <Frame placement={placement} state="expanded" phrase={phrase} head={head} footer={footer} className="ov-frame" testId="overlay-frame">
      <NowPlayingSlot />
      {tabs}
      {content}
    </Frame>
  )
}
