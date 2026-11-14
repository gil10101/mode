// The AI backends as the connectors core reports them (06 §10;
// `mode_core::connectors::api`): `ai_backends_list` (the backend rows and
// the stored key, never the key itself), `ai_health` (the active backend's
// state, its models, the month's tokens and the budget) and
// `ai_backends_status` (both local servers probed, for the Integrations
// card's lines). Writes go through `ai_backend_set` (the backend, the local
// address with its address policy, the local-only switch, the models) and
// `ai_key_set`; the settings they write are patched into the settings store
// so every control shows them at once. Shared by Settings → AI, Privacy,
// Urgency's budget rows and the AI backends card.
import { useEffect, useState } from 'react'
import { create } from 'zustand'
import { invoke } from '../../tauri'
import { cmdErrorText } from '../../state/contract'
import type { AiBackend, AiBackendRow, AiBackendSetArgs, AiHealth, AiProviderDraft, AiProviderKindRow, AiProviderRow, AiRoute, CoreSecretStatus } from '../../state/contract-connectors'
import type { AiBackendsStatus, AiTestResult } from '../../state/contract-settings'
import type { FieldError } from '../../state/contract-automations'
import { useSettingsStore } from '../../state/settings-store'

/** The setting each `ai_backend_set` argument writes. */
export const AI_SET_KEYS: Record<keyof AiBackendSetArgs, string> = {
  backend: 'ai.backend',
  local_url: 'ai.local_url',
  local_only: 'ai.local_only',
  model_scoring: 'ai.model_scoring',
  model_summary: 'ai.model_summary',
  model_chat: 'ai.model_chat',
  local_model_scoring: 'ai.local_model_scoring',
  local_model_summary: 'ai.local_model_summary',
  local_model_chat: 'ai.local_model_chat',
}

/** The settings patch an `ai_backend_set` call stands for (the core writes them; `local_url` loses its trailing slash). */
export function aiSetPatch(args: AiBackendSetArgs): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const [k, v] of Object.entries(args) as [keyof AiBackendSetArgs, unknown][]) {
    if (v === undefined) continue
    out[AI_SET_KEYS[k]] = k === 'local_url' && typeof v === 'string' ? v.replace(/\/+$/, '') : v
  }
  return out
}

export interface AiState {
  rows: AiBackendRow[] | null
  health: AiHealth | null
  /** Both local servers (`ai_backends_status`); null when the core did not answer. */
  status: AiBackendsStatus | null
  loaded: boolean
  /** Refusals by settings key (the address policy) or `key` for the API key. */
  errors: Record<string, string>
  refresh(probe?: boolean): Promise<void>
  /** `ai_backend_set`; resolves false on a refusal (shown under the control). */
  set(args: AiBackendSetArgs): Promise<boolean>
  /** `ai_key_set {backend: 'claude_api', key}`; `null` removes the key. */
  setKey(key: string | null): Promise<CoreSecretStatus | null>

  /* ---- providers and routes (06 §10.1A) ---- */
  /** `ai_providers_list`; null until read (or when the core has no registry). */
  providers: AiProviderRow[] | null
  routes: AiRoute[] | null
  kinds: AiProviderKindRow[] | null
  /** The last `ai_provider_save` refusal's fields. */
  providerErrors: FieldError[]
  /** The last `ai_route_set` refusal, by purpose. */
  routeErrors: Record<string, string>
  loadProviders(): Promise<void>
  /** `ai_provider_save`; null on a refusal (its fields in `providerErrors`). */
  saveProvider(draft: AiProviderDraft): Promise<AiProviderRow | null>
  deleteProvider(id: string): Promise<boolean>
  setProviderKey(id: string, key: string | null): Promise<CoreSecretStatus | null>
  testProvider(id: string): Promise<AiTestResult>
  providerModels(id: string): Promise<string[]>
  /** `ai_route_set`; null on a refusal (its message in `routeErrors[purpose]`). */
  setRoute(route: AiRoute): Promise<AiRoute | null>
}

export const useAi = create<AiState>((set, get) => ({
  rows: null,
  health: null,
  status: null,
  loaded: false,
  errors: {},

  async refresh(probe = false) {
    const [rows, health, status] = await Promise.all([
      invoke<AiBackendRow[]>('ai_backends_list').catch(() => null),
      invoke<AiHealth>('ai_health', { probe }).catch(() => null),
      invoke<AiBackendsStatus>('ai_backends_status').catch(() => null),
    ])
    set({ rows: Array.isArray(rows) ? rows : null, health: health ?? null, status: status ?? null, loaded: true })
    if (get().providers) void get().loadProviders()
  },

  async set(args) {
    const patch = aiSetPatch(args)
    const keys = Object.keys(patch)
    try {
      const health = await invoke<AiHealth>('ai_backend_set', args as unknown as Record<string, unknown>)
      useSettingsStore.getState().patchValues(patch)
      set((s) => {
        const errors = { ...s.errors }
        for (const k of keys) delete errors[k]
        return { health: health ?? s.health, errors }
      })
      void get().refresh()
      return true
    } catch (e) {
      const why = cmdErrorText(e)
      set((s) => ({ errors: { ...s.errors, ...Object.fromEntries(keys.map((k) => [k, why])) } }))
      return false
    }
  },

  async setKey(key) {
    try {
      const r = await invoke<CoreSecretStatus>('ai_key_set', { backend: 'claude_api', key })
      set((s) => {
        const errors = { ...s.errors }
        delete errors.key
        return { errors }
      })
      await get().refresh()
      return r ?? null
    } catch (e) {
      set((s) => ({ errors: { ...s.errors, key: cmdErrorText(e) } }))
      return null
    }
  },

  providers: null,
  routes: null,
  kinds: null,
  providerErrors: [],
  routeErrors: {},

  async loadProviders() {
    const [providers, routes, kinds] = await Promise.all([
      invoke<AiProviderRow[]>('ai_providers_list').catch(() => null),
      invoke<AiRoute[]>('ai_routes_list').catch(() => null),
      get().kinds ? Promise.resolve(get().kinds) : invoke<AiProviderKindRow[]>('ai_provider_kinds').catch(() => null),
    ])
    set({ providers: Array.isArray(providers) ? providers : null, routes: Array.isArray(routes) ? routes : null, kinds: Array.isArray(kinds) ? kinds : null })
  },

  async saveProvider(draft) {
    try {
      const row = await invoke<AiProviderRow>('ai_provider_save', { draft })
      set({ providerErrors: [] })
      await get().loadProviders()
      void get().refresh()
      return row ?? null
    } catch (e) {
      const err = e as { fields?: FieldError[] }
      set({ providerErrors: Array.isArray(err?.fields) && err.fields.length ? err.fields : [{ field: 'form', message: cmdErrorText(e) }] })
      return null
    }
  },

  async deleteProvider(id) {
    try {
      const gone = await invoke<boolean>('ai_provider_delete', { id })
      await get().loadProviders()
      void get().refresh()
      return gone === true
    } catch (e) {
      set({ providerErrors: [{ field: 'form', message: cmdErrorText(e) }] })
      return false
    }
  },

  async setProviderKey(id, key) {
    try {
      const r = await invoke<CoreSecretStatus>('ai_provider_key_set', { id, key })
      await get().loadProviders()
      void get().refresh()
      return r ?? null
    } catch (e) {
      set({ providerErrors: [{ field: 'key', message: cmdErrorText(e) }] })
      return null
    }
  },

  async testProvider(id) {
    try {
      return await invoke<AiTestResult>('ai_provider_test', { id })
    } catch (e) {
      return { ok: false, latency_ms: null, error: cmdErrorText(e) }
    }
  },

  async providerModels(id) {
    try {
      const m = await invoke<string[]>('ai_provider_models', { id })
      return Array.isArray(m) ? m : []
    } catch {
      return get().providers?.find((p) => p.id === id)?.models ?? []
    }
  },

  async setRoute(route) {
    try {
      const saved = await invoke<AiRoute>('ai_route_set', { route })
      set((s) => {
        const routeErrors = { ...s.routeErrors }
        delete routeErrors[route.purpose]
        return { routeErrors, routes: (s.routes ?? []).map((r) => (r.purpose === route.purpose ? saved : r)) }
      })
      void get().refresh()
      return saved ?? null
    } catch (e) {
      set((s) => ({ routeErrors: { ...s.routeErrors, [route.purpose]: cmdErrorText(e) } }))
      return null
    }
  },
}))

/** The providers an agent's sessions may run on (06 §10.1A): session-capable and on. */
export function sessionProviders(rows: readonly AiProviderRow[] | null | undefined): AiProviderRow[] {
  return (rows ?? []).filter((p) => p.session_capable && p.enabled)
}

/** The providers a route may name: callable and on. */
export function callableProviders(rows: readonly AiProviderRow[] | null | undefined): AiProviderRow[] {
  return (rows ?? []).filter((p) => p.callable && p.enabled)
}

/** `openrouter.ai`, `localhost:11434`: the host of a provider's address. */
export function providerHost(url: string | null | undefined): string | null {
  if (!url) return null
  return url.replace(/^https?:\/\//, '').replace(/\/.*$/, '').replace(/^127\.0\.0\.1(?=:|$)/, 'localhost')
}

/** The Claude API row of `ai_backends_list`. */
export function claudeRow(rows: readonly AiBackendRow[] | null | undefined): AiBackendRow | undefined {
  return rows?.find((r) => r.backend === 'claude_api')
}

/** The models a local backend serves: from `ai_health` when it is the active one, else from `ai_backends_status`. */
export function localModels(backend: AiBackend, health: AiHealth | null, status: AiBackendsStatus | null): string[] {
  if (health && health.backend === backend && health.models.length) return health.models
  if (backend === 'ollama') return status?.ollama.models ?? []
  if (backend === 'lm_studio') return status?.lm_studio.models ?? []
  return []
}

/** The Claude provider's models when its key is stored (`ai_provider_models`); empty otherwise. Used by every model picker that offers Claude ids (03 §3.5 Model, §8.14 Scoring and Summaries). */
export function useClaudeModels(): string[] {
  const [models, setModels] = useState<string[]>([])
  useEffect(() => {
    let live = true
    invoke<AiProviderRow[]>('ai_providers_list')
      .then((rows) => {
        const claude = Array.isArray(rows) ? rows.find((r) => r.kind === 'claude_api' && r.key_stored && r.enabled) : undefined
        return claude ? invoke<string[]>('ai_provider_models', { id: claude.id }) : []
      })
      .then((m) => live && setModels(Array.isArray(m) ? m : []))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [])
  return models
}
