// Dictation step preview: three scenes of 4.5 s, each with the pill listening (a three-bar meter), then
// working (the kit spinner), then its result over a different screen: (a) the note "Groceries" open with
// three bullets, "- Basil and tomatoes" in as a fourth, "typed · 3 words"; (b) a Slack thread from
// Priya with the reply field focused, the reply in it, "typed · 6 words"; (c) the Chat field,
// "Hey Mode, summarize this thread" heard, "thinking · Mode", then "Mode answered · …" with the reply's
// first line in the thread. Dictated text lands whole, in one frame, once transcribed (whisper hands the
// words over on release), never typed out. 13.5 s a loop; under reduced motion the first scene's end state.
import { useState } from 'react'
import { PersonAvatar } from '../../../kit/avatars'
import { Button } from '../../../kit/button'
import { BubbleUser, ChatInput, ChatThread, Reply } from '../../../kit/chat'
import { StaticInput } from '../../../kit/controls'
import { MsgRow } from '../../../kit/lists'
import { Pill } from '../../../kit/overlay'
import { Spinner } from '../../../kit/primitives'
import { Frame, type Placement } from './Frame'
import { reducedMotion, useLoop } from './loop'

const SCENE = 4500
const PERIOD = SCENE * 3
/** Into a scene: listening until LISTENED, working until RESULT, then the result. */
const LISTENED = 1700
const RESULT = 2500
const ANSWERED = 3100

const BULLETS = ['- Oat milk', '- Lemons', '- Coffee filters']
export const SPOKEN_NOTE = '- Basil and tomatoes'
export const SPOKEN_REPLY = 'Thanks Priya, shipping the fix tonight.'
export const SPOKEN_ASK = 'Hey Mode, summarize this thread'
const ANSWER = 'Priya needs the retry change approved before the 4 pm train; the fix ships tonight.'

const words = (s: string) => s.replace(/^-\s*/, '').split(/\s+/).filter(Boolean).length

export function Meter() {
  return (
    <span className="m-ob-meter" aria-hidden="true">
      <i />
      <i />
      <i />
    </span>
  )
}

export type DictationPhase = 'listening' | 'transcribing' | 'typed' | 'thinking' | 'answered'

/** The dictation pill in one of its states: `typed` counts `text`'s words; `answered` shows the answer's start. */
export function DictationPill({ phase, placement, text = SPOKEN_NOTE }: { phase: DictationPhase; placement: Placement; text?: string }) {
  const phrase = phase === 'typed' ? `typed · ${words(text)} words` : phase === 'thinking' ? 'thinking · Mode' : phase === 'answered' ? 'Mode answered · Priya needs the retry change…' : phase
  return <Pill notch={placement === 'notch'} mascot={phase === 'listening' ? <Meter /> : phase === 'transcribing' || phase === 'thinking' ? <Spinner /> : undefined} phrase={phrase} />
}

function NoteScene({ t }: { t: number }) {
  return (
    <div className="m-card m-ob-field m-ob-note m-ob-enter" data-scene="note">
      <div className="m-t-title">Groceries</div>
      {BULLETS.map((b) => (
        <div key={b} className="m-ob-note-line">
          {b}
        </div>
      ))}
      <div className="m-ob-note-line">
        {t >= RESULT && <span className="m-ob-enter">{SPOKEN_NOTE}</span>}
        <span className="m-caret" />
      </div>
    </div>
  )
}

function ThreadScene({ t }: { t: number }) {
  return (
    <div className="m-card m-ob-field m-ob-enter" data-scene="reply">
      <MsgRow avatar={<PersonAvatar initials="PR" source="slack" />} sender="Priya" channel="#release" time="09:41" preview="Can you approve the retry change before the 4 pm train?" />
      <div className="m-composer">
        <StaticInput lg value={t >= RESULT ? <span className="m-ob-enter">{SPOKEN_REPLY}</span> : undefined} placeholder="Reply to Priya" focused />
        <Button variant="primary">Send</Button>
      </div>
    </div>
  )
}

function ChatScene({ t }: { t: number }) {
  return (
    <div className="m-card m-ob-field m-ob-chat m-ob-enter" data-scene="chat">
      <ChatThread>
        {t >= LISTENED && <BubbleUser className="m-ob-enter">{SPOKEN_ASK}</BubbleUser>}
        {t >= ANSWERED && (
          <Reply actions={false} className="m-ob-enter">
            {ANSWER}
          </Reply>
        )}
      </ChatThread>
      <ChatInput editable={false} label="Message" />
    </div>
  )
}

export function DictationPreview({ placement }: { placement: Placement }) {
  const loop = useLoop(PERIOD)
  const [reduced] = useState(reducedMotion)
  const scene = reduced ? 0 : Math.min(2, Math.floor(loop / SCENE))
  const t = reduced ? SCENE : loop - scene * SCENE
  const phase: DictationPhase =
    scene === 2 ? (t < LISTENED ? 'listening' : t < ANSWERED ? 'thinking' : 'answered') : t < LISTENED ? 'listening' : t < RESULT ? 'transcribing' : 'typed'
  return (
    <Frame step="dictation">
      <div className={`m-ob-screen is-${placement}`} data-scene={scene}>
        <div className="m-ob-spot">
          <DictationPill phase={phase} placement={placement} text={scene === 1 ? SPOKEN_REPLY : SPOKEN_NOTE} />
        </div>
        {scene === 0 ? <NoteScene key="note" t={t} /> : scene === 1 ? <ThreadScene key="reply" t={t} /> : <ChatScene key="chat" t={t} />}
      </div>
    </Frame>
  )
}
