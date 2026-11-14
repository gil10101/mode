// Settings → Chat (03 §8.21; ADR-0055, ADR-0008 as amended 2026-11-08): the
// Chat model (moved from the AI page; written through `ai_backend_set` like
// every model, with the Chat route's provider models and the CLI aliases as
// the choices), the reply timeout (`ai.timeout_seconds.chat`, a field of an
// object key the core validates), how long conversations are kept
// (`retention.chat_days`, the nightly sweep) and whether the sources panel
// opens by default (`chat.sources_open`).
import { useEffect, useState } from 'react'
import { SettingError } from '../../kit/settings'
import { MODEL_ALIASES } from '../../state/contract-agents'
import type { AiBackendSetArgs } from '../../state/contract-connectors'
import type { AiBackend } from '../../state/contract-settings'
import { useSetting } from '../../state/settings-store'
import { localModels, useAi } from './ai'
import { AiModel } from './AiPage'
import { Field, Group, NullableSelect, SettingNumber, SettingSwitch } from './controls'

/** The Model row's choices: the stored value first (so it always shows), then the provider's models and the aliases; the Claude picker passes no stored value (a custom one is its Other… id). */
export function chatModelOptions(current: string | null, provider: readonly string[], aliases: readonly string[] = MODEL_ALIASES): string[] {
  return Array.from(new Set([...(current ? [current] : []), ...provider, ...aliases]))
}

export function ChatPage() {
  const [backend] = useSetting<AiBackend>('ai.backend')
  const routes = useAi((s) => s.routes)
  const health = useAi((s) => s.health)
  const status = useAi((s) => s.status)
  const pid = routes?.find((r) => r.purpose === 'chat')?.provider_id ?? null
  const [providerModels, setProviderModels] = useState<string[]>([])
  useEffect(() => {
    const s = useAi.getState()
    if (!s.loaded) void s.refresh()
    if (!s.routes) void s.loadProviders()
  }, [])
  useEffect(() => {
    if (!pid) {
      setProviderModels([])
      return
    }
    let live = true
    void useAi
      .getState()
      .providerModels(pid)
      .then((m) => live && setProviderModels(m))
    return () => {
      live = false
    }
  }, [pid])

  const local = backend === 'ollama' || backend === 'lm_studio'
  const k = local ? 'ai.local_model_chat' : 'ai.model_chat'
  const arg: keyof AiBackendSetArgs = local ? 'local_model_chat' : 'model_chat'
  const [model] = useSetting<string | null>(k)
  const error = useAi((s) => s.errors[k] ?? null)
  const localOptions: (readonly [string | null, string])[] = [[null, 'not chosen'] as const, ...chatModelOptions(model, localModels(backend, health, status), []).map((m) => [m, m] as const)]

  return (
    <Group title="Conversations" id="conversations">
      {local ? (
        <Field title="Model" desc="What Chat answers with." k={k} error={error ? <SettingError testId={`error-${k}`}>{error}</SettingError> : undefined}>
          <NullableSelect label="Chat model" value={model} options={localOptions} onPick={(v) => v && void useAi.getState().set({ [arg]: v } as AiBackendSetArgs)} testId="chat-model" />
        </Field>
      ) : (
        // The Claude model: the picker with the default first, the Chat route's provider models and the aliases, then Other… (03 §8.21).
        <AiModel arg="model_chat" k="ai.model_chat" title="Model" label="Chat model" desc="What Chat answers with." models={chatModelOptions(null, providerModels)} testId="chat-model" />
      )}
      <SettingNumber
        k="ai.timeout_seconds"
        title="Timeout"
        desc="How long a reply may take before Chat stops waiting."
        unit="seconds"
        label="Chat timeout"
        read={(v) => (v as { chat?: number } | null)?.chat ?? null}
        write={(n, v) => ({ ...((v as object | null) ?? {}), chat: n })}
      />
      <SettingNumber k="retention.chat_days" title="Keep conversations" desc="Older conversations are deleted on the nightly sweep." unit="days" />
      <SettingSwitch k="chat.sources_open" title="Sources panel" desc="Open by default; the chevron in the Chat head changes it too." label="Open the sources panel by default" />
    </Group>
  )
}
