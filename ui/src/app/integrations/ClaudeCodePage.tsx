// The Claude Code card's detail page (03 §7C.3, §8.6): the facts of the
// hook install, the scope statement, the actions (Install / Update / Repair,
// Test round trip, Copy JSON, Open settings file, Uninstall hooks) and the
// card's settings (default terminal app, path to `claude`, hook timeouts; a
// timeout change offers Update hooks).
import { useState } from 'react'
import { Button } from '../../kit/button'
import { Form, Section } from '../../kit/views'
import { DeviceGlyph } from '../../kit/settings'
import { invoke } from '../../tauri'
import { BrandMark, brandForTerminal } from '../../kit/brands'
import { useSetting, useSettingsStore } from '../../state/settings-store'
import { copyText, useNow } from '../ui'
import { Field, FieldError, Note, PickerField, SettingNumber, SettingText } from '../settings/form-controls'
import { useClaudeHealth, useHooks } from './hooks-store'
import { ago, claudeCodeCard, tilde } from './model'

const TERMINALS: Record<string, string[]> = {
  macos: ['Terminal.app', 'iTerm2', 'Warp', 'Ghostty'],
  windows: ['Windows Terminal', 'PowerShell', 'WSL'],
  linux: ['GNOME Terminal', 'Konsole', 'Ghostty', 'Alacritty'],
}

export function ClaudeCodePage() {
  const now = useNow(5000)
  const status = useHooks((s) => s.status)
  const diag = useHooks((s) => s.diag)
  const statusError = useHooks((s) => s.statusError)
  const platform = useSettingsStore((s) => s.platform)
  const health = useClaudeHealth()
  const card = claudeCodeCard(status, diag, now, health)
  const [rt, setRt] = useState<string | null>(null)
  const [terminal, setTerminal] = useSetting<string | null>('sessions.terminal_app')
  const terminals = platform?.terminal_apps?.length ? platform.terminal_apps : TERMINALS[platform?.os ?? 'macos']!
  const row = diag?.hooks.find((h) => h.settings_path === status?.path) ?? diag?.hooks[0]
  const installed = card.status.state !== 'idle'

  const facts: [string, React.ReactNode][] = [
    ['Settings file', <code key="p">{tilde(status?.path ?? row?.settings_path ?? '~/.claude/settings.json')}</code>],
    ['Relay', <code key="r">{tilde(status?.relay_path ?? diag?.relay.path ?? '') || 'not staged'}</code>],
    ['Relay version', status?.version ?? diag?.relay.version ?? 'unknown'],
    ['Events', status?.events.length ? status.events.join(', ') : 'none'],
    ['Installed', when(status?.installed_at ?? row?.installed_at, now)],
    ['Verified', status?.last_round_trip ? `${when(status.verified_at ?? row?.verified_at ?? status.last_round_trip.at, now)} · round trip ${status.last_round_trip.latency_ms} ms` : when(status?.verified_at ?? row?.verified_at, now)],
    ['Backup', status?.backup_path ?? row?.backup_path ? <code key="b">{tilde(status?.backup_path ?? row?.backup_path)}</code> : '—'],
    ['Claude Code', diag ? (diag.claude.found ? <span key="c">found at <code>{tilde(diag.claude.path)}</code>{diag.claude.version ? ` · ${diag.claude.version}` : ''}</span> : 'not found') : '…'],
  ]

  const actions = (
    <>
      {card.action.kind === 'docs' ? (
        <Button variant="primary" onClick={() => void invoke('plugin:opener|open_url', { url: 'https://code.claude.com/docs' }).catch(() => {})}>
          Open docs
        </Button>
      ) : (
        <Button variant="primary" onClick={() => void useHooks.getState().preview('install')}>
          {card.action.label}
        </Button>
      )}
      <Button
        disabled={!installed}
        onClick={() =>
          void useHooks
            .getState()
            .roundTrip()
            .then((r) => setRt(r ? (r.acked ? `round trip ${r.latency_ms} ms` : 'the core did not answer') : (useHooks.getState().statusError ?? 'round trip failed')))
        }
      >
        Test round trip
      </Button>
      <Button
        onClick={() =>
          void invoke<{ after: string }>('hooks_install_preview').then(
            (p) => copyText(p.after, 'JSON copied'),
            () => {},
          )
        }
      >
        Copy JSON
      </Button>
      <Button onClick={() => void invoke('plugin:opener|open_path', { path: status?.path ?? '' }).catch(() => {})} disabled={!status?.path}>
        Open settings file
      </Button>
      <Button disabled={!installed} onClick={() => void useHooks.getState().preview('uninstall')}>
        Uninstall hooks
      </Button>
      {rt && (
        <span className="m-t-meta" role="status" data-testid="round-trip">
          {rt}
        </span>
      )}
    </>
  )

  return (
    <>
      <Section title="Hooks" foot={actions}>
      <Form label="Claude Code" className="ig-facts">
        {facts.map(([label, value]) => (
          <Field key={label} label={label}>
            <span className="m-t-dense">{value}</span>
          </Field>
        ))}
        {status?.conflicts.length ? (
          <Note bad>
            {`Another hook is registered for ${status.conflicts.map((c) => c.event).join(', ')}; it is kept as is.`}
          </Note>
        ) : null}
        {statusError && <Note bad>{statusError}</Note>}
        <Note>Mode edits only ~/.claude/settings.json; never a project's .claude/settings.json or .claude/settings.local.json.</Note>
      </Form>
      </Section>
      <Section title="Settings">
      <Form label="Claude Code settings">
        <Field label="Terminal app">
          <div className="m-inline" data-setting="sessions.terminal_app">
            {(() => {
              const id = brandForTerminal(terminal ?? terminals[0])
              return id ? <BrandMark id={id} size={16} label={terminal ?? terminals[0] ?? ''} /> : null
            })()}
            <PickerField label="Default terminal app" value={terminal} options={[[null, 'first detected'], ...terminals.map((t) => [t, t] as const)]} onPick={(v) => void setTerminal(v)} />
            <DeviceGlyph />
            <FieldError k="sessions.terminal_app" />
          </div>
        </Field>
        <SettingText k="sessions.claude_path" label="Path to claude" placeholder="from PATH" wide />
        <TimeoutRows />
      </Form>
      </Section>
    </>
  )
}

/** "2 d ago" with the full time in the tooltip-free text form "2026-10-19 14:03". */
function when(iso: string | null | undefined, now: number): string {
  if (!iso) return '—'
  const rel = ago(iso, now)
  return `${rel ?? ''} · ${iso.slice(0, 16).replace('T', ' ')}`.replace(/^ · /, '')
}

/** Hook timeouts; a change makes the installed block stale, so Update hooks is offered (03 §8.6). */
function TimeoutRows() {
  const [decision] = useSetting<number>('hooks.decision_timeout_seconds')
  const [asyncT] = useSetting<number>('hooks.async_timeout_seconds')
  const [seen] = useState(() => `${decision}/${asyncT}`)
  const changed = seen !== `${decision}/${asyncT}`
  return (
    <>
      <SettingNumber k="hooks.decision_timeout_seconds" label="Decision timeout" unit="seconds" hint="permission requests and questions" />
      <SettingNumber k="hooks.async_timeout_seconds" label="Event timeout" unit="seconds" hint="every other hook event" />
      {changed && (
        <Field label="">
          <div className="m-inline">
            <span className="m-t-meta">The installed hooks still use the old timeouts.</span>
            <Button onClick={() => void useHooks.getState().preview('install')}>Update hooks</Button>
          </div>
        </Field>
      )}
    </>
  )
}
