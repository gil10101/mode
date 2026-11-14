// Settings → AI (03 §8.14; ADR-0020, ADR-0021, ADR-0032 D14) on the
// connectors core's commands (06 §10): one backend (Claude API with the
// user's own key, Ollama, LM Studio) chosen through `ai_backend_set`, the key
// stored through `ai_key_set` (only "stored" and its last four characters
// ever come back, XC-22), the active backend's state from `ai_health`, the
// local server address (loopback or private network only), a model per
// feature, the local-only switch (re-queues scoring), the token budget and
// Test call (`ai_test_call`). The backend is named as "Claude API"; no
// slogan anywhere.
import { useCallback, useEffect, useState } from 'react'
import { Button } from '../../kit/button'
import { BrandMark, brandForBackend } from '../../kit/brands'
import { Seg, SegItem, Switch } from '../../kit/controls'
import { ModelPicker, SettingError } from '../../kit/settings'
import { invoke } from '../../tauri'
import type { AiBackendRow, AiBackendSetArgs, AiHealth } from '../../state/contract-connectors'
import type { AiBackend, AiTestResult, SecretStatus } from '../../state/contract-settings'
import { ANTHROPIC_KEY } from '../../state/contract-settings'
import { errorText, stableDefault, useSetting } from '../../state/settings-store'
import { navigate } from '../router'
import { claudeRow, localModels, useAi, useClaudeModels } from './ai'
import { CommitInput, Field, Group, Note, NullableSelect } from './controls'
import { ProvidersGroup, RoutesGroup } from './ProvidersPage'
import { BudgetRows } from './UrgencyPage'

export const BACKEND_NAME: Record<AiBackend, string> = { claude_api: 'Claude API', ollama: 'Ollama', lm_studio: 'LM Studio' }

/** The status line of the backend scoring goes to (06 §10.7); an added provider is named (06 §10.1A). */
export function healthText(h: AiHealth | null): string {
  if (!h) return 'not checked'
  const name = h.label || (BACKEND_NAME as Record<string, string>)[h.backend] || h.backend
  switch (h.state) {
    case 'ready':
      return h.backups ? `${name} ready · ${h.backups} ${h.backups === 1 ? 'backup' : 'backups'}` : `${name} ready`
    case 'no_key':
      return 'No API key yet'
    case 'invalid_key':
      return 'The API key was refused; replace it'
    case 'unreachable':
      return `${name} is not reachable`
    case 'budget_reached':
      return 'The monthly token budget is reached; scoring waits until next month'
    case 'local_only_blocked':
      return 'Local-only is on: no local backend is set up'
    default:
      return h.message || h.state
  }
}

/** The note under a local address: the LAN notice of 06 §10.3. */
export function addressNote(row: Pick<AiBackendRow, 'address'> | undefined): string | null {
  if (row?.address === 'private') return 'on your private network: data leaves this computer but not your network'
  if (row?.address === 'public' || row?.address === 'invalid') return 'not accepted: use this computer or your private network'
  return null
}

/** The key row (03 §8.14): a password field until stored, then "stored" with Replace and Remove. */
export function ApiKeyRow({ onChange }: { onChange?: (stored: boolean) => void }) {
  const row = useAi((s) => claudeRow(s.rows))
  const loaded = useAi((s) => s.loaded)
  const error = useAi((s) => s.errors.key ?? null)
  const [fallback, setFallback] = useState<SecretStatus | null>(null)
  const [replacing, setReplacing] = useState(false)
  const [value, setValue] = useState('')
  // A core without `ai_backends_list` still answers `secret_status`.
  useEffect(() => {
    if (!loaded || row) return
    invoke<SecretStatus>('secret_status', { name: ANTHROPIC_KEY }).then(setFallback, () => setFallback({ name: ANTHROPIC_KEY, stored: false }))
  }, [loaded, row])
  const stored = (row ? row.key_stored === true : fallback?.stored === true) && !replacing
  const last4 = row?.key_last4 ?? fallback?.last4 ?? null
  const write = async (v: string | null) => {
    const r = await useAi.getState().setKey(v)
    if (!r) return
    setFallback({ name: ANTHROPIC_KEY, stored: r.stored, last4: r.last4 ?? null })
    setReplacing(false)
    setValue('')
    onChange?.(r.stored)
  }
  return (
    <Field
      title="Anthropic API key"
      desc={stored ? (last4 ? `Stored, ends in ${last4}. Kept in the OS keyring.` : 'Stored in the OS keyring.') : 'Kept in the OS keyring, never shown again.'}
      testId="api-key"
      error={error ? <SettingError>{error}</SettingError> : undefined}
    >
      {stored ? (
        <>
          <Button onClick={() => setReplacing(true)}>Replace</Button>
          <Button onClick={() => void write(null)}>Remove</Button>
        </>
      ) : (
        <>
          <input
            className="m-input"
            type="password"
            autoComplete="off"
            spellCheck={false}
            aria-label="Anthropic API key"
            placeholder="sk-ant-…"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && value.trim()) void write(value.trim())
            }}
          />
          <Button disabled={!value.trim()} onClick={() => void write(value.trim())}>
            Save key
          </Button>
          {replacing && <Button onClick={() => setReplacing(false)}>Cancel</Button>}
        </>
      )}
    </Field>
  )
}

/** Test call: a minimal request; latency or the error inline. */
export function useTestCall() {
  const [result, setResult] = useState<AiTestResult | null>(null)
  const [busy, setBusy] = useState(false)
  const run = useCallback(async (backend?: AiBackend) => {
    setBusy(true)
    try {
      setResult(await invoke<AiTestResult>('ai_test_call', backend ? { backend } : {}))
    } catch (e) {
      setResult({ ok: false, latency_ms: null, error: errorText(e) })
    } finally {
      setBusy(false)
    }
  }, [])
  return { result, busy, run }
}

export function testText(r: AiTestResult | null, backend: AiBackend): string | null {
  if (!r) return null
  if (r.ok) return `${BACKEND_NAME[backend]} answered in ${r.latency_ms ?? '?'} ms`
  return `Could not reach ${BACKEND_NAME[backend]} (${r.error ?? 'no answer'}).`
}

/** The local-only switch row (06 §10.4): through `ai_backend_set`, which re-queues scoring at once. Also on Privacy. */
export function LocalOnlySwitch({ title = 'Local-only', desc }: { title?: string; desc?: string }) {
  const [on] = useSetting<boolean>('ai.local_only')
  const error = useAi((s) => s.errors['ai.local_only'] ?? null)
  return (
    <Field title={title} desc={desc} k="ai.local_only" error={error ? <SettingError>{error}</SettingError> : undefined}>
      <Switch on={on === true} label={title} onChange={(next) => void useAi.getState().set({ local_only: next })} />
    </Field>
  )
}

/**
 * A Claude API model of a feature (03 §8.14 Model per feature, §8.21 Chat): the model picker with the key's
 * registry default first, the Claude provider's models, then Other… (the Model id row); written
 * through `ai_backend_set` on a pick, on Enter or when the field is left.
 */
export function AiModel({ arg, k, title, label, desc, models, testId }: { arg: keyof AiBackendSetArgs; k: string; title: string; label?: string; desc: string; models: readonly string[]; testId?: string }) {
  const [stored] = useSetting<string | null>(k)
  const error = useAi((s) => s.errors[k] ?? null)
  const def = String(stableDefault(k) ?? '')
  const asChoice = (v: string | null) => (!v || v === def ? null : v)
  const [draft, setDraft] = useState<string | null>(asChoice(stored))
  useEffect(() => setDraft(!stored || stored === def ? null : stored), [stored, def])
  return (
    <ModelPicker
      id={`ai-${arg}`}
      title={title}
      label={label ?? `${title} model`}
      selectTestId={testId}
      desc={desc}
      setting={k}
      defaultLabel={`Default · ${def}`}
      restLabel="Default"
      models={models.filter((m) => m !== def)}
      value={draft}
      onChange={setDraft}
      onCommit={(v) => {
        const next = v ?? def
        if (next !== stored) void useAi.getState().set({ [arg]: next } as AiBackendSetArgs)
      }}
      error={error ? <SettingError testId={`error-${k}`}>{error}</SettingError> : undefined}
    />
  )
}

/** A text setting written through `ai_backend_set` (the local address). */
function AiText({ arg, k, title, desc, stack }: { arg: keyof AiBackendSetArgs; k: string; title: string; desc?: string | null; stack?: boolean }) {
  const [value] = useSetting<string | null>(k)
  const error = useAi((s) => s.errors[k] ?? null)
  return (
    <Field title={title} desc={desc ?? undefined} k={k} stack={stack} error={error ? <SettingError testId={`error-${k}`}>{error}</SettingError> : undefined}>
      <CommitInput label={title} value={value} onCommit={(v) => void useAi.getState().set({ [arg]: String(v).trim() } as AiBackendSetArgs)} />
    </Field>
  )
}

export function AiPage() {
  const [backend] = useSetting<AiBackend>('ai.backend')
  const rows = useAi((s) => s.rows)
  const health = useAi((s) => s.health)
  const status = useAi((s) => s.status)
  const backendError = useAi((s) => s.errors['ai.backend'] ?? null)
  const [checking, setChecking] = useState(false)
  useEffect(() => {
    void useAi.getState().refresh()
    void useAi.getState().loadProviders()
  }, [])
  const test = useTestCall()
  const claudeModels = useClaudeModels()
  const models = backend === 'claude_api' ? [] : localModels(backend, health, status)
  const localRow = rows?.find((r) => r.backend === backend && r.local)
  const local = backend === 'ollama' ? status?.ollama : backend === 'lm_studio' ? status?.lm_studio : undefined
  const text = testText(test.result, backend)
  const check = async () => {
    setChecking(true)
    await useAi.getState().refresh(true)
    setChecking(false)
  }
  const healthy = !health || health.state === 'ready'
  const healthLine = (
    <span role="status" data-testid="ai-health">
      {healthText(health)}
    </span>
  )
  return (
    <>
      <Group title="Backend" id="backend">
        <Field title="Selected backend" desc="What a feature calls when no route names a provider." k="ai.backend" error={backendError ? <SettingError>{backendError}</SettingError> : undefined}>
          <Seg label="Backend">
            {(['claude_api', 'ollama', 'lm_studio'] as const).map((b) => (
              <SegItem key={b} active={backend === b} onClick={() => void useAi.getState().set({ backend: b })}>
                <BrandMark id={brandForBackend(b)!} size={12} label="" />
                {BACKEND_NAME[b]}
              </SegItem>
            ))}
          </Seg>
        </Field>
        {backend === 'claude_api' && <ApiKeyRow />}
        <Field title="Status" desc={healthy ? healthLine : undefined} error={healthy ? undefined : <SettingError>{healthLine}</SettingError>}>
          <Button disabled={checking} onClick={() => void check()}>
            {checking ? 'Checking…' : 'Check'}
          </Button>
        </Field>
        <Field
          title="Test call"
          desc={text && test.result?.ok ? <span role="status">{text}</span> : 'A one-token request to the backend.'}
          error={text && !test.result?.ok ? <SettingError>{text}</SettingError> : undefined}
        >
          <Button disabled={test.busy} onClick={() => void test.run(backend)}>
            {test.busy ? 'Testing…' : 'Test call'}
          </Button>
        </Field>
        <LocalOnlySwitch desc="Disables every remote AI call; connector polling continues." />
      </Group>

      <ProvidersGroup />
      <RoutesGroup />

      <Group title="Local backend" id="local">
        <AiText arg="local_url" k="ai.local_url" title="Address" desc={addressNote(localRow) ?? (local ? (local.reachable ? `${local.models.length} models` : 'Not reachable') : 'Ollama or LM Studio on this computer or your network.')} />
        <Field title="Local models" desc={status ? `Ollama ${status.ollama.reachable ? 'reachable' : 'not running'} · LM Studio ${status.lm_studio.reachable ? 'reachable' : 'not running'}` : 'Not checked.'}>
          <Button onClick={() => void useAi.getState().refresh()}>Refresh models</Button>
        </Field>
      </Group>

      <Group title="Models" id="models">
        {backend === 'claude_api' ? (
          <>
            <AiModel arg="model_scoring" k="ai.model_scoring" title="Scoring" desc="Rates incoming messages." models={claudeModels} />
            <AiModel arg="model_summary" k="ai.model_summary" title="Summaries" desc="Writes the summaries of sessions and threads." models={claudeModels} />
          </>
        ) : (
          <>
            <LocalModel arg="local_model_scoring" k="ai.local_model_scoring" title="Scoring" models={models} />
            <LocalModel arg="local_model_summary" k="ai.local_model_summary" title="Summaries" models={models} />
          </>
        )}
        <Field title="Chat" desc="Chat's model and timeout are on the Chat page.">
          <Button quiet onClick={() => navigate({ view: 'settings', rest: 'chat' })}>
            Open Chat settings
          </Button>
        </Field>
      </Group>

      <Group title="Budget" id="budget">
        <BudgetRows />
        <Note>Mode counts tokens, never money, and never uses a claude.ai sign-in: the Claude API runs on your own key.</Note>
      </Group>
    </>
  )
}

function LocalModel({ arg, k, title, models }: { arg: keyof AiBackendSetArgs; k: string; title: string; models: string[] }) {
  const [value] = useSetting<string | null>(k)
  const error = useAi((s) => s.errors[k] ?? null)
  return (
    <Field title={title} k={k} error={error ? <SettingError>{error}</SettingError> : undefined}>
      <NullableSelect label={title} value={value} options={[[null, 'not chosen'] as const, ...models.map((m) => [m, m] as const)]} onPick={(v) => v && void useAi.getState().set({ [arg]: v } as AiBackendSetArgs)} />
    </Field>
  )
}
