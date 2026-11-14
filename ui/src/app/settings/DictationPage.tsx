// Settings → Dictation (XC-36, XC-37; 03 §8.1): the switch (turning it on
// downloads the speech model, then opens the microphone once so the OS asks),
// the hold key (set in Shortcuts, the only place keys are printed: ADR-0038),
// the model and, for the multilingual one, the language, "Hey Mode" requests,
// what happens with nothing focused, a field to try it in, and the privacy line.
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '../../kit/button'
import { Select, Switch } from '../../kit/controls'
import { SettingError } from '../../kit/settings'
import { EVENTS, type DictationModel, type DictationModelProgress, type DictationModels, type DictationState, type DictationStatus, type MicCheck } from '../../state/contract'
import { errorText, useSetting } from '../../state/settings-store'
import { invoke, listen } from '../../tauri'
import { navigate } from '../router'
import { Field, Group, Note, SettingSeg } from './controls'

/** `auto`, then the 20 most used of whisper's languages (multilingual model only). */
export const DICTATION_LANGUAGES = [
  ['auto', 'Detect automatically'],
  ['en', 'English'],
  ['es', 'Spanish'],
  ['fr', 'French'],
  ['de', 'German'],
  ['it', 'Italian'],
  ['pt', 'Portuguese'],
  ['nl', 'Dutch'],
  ['pl', 'Polish'],
  ['ru', 'Russian'],
  ['uk', 'Ukrainian'],
  ['tr', 'Turkish'],
  ['ar', 'Arabic'],
  ['hi', 'Hindi'],
  ['ja', 'Japanese'],
  ['ko', 'Korean'],
  ['zh', 'Chinese'],
  ['sv', 'Swedish'],
  ['da', 'Danish'],
  ['no', 'Norwegian'],
  ['fi', 'Finnish'],
] as const

/** Megabytes as the page prints them ("57 MB"). */
export function mb(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`
}

/** One model as a picker row: "base.en · 57 MB · installed". */
export function modelLabel(m: Pick<DictationModel, 'label' | 'size_bytes' | 'installed' | 'multilingual'>): string {
  return [m.label, mb(m.size_bytes), m.multilingual ? 'many languages' : null, m.installed ? 'installed' : null].filter(Boolean).join(' · ')
}

type Phase = { kind: 'idle' } | { kind: 'download'; downloaded: number; total: number } | { kind: 'mic' }

export function DictationPage() {
  const [enabled, setEnabled] = useSetting<boolean>('dictation.enabled')
  const [model, setModel] = useSetting<string>('dictation.model')
  const [language, setLanguage] = useSetting<string>('dictation.language')
  const [status, setStatus] = useState<DictationStatus | null>(null)
  const [models, setModels] = useState<DictationModel[]>([])
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' })
  /** A dictation failure (download, microphone, a hold): under the switch. */
  const [error, setError] = useState<string | null>(null)
  /** A failed "Hey Mode" request (no key, backend unreachable): under the Commands row (03 §8.20). */
  const [requestError, setRequestError] = useState<string | null>(null)
  const [commands, setCommands] = useSetting<boolean>('dictation.commands')
  const [replyCard, setReplyCard] = useSetting<boolean>('dictation.reply_card')
  /** Turn the switch on once the download (and the microphone check) finish. */
  const [turningOn, setTurningOnState] = useState(false)
  const turningRef = useRef(false)
  const setTurningOn = useCallback((on: boolean) => {
    turningRef.current = on
    setTurningOnState(on)
  }, [])

  const refresh = useCallback(async () => {
    try {
      const [s, list] = await Promise.all([invoke<DictationStatus>('dictation_status'), invoke<DictationModels>('dictation_models')])
      setStatus(s)
      setModels(list?.models ?? [])
    } catch (e) {
      setError(errorText(e))
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const micThenOn = useCallback(async () => {
    setPhase({ kind: 'mic' })
    try {
      const r = await invoke<MicCheck>('dictation_mic_check')
      if (!r?.ok) {
        setError(r?.error ?? 'the microphone could not be opened')
        return
      }
      await setEnabled(true)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setPhase({ kind: 'idle' })
      setTurningOn(false)
    }
  }, [setEnabled, setTurningOn])

  // The model download's progress, then the microphone check when it was turning on.
  useEffect(() => {
    const sub = listen<DictationModelProgress>(EVENTS.dictationModel, (p) => {
      if (p.error) {
        setPhase({ kind: 'idle' })
        setTurningOn(false)
        setError(p.error)
        return
      }
      if (!p.done) {
        setPhase({ kind: 'download', downloaded: p.downloaded, total: p.total })
        return
      }
      setPhase({ kind: 'idle' })
      void refresh()
      if (turningRef.current) void micThenOn()
    })
    return () => void sub.then((u) => u())
  }, [refresh, micThenOn, setTurningOn])

  // A failure while dictating shows under the switch; a failed request under the Commands row.
  // The next hold that ends well clears both.
  useEffect(() => {
    const sub = listen<DictationState>(EVENTS.dictation, (p) => {
      if (p.state === 'error' && p.error) {
        if (p.source === 'request') setRequestError(p.error)
        else setError(p.error)
      } else if (p.state === 'done') {
        setError(null)
        setRequestError(null)
      }
    })
    return () => void sub.then((u) => u())
  }, [])

  const ensure = async (id: string) => {
    try {
      await invoke('dictation_model_ensure', { model: id })
    } catch (e) {
      setTurningOn(false)
      setPhase({ kind: 'idle' })
      setError(errorText(e))
    }
  }

  const toggle = async (on: boolean) => {
    setError(null)
    if (!on) {
      setTurningOn(false)
      await setEnabled(false)
      return
    }
    setTurningOn(true)
    const current = models.find((m) => m.id === model)
    if (current?.installed) await micThenOn()
    else await ensure(model)
  }

  const pickModel = async (id: string) => {
    setError(null)
    await setModel(id)
    const m = models.find((x) => x.id === id)
    if (enabled && m && !m.installed) await ensure(id)
  }

  const available = status?.available !== false
  const busy = phase.kind !== 'idle' || turningOn
  const current = models.find((m) => m.id === model)
  const options = models.map((m) => [m.id, modelLabel(m)] as const)

  const switchDesc =
    phase.kind === 'download' ? (
      <span role="status" data-testid="dictation-progress">
        {`Downloading ${model} · ${Math.round(phase.downloaded / 1_000_000)} of ${phase.total ? mb(phase.total) : '…'}`}
      </span>
    ) : phase.kind === 'mic' ? (
      <span role="status" data-testid="dictation-progress">
        Asking for the microphone
      </span>
    ) : available ? (
      'Hold a key, say it, and it is typed.'
    ) : (
      'Not in this build.'
    )

  return (
    <>
      <Group title="Dictation" id="dictation">
        <Field title="Turn on dictation" desc={switchDesc} k="dictation.enabled" error={error ? <SettingError testId="dictation-error">{error}</SettingError> : undefined}>
          <Switch on={!!enabled || turningOn} label="Turn on dictation" disabled={!available || phase.kind !== 'idle'} onChange={(on) => void toggle(on)} />
        </Field>
        <Field title="Hold to dictate" desc="The key is set in Shortcuts." k="dictation.hotkey">
          <Button quiet onClick={() => navigate({ view: 'shortcuts', query: 'dictate' })} data-testid="dictation-shortcut">
            Set in Shortcuts
          </Button>
        </Field>
        <Field title="Model" desc={current && !current.installed && !busy ? 'Downloads when dictation is on.' : 'Runs on this machine.'} k="dictation.model">
          <Select label="Model" value={model} options={options.length ? options : [[model, model] as const]} onChange={(v) => void pickModel(String(v))} testId="dictation-model" />
        </Field>
        {current?.multilingual && (
          <Field title="Language" k="dictation.language">
            <Select label="Language" value={language} options={DICTATION_LANGUAGES} onChange={(v) => void setLanguage(v)} testId="dictation-language" />
          </Field>
        )}
        <Note>Audio stays on this machine. Only a “Hey Mode” request goes to your AI backend, as a Chat message.</Note>
      </Group>

      <Group title="Hey Mode requests" id="requests">
        <Field
          title="Commands"
          desc="Start with “Hey Mode” and the rest goes to Chat. The answer shows in the overlay when the app window is not in front."
          k="dictation.commands"
          error={
            requestError ? (
              <SettingError testId="dictation-request-error">
                {requestError}
                <Button quiet className="sg-err-action" onClick={() => navigate({ view: 'settings', rest: 'ai' })}>
                  Open AI settings
                </Button>
              </SettingError>
            ) : undefined
          }
        >
          <Switch on={commands !== false} label="“Hey Mode” commands" onChange={(on) => void setCommands(on)} />
        </Field>
        <Field title="Answer in the overlay" desc="Show the answer in the overlay when the app window is not in front." k="dictation.reply_card">
          <Switch on={replyCard !== false} disabled={commands === false} label="Answer in the overlay" onChange={(on) => void setReplyCard(on)} />
        </Field>
        <SettingSeg
          k="dictation.unfocused"
          title="When nothing in Mode has focus"
          options={[
            ['capture', 'Capture a note'],
            ['nothing', 'Do nothing'],
          ]}
        />
      </Group>

      <Group title="Try it" id="try">
        <Field title="Test field" desc="Filled by the same path as any other field. A “Hey Mode” request goes to Chat instead." stack htmlFor="dictation-try">
          <input id="dictation-try" className="m-input" placeholder="Hold the key and say something" aria-label="Try dictation" data-testid="dictation-try" />
        </Field>
      </Group>
    </>
  )
}
