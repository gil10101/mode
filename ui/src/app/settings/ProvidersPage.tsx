// Providers and routes on Settings → AI (06 §10.1A; ADR-0059): the registry
// of endpoints Mode's own features and an agent's sessions may call (the
// three built-ins first, then the user's), each row with its ⋮ (set the
// key, test, edit, turn off, remove), the add / edit dialog with the fields
// of its kind, and one route per feature (chat, scoring, summaries): the
// provider, the model and up to three backups tried in order. Secrets go to
// the keyring and never come back (XC-22); the model name appears in the
// rows only as the user typed it.
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { BrandMark, brandForProvider } from '../../kit/brands'
import { Button } from '../../kit/button'
import { Chip, ChipsField, Switch, TextArea } from '../../kit/controls'
import { Icon } from '../../kit/icons'
import { ModelPicker, SettingError, useModelChoice } from '../../kit/settings'
import { ConnectorRow } from '../../kit/views'
import type { AiProviderDraft, AiProviderKindRow, AiProviderRow, AiPurpose, AiRoute, AiRouteTarget, ProviderAuth, ProviderKind } from '../../state/contract-connectors'
import type { FieldError } from '../../state/contract-automations'
import type { AiTestResult } from '../../state/contract-settings'
import { Dialog, openMenu, type MenuEntry } from '../ui'
import { useSessionsStore } from '../../state/sessions-store'
import { callableProviders, providerHost, useAi } from './ai'
import { Block, Field, Group, Note, NullableSelect } from './controls'

export const PURPOSES: readonly [AiPurpose, string][] = [
  ['chat', 'Chat'],
  ['scoring', 'Scoring'],
  ['summary', 'Summaries'],
]
const AUTH_LABEL: Record<ProviderAuth, string> = { none: 'No secret', api_key: 'API key', bearer: 'Bearer token' }
const KIND_FALLBACK: Record<ProviderKind, string> = { claude_api: 'Anthropic API', openai_compat: 'OpenAI-compatible', ollama: 'Ollama', lm_studio: 'LM Studio', bedrock: 'Amazon Bedrock', vertex: 'Google Vertex AI', foundry: 'Microsoft Foundry' }
const MAX_BACKUPS = 3

export function kindLabel(kinds: readonly AiProviderKindRow[] | null, kind: ProviderKind): string {
  return kinds?.find((k) => k.kind === kind)?.label ?? KIND_FALLBACK[kind]
}

/** "Anthropic API · openrouter.ai · key stored · chat, summaries · 2 agents" (03 §8.14). */
export function providerDetail(p: AiProviderRow, kinds: readonly AiProviderKindRow[] | null): string {
  const parts: string[] = [kindLabel(kinds, p.kind)]
  const host = providerHost(p.url)
  if (host && !(p.builtin && p.kind === 'claude_api')) parts.push(host)
  if (p.address === 'private') parts.push('on your private network')
  parts.push(p.key_stored ? `key stored${p.key_last4 ? ` · ends in ${p.key_last4}` : ''}` : p.auth === 'none' ? 'no secret needed' : 'no key')
  if (!p.callable) parts.push('sessions only')
  if (p.routed.length) parts.push(p.routed.map((r) => PURPOSES.find(([id]) => id === r)?.[1].toLowerCase() ?? r).join(', '))
  if (p.agents) parts.push(`${p.agents} ${p.agents === 1 ? 'agent' : 'agents'}`)
  return parts.join(' · ')
}

export function emptyProvider(kind: ProviderKind, kinds: readonly AiProviderKindRow[] | null): AiProviderDraft {
  const k = kinds?.find((x) => x.kind === kind)
  return { id: null, kind, name: '', base_url: null, auth: k?.auths[0] ?? 'api_key', models: [], extra: {}, enabled: true }
}

function draftOf(p: AiProviderRow): AiProviderDraft {
  return { id: p.id, kind: p.kind, name: p.name, base_url: p.base_url, auth: p.auth, models: [...p.models], extra: { ...p.extra }, enabled: p.enabled }
}

function Err({ errors, field }: { errors: FieldError[]; field: string }) {
  const e = errors.find((x) => x.field === field)
  if (!e) return null
  return <SettingError>{e.message}</SettingError>
}

/** The refusal of `field`, as a row's error (or nothing). */
function errOf(errors: FieldError[], field: string) {
  return errors.some((x) => x.field === field) ? <Err errors={errors} field={field} /> : undefined
}

/** The add / edit dialog: the fields of the provider's kind (03 §8.14 "Providers"). */
export function ProviderDialog({ draft: initial, onClose }: { draft: AiProviderDraft; onClose: (saved: AiProviderRow | null) => void }) {
  const kinds = useAi((s) => s.kinds)
  const errors = useAi((s) => s.providerErrors)
  const [d, setD] = useState<AiProviderDraft>(initial)
  const [key, setKey] = useState('')
  const [busy, setBusy] = useState(false)
  const patch = (p: Partial<AiProviderDraft>) => setD((x) => ({ ...x, ...p }))
  const kind = kinds?.find((k) => k.kind === d.kind)
  const isNew = !d.id
  const local = d.kind === 'ollama' || d.kind === 'lm_studio'
  const save = async () => {
    setBusy(true)
    const s = useAi.getState()
    const row = await s.saveProvider(d)
    if (row && key.trim()) await s.setProviderKey(row.id, key.trim())
    setBusy(false)
    if (row) onClose(row)
  }
  const kindOptions = (kinds ?? []).map((k) => [k.kind, k.label] as const)
  const authOptions = (kind?.auths ?? ['api_key', 'bearer', 'none']).map((a) => [a, AUTH_LABEL[a]] as const)
  const text = (label: string, value: string | null | undefined, onCommit: (v: string | null) => void, placeholder?: string, wide = true) => (
    <input className={wide ? 'm-input sg-wide' : 'm-input'} aria-label={label} placeholder={placeholder} value={value ?? ''} onChange={(e) => onCommit(e.target.value || null)} />
  )
  return (
    <Dialog title={isNew ? 'Add provider' : `Edit ${initial.name}`} onClose={() => onClose(null)} testId="provider-dialog">
      <Group compact>
        <Field title="Kind">
          {isNew ? (
            <NullableSelect label="Kind" value={d.kind} options={kindOptions} onPick={(k) => patch({ ...emptyProvider(k, kinds), name: d.name })} />
          ) : (
            <span className="m-set-unit">{kindLabel(kinds, d.kind)}</span>
          )}
        </Field>
        <Field title="Name" error={errOf(errors, 'name')}>
          {text('Name', d.name, (v) => patch({ name: v ?? '' }), 'OpenRouter', false)}
        </Field>
        <Field
          title="Address"
          desc={local ? 'This computer or your private network.' : d.kind === 'claude_api' ? 'Empty for api.anthropic.com; otherwise a gateway speaking the Anthropic API.' : d.kind === 'openai_compat' ? 'The base before /v1.' : 'Optional: a gateway in front of the cloud.'}
          error={errOf(errors, 'base_url')}
        >
          {text('Address', d.base_url, (v) => patch({ base_url: v }), kind?.default_url ?? (local ? 'http://127.0.0.1:11434' : 'https://…'))}
        </Field>
        {d.kind === 'bedrock' || d.kind === 'vertex' ? (
          <Field title="Region" error={errOf(errors, 'extra.region')}>
            {text('Region', d.extra.region, (v) => patch({ extra: { ...d.extra, region: v } }), d.kind === 'bedrock' ? 'us-east-1' : 'us-east5', false)}
          </Field>
        ) : null}
        {d.kind === 'vertex' ? (
          <Field title="Project" error={errOf(errors, 'extra.project_id')}>
            {text('Project', d.extra.project_id, (v) => patch({ extra: { ...d.extra, project_id: v } }), 'my-project', false)}
          </Field>
        ) : null}
        {d.kind === 'foundry' ? (
          <Field title="Resource" error={errOf(errors, 'extra.resource')}>
            {text('Resource', d.extra.resource, (v) => patch({ extra: { ...d.extra, resource: v } }), 'my-foundry-resource', false)}
          </Field>
        ) : null}
        <Field title="Secret" desc="How the key is sent." error={errOf(errors, 'auth')}>
          <NullableSelect label="Secret" value={d.auth} options={authOptions} onPick={(a) => patch({ auth: a })} />
        </Field>
        {d.auth !== 'none' && (
          <Field title={d.auth === 'bearer' ? 'Token' : 'Key'} desc="Kept in the OS keyring, never shown again." error={errOf(errors, 'key')}>
            <input className="m-input" type="password" autoComplete="off" spellCheck={false} aria-label="Secret value" placeholder={isNew ? '' : 'unchanged'} value={key} onChange={(e) => setKey(e.target.value)} />
          </Field>
        )}
        <Field title="Models" desc="The first is the default." stack error={errOf(errors, 'models')}>
          <ChipsField label="Models" values={d.models} max={20} placeholder="a model id" onChange={(models) => patch({ models })} />
        </Field>
        {d.kind !== 'openai_compat' && (
          <Field title="Headers" desc="One Name: value per line, sent by sessions." stack error={errOf(errors, 'extra.custom_headers')}>
            <TextArea aria-label="Custom headers" value={d.extra.custom_headers ?? ''} onChange={(e) => patch({ extra: { ...d.extra, custom_headers: e.target.value || null } })} />
          </Field>
        )}
        <Field title="On" desc={kind ? `${kind.callable ? "Mode's own calls" : 'Sessions only'}${kind.session_capable && kind.callable ? ' and sessions' : ''}.` : undefined} error={errOf(errors, 'form')}>
          <Switch on={d.enabled} label="Provider on" onChange={(enabled) => patch({ enabled })} />
        </Field>
      </Group>
      <div className="m-card-actions">
        <Button onClick={() => onClose(null)}>Cancel</Button>
        <Button variant="primary" disabled={busy} onClick={() => void save()}>
          {busy ? 'Saving…' : isNew ? 'Add provider' : 'Save'}
        </Button>
      </div>
    </Dialog>
  )
}

function KeyDialog({ row, onClose }: { row: AiProviderRow; onClose: () => void }) {
  const [value, setValue] = useState('')
  const errors = useAi((s) => s.providerErrors)
  const write = async (v: string | null) => {
    const r = await useAi.getState().setProviderKey(row.id, v)
    if (r) onClose()
  }
  return (
    <Dialog title={`${row.name} ${row.auth === 'bearer' ? 'token' : 'key'}`} onClose={onClose} testId="provider-key">
      <Group compact>
        <Field title={row.auth === 'bearer' ? 'Token' : 'Key'} desc={row.key_stored ? `Stored${row.key_last4 ? `, ends in ${row.key_last4}` : ''}.` : 'Not stored yet.'} error={errOf(errors, 'key')}>
          <input className="m-input" type="password" autoComplete="off" spellCheck={false} aria-label={`${row.name} secret`} value={value} onChange={(e) => setValue(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && value.trim() && void write(value.trim())} />
        </Field>
      </Group>
      <div className="m-card-actions">
        <Button onClick={onClose}>Cancel</Button>
        {row.key_stored && <Button onClick={() => void write(null)}>Remove</Button>}
        <Button variant="primary" disabled={!value.trim()} onClick={() => void write(value.trim())}>
          Save
        </Button>
      </div>
    </Dialog>
  )
}

/** The provider rows and the add button (the "providers" group of Settings → AI). */
export function ProvidersGroup() {
  const providers = useAi((s) => s.providers)
  const kinds = useAi((s) => s.kinds)
  const formError = useAi((s) => s.providerErrors.find((e) => e.field === 'form')?.message ?? null)
  const [dialog, setDialog] = useState<{ draft: AiProviderDraft } | { key: AiProviderRow } | null>(null)
  const [tests, setTests] = useState<Record<string, AiTestResult | 'busy'>>({})
  const test = async (p: AiProviderRow) => {
    setTests((t) => ({ ...t, [p.id]: 'busy' }))
    const r = await useAi.getState().testProvider(p.id)
    setTests((t) => ({ ...t, [p.id]: r }))
  }
  const menu = (p: AiProviderRow): MenuEntry[] => [
    { id: 'key', label: p.key_stored ? 'Replace key…' : 'Set key…', dimmed: p.auth === 'none', reason: p.auth === 'none' ? 'This provider takes no secret' : undefined, run: () => setDialog({ key: p }) },
    { id: 'test', label: 'Test call', dimmed: !p.callable || !p.enabled, reason: !p.callable ? 'Runs sessions only; launch an agent on it' : !p.enabled ? 'Turned off' : undefined, run: () => void test(p) },
    { id: 'edit', label: 'Edit…', run: () => setDialog({ draft: draftOf(p) }) },
    { kind: 'sep', id: 'sep' },
    { id: 'toggle', label: p.enabled ? 'Turn off' : 'Turn on', run: () => void useAi.getState().saveProvider({ ...draftOf(p), enabled: !p.enabled }) },
    {
      id: 'remove',
      label: 'Remove',
      danger: true,
      dimmed: p.builtin,
      reason: p.builtin ? 'Built in; turn it off instead' : undefined,
      run: () =>
        void useAi
          .getState()
          .deleteProvider(p.id)
          .then((gone) => gone && useSessionsStore.getState().showNotice(`${p.name} removed; its routes and agents fall back.`)),
    },
  ]
  // The service's own mark where Mode can name it (10 §7.4.16); the integrations icon otherwise, never another brand's logo.
  const mark = (p: AiProviderRow): ReactNode => {
    const b = brandForProvider(p)
    return b ? <BrandMark id={b} size={16} label="" /> : <Icon id="integrations" />
  }
  return (
    <Group title="Providers" id="providers">
      <Block testId="providers">
        {providers && providers.length > 0 ? (
          <div className="m-list" role="list" aria-label="Providers">
            {providers.map((p) => {
              const t = tests[p.id]
              const line = t === 'busy' ? 'testing…' : t ? (t.ok ? `answered in ${t.latency_ms ?? '?'} ms` : `could not reach it (${t.error ?? 'no answer'})`) : null
              return (
                <ConnectorRow
                  key={p.id}
                  state={!p.enabled ? 'idle' : p.key_stored || p.auth === 'none' ? 'connected' : 'pending'}
                  stateLabel={!p.enabled ? 'off' : p.key_stored || p.auth === 'none' ? 'ready' : 'no key'}
                  mark={mark(p)}
                  name={p.name}
                  detail={line ? `${providerDetail(p, kinds)} · ${line}` : providerDetail(p, kinds)}
                  className={!p.enabled ? 'is-off' : undefined}
                  menuLabel={`${p.name} options`}
                  onMore={(e) => openMenu(e.currentTarget, menu(p), p.name)}
                />
              )
            })}
          </div>
        ) : (
          <div className="m-set-desc">{providers === null ? 'The provider registry is not available in this core.' : 'No providers.'}</div>
        )}
      </Block>
      <Field title="Add provider" desc="Gateways, OpenAI-compatible endpoints, local servers and clouds." error={formError ? <SettingError>{formError}</SettingError> : undefined}>
        <Button disabled={providers === null} onClick={() => setDialog({ draft: emptyProvider('claude_api', kinds) })}>
          Add provider
        </Button>
      </Field>
      {dialog && 'draft' in dialog && <ProviderDialog draft={dialog.draft} onClose={() => setDialog(null)} />}
      {dialog && 'key' in dialog && <KeyDialog row={dialog.key} onClose={() => setDialog(null)} />}
    </Group>
  )
}

function BackupDialog({ providers, exclude, onClose, onAdd }: { providers: AiProviderRow[]; exclude: string[]; onClose: () => void; onAdd: (t: AiRouteTarget) => void }) {
  const choices = providers.filter((p) => !exclude.includes(p.id))
  const [pid, setPid] = useState<string | null>(choices[0]?.id ?? null)
  const [model, setModel] = useState('')
  const [models, setModels] = useState<string[]>([])
  useEffect(() => {
    if (!pid) return
    let live = true
    void useAi
      .getState()
      .providerModels(pid)
      .then((m) => live && setModels(m))
    return () => {
      live = false
    }
  }, [pid])
  return (
    <Dialog title="Add backup" onClose={onClose} testId="route-backup">
      <Group compact>
        <Field title="Provider">
          <NullableSelect label="Backup provider" value={pid} options={choices.map((p) => [p.id, p.name] as const)} onPick={(id) => (setPid(id), setModel(''))} />
        </Field>
        <ModelPicker id="route-backup-model" label="Backup model" defaultLabel="Provider default" models={models} value={model || null} onChange={(v) => setModel(v ?? '')} />
      </Group>
      <div className="m-card-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" disabled={!pid} onClick={() => pid && (onAdd({ provider_id: pid, model: model.trim() || null }), onClose())}>
          Add backup
        </Button>
      </div>
    </Dialog>
  )
}

/** One route (03 §8.14 "Routes"): the provider, the model and the backups of a feature. */
function RouteRow({ purpose, label, route, providers }: { purpose: AiPurpose; label: string; route: AiRoute; providers: AiProviderRow[] }) {
  const error = useAi((s) => s.routeErrors[purpose] ?? null)
  const [adding, setAdding] = useState(false)
  const [models, setModels] = useState<string[]>([])
  const pid = route.provider_id
  useEffect(() => {
    if (!pid) {
      setModels([])
      return
    }
    let live = true
    void useAi
      .getState()
      .providerModels(pid)
      .then((m) => live && setModels(m))
    return () => {
      live = false
    }
  }, [pid])
  const set = (patch: Partial<AiRoute>) => void useAi.getState().setRoute({ ...route, ...patch })
  // The model as typed in its Model id row; written on a pick, on Enter and when the field is left.
  const [draft, setDraft] = useState<string | null>(route.model)
  useEffect(() => setDraft(route.model), [route.model])
  const model = useModelChoice({
    id: `route-${purpose}-model`,
    label: `${label} model`,
    defaultLabel: 'Provider default',
    models,
    value: draft,
    onChange: setDraft,
    onCommit: (v) => v !== route.model && set({ model: v }),
  })
  const nameOf = (id: string) => providers.find((p) => p.id === id)?.name ?? id
  const options: (readonly [string | null, string])[] = [[null, 'the selected backend'], ...providers.map((p) => [p.id, p.name] as const)]
  const exclude = [pid, ...route.fallbacks.map((t) => t.provider_id)].filter((x): x is string => !!x)
  return (
    <>
      <Field title={label} stack testId={`route-${purpose}`} error={error ? <SettingError>{error}</SettingError> : undefined}>
        <div className="m-inline sg-route-line">
          <NullableSelect label={`${label} provider`} value={pid} options={options} onPick={(id) => set({ provider_id: id, model: null, fallbacks: id ? route.fallbacks : [] })} />
          {pid && model.select}
        </div>
        {pid && (
          <div className="m-inline sg-route-line">
            <span className="m-set-unit">{route.fallbacks.length ? 'then' : 'no backup'}</span>
            {route.fallbacks.map((t, i) => (
              <Chip key={`${t.provider_id}-${i}`} removable onRemove={() => set({ fallbacks: route.fallbacks.filter((_, j) => j !== i) })}>
                {t.model ? `${nameOf(t.provider_id)} · ${t.model}` : nameOf(t.provider_id)}
              </Chip>
            ))}
            {route.fallbacks.length < MAX_BACKUPS && (
              <Button quiet onClick={() => setAdding(true)}>
                Add backup
              </Button>
            )}
          </div>
        )}
        {adding && <BackupDialog providers={providers} exclude={exclude} onClose={() => setAdding(false)} onAdd={(t) => set({ fallbacks: [...route.fallbacks, t] })} />}
      </Field>
      {pid && model.idRow}
    </>
  )
}

/** The three routes (the "routes" group of Settings → AI). */
export function RoutesGroup() {
  const routes = useAi((s) => s.routes)
  const rows = useAi((s) => s.providers)
  // Derived outside the selector: a selector returning a fresh array re-renders without end.
  const providers = useMemo(() => callableProviders(rows), [rows])
  if (!routes) return null
  return (
    <Group title="Routes" id="routes">
      <Note>A route names where a feature's calls go and which backups answer when that endpoint cannot be reached; without one, the selected backend is used. Local-only still keeps every call on this computer or your network.</Note>
      {PURPOSES.map(([purpose, label]) => {
        const r = routes.find((x) => x.purpose === purpose) ?? { purpose, provider_id: null, model: null, fallbacks: [] }
        return <RouteRow key={purpose} purpose={purpose} label={label} route={r} providers={providers} />
      })}
    </Group>
  )
}
