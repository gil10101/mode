// The Agents view's state (03 §7E): the list with its brain line and usage
// cell, selection and search, the editor's draft with validation, memories
// and the installed skills. Validation mirrors `mode_core::agents::validate`
// so a refused save reads the same inline as a draft refused here.
import { create } from 'zustand'
import { invoke, listen, type Unlisten } from '../tauri'
import { AGENT_EVENTS, MAX_FALLBACKS, MAX_RULE, type AgentDraft, type McpServer, type AgentSummary, type AgentView, type FieldError, type MemoryRow } from './contract-agents'
import { agoLabel, saveErrors } from './automations-store'
import { cmdErrorText } from './contract'

export type SortKey = 'position' | 'name' | 'last_used' | 'sessions'

export function emptyDraft(): AgentDraft {
  return {
    id: null,
    name: '',
    title: null,
    instructions: null,
    avatar_variant: '',
    model: null,
    effort: null,
    fallback_models: [],
    ask_before_switching: true,
    skills: [],
    tables: [],
    permission_mode: null,
    allowed_tools: [],
    denied_tools: [],
    notify: true,
    workspace: null,
    claude_path: null,
    extra_args: [],
    isolate_config: false,
    provider_id: null,
    repos: [],
    connectors: [],
    rules: [],
    use_memories: true,
  }
}

/** The draft of a view (its flat fields). */
export function draftOf(v: AgentView): AgentDraft {
  return {
    id: v.id,
    name: v.name,
    title: v.title,
    instructions: v.instructions,
    avatar_variant: v.avatar_variant,
    model: v.model,
    effort: v.effort,
    fallback_models: [...v.fallback_models],
    ask_before_switching: v.ask_before_switching,
    skills: [...v.skills],
    tables: [...v.tables],
    permission_mode: v.permission_mode,
    allowed_tools: [...v.allowed_tools],
    denied_tools: [...v.denied_tools],
    notify: v.notify,
    workspace: v.workspace,
    claude_path: v.claude_path,
    extra_args: [...v.extra_args],
    isolate_config: v.isolate_config,
    provider_id: v.provider_id ?? null,
    repos: [...(v.repos ?? [])],
    connectors: [...(v.connectors ?? [])],
    rules: [...(v.rules ?? [])],
    use_memories: v.use_memories ?? true,
  }
}

function sameDraft(a: AgentDraft, b: AgentDraft | null): boolean {
  return !!b && JSON.stringify(a) === JSON.stringify(b)
}

/** The inline errors the core would return (03 §7E.2 Validation); empty when the draft can be saved. */
export function validateDraft(d: AgentDraft): FieldError[] {
  const out: FieldError[] = []
  if (!d.name.trim()) out.push({ field: 'name', message: 'Give the agent a name.' })
  else if (d.name.trim().length > 60) out.push({ field: 'name', message: 'At most 60 characters.' })
  if (d.fallback_models.length > MAX_FALLBACKS) out.push({ field: 'fallback_models', message: `At most ${MAX_FALLBACKS} backup models.` })
  const seen = new Set<string>()
  for (const m of d.fallback_models) {
    const t = m.trim()
    if (!t) out.push({ field: 'fallback_models', message: 'A backup model is empty.' })
    else if (seen.has(t) || t === (d.model ?? '').trim()) out.push({ field: 'fallback_models', message: `${t} is listed twice.` })
    seen.add(t)
  }
  for (const a of d.extra_args) {
    const t = a.trim()
    if (t === '--bare') out.push({ field: 'extra_args', message: '--bare would skip hooks and memory (ADR-0021).' })
    else if (t === '--dangerously-skip-permissions') out.push({ field: 'extra_args', message: 'Choose the permission mode instead.' })
  }
  for (const [field, list] of [['repos', d.repos], ['connectors', d.connectors], ['rules', d.rules]] as const) {
    if (list.some((x) => !x.trim())) out.push({ field, message: 'An entry is empty.' })
  }
  if (d.repos.some((r) => r.trim() && !/^(~(\/|$)|\/|[A-Za-z]:[\\/])/.test(r.trim()))) out.push({ field: 'repos', message: 'Use a full path (/… or ~/…).' })
  if (d.rules.some((r) => r.trim().length > MAX_RULE)) out.push({ field: 'rules', message: `A rule is at most ${MAX_RULE} characters.` })
  // One message per field.
  return out.filter((e, i) => out.findIndex((x) => x.field === e.field) === i)
}

/** "opus · high", "the CLI's default model". */
export function brainLine(a: Pick<AgentSummary, 'model' | 'effort'>): string {
  const parts = [a.model ?? "the CLI's default model", a.effort ? `${a.effort} effort` : null].filter(Boolean)
  return parts.join(' · ')
}

/** "12 sessions · 2 h ago", "never used"; "muted" and "automated" after it. */
export function usageLine(a: AgentSummary, now: number): string {
  const parts: string[] = []
  if (a.sessions) parts.push(`${a.sessions} ${a.sessions === 1 ? 'session' : 'sessions'}`)
  const ago = agoLabel(a.last_used_at, now)
  if (ago) parts.push(ago)
  if (!parts.length) parts.push('never used')
  if (a.automated) parts.push('automated')
  if (!a.notify) parts.push('muted')
  return parts.join(' · ')
}

/** A stable tint per agent (the session avatars tint per repo). */
export function agentHue(id: string): number {
  let h = 0
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0
  return h % 360
}

/** "1.2M", "48k", "900". */
export function compactTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1).replace(/\.0$/, '')}M`
  if (n >= 1000) return `${Math.round(n / 1000)}k`
  return String(n)
}

export interface EditorState {
  draft: AgentDraft
  /** The saved version (`null` for a new agent). */
  base: AgentDraft | null
  /** What the page shows beside the draft (memories, usage, automations). */
  view: AgentView | null
  errors: FieldError[]
  saving: boolean
}

export interface AgentsState {
  rows: AgentSummary[]
  loaded: boolean
  error: string | null
  selectedId: string | null
  search: string
  sort: SortKey
  editor: EditorState | null
  /** Installed skills per workspace ('' = the user's alone). */
  skills: Record<string, string[]>
  /** Configured MCP servers per folder set (key: the folders joined by newlines). */
  mcp: Record<string, McpServer[]>
  reset(): void
  load(): Promise<void>
  setRows(rows: AgentSummary[]): void
  select(id: string | null): void
  setSearch(q: string): void
  setSort(s: SortKey): void
  visible(): AgentSummary[]
  /** `agent_get` into the editor; false when the agent is gone. */
  open(id: string): Promise<boolean>
  /** A new draft, or the editor on a loaded view. */
  edit(target: AgentView | null): void
  closeEditor(): void
  patchDraft(patch: Partial<AgentDraft>): void
  dirty(): boolean
  save(): Promise<AgentView | null>
  remove(id: string): Promise<boolean>
  memorySave(title: string, body: string, id?: string | null): Promise<MemoryRow | null>
  memoryDelete(id: string): Promise<boolean>
  loadSkills(workspace: string | null): Promise<string[]>
  loadMcp(folders: string[]): Promise<McpServer[]>
}

const initial = {
  rows: [] as AgentSummary[],
  loaded: false,
  error: null as string | null,
  selectedId: null as string | null,
  search: '',
  sort: 'position' as SortKey,
  editor: null as EditorState | null,
  skills: {} as Record<string, string[]>,
  mcp: {} as Record<string, McpServer[]>,
}

function sorted(rows: AgentSummary[], sort: SortKey): AgentSummary[] {
  const out = [...rows]
  if (sort === 'name') out.sort((a, b) => a.name.localeCompare(b.name))
  else if (sort === 'last_used') out.sort((a, b) => (b.last_used_at ?? '').localeCompare(a.last_used_at ?? ''))
  else if (sort === 'sessions') out.sort((a, b) => b.sessions - a.sessions)
  return out
}

/** The rows the list shows: sorted, then filtered by the search (name, title, model, effort). */
export function visibleRows(rows: AgentSummary[], search: string, sort: SortKey): AgentSummary[] {
  const q = search.trim().toLowerCase()
  const out = sorted(rows, sort)
  if (!q) return out
  return out.filter((r) => `${r.name} ${r.title ?? ''} ${r.model ?? ''} ${r.effort ?? ''}`.toLowerCase().includes(q))
}

export const useAgents = create<AgentsState>((set, get) => ({
  ...initial,
  reset() {
    set({ ...initial })
  },
  async load() {
    try {
      const rows = await invoke<AgentSummary[]>('agents_list')
      set({ rows: Array.isArray(rows) ? rows : [], loaded: true, error: null })
    } catch (e) {
      set({ loaded: true, error: cmdErrorText(e) })
    }
  },
  setRows(rows) {
    set({ rows, loaded: true, error: null })
    const ed = get().editor
    if (ed?.view && !rows.some((r) => r.id === ed.view!.id)) set({ editor: null })
  },
  select(id) {
    set({ selectedId: id })
  },
  setSearch(q) {
    set({ search: q })
  },
  setSort(s) {
    set({ sort: s })
  },
  visible() {
    return visibleRows(get().rows, get().search, get().sort)
  },
  async open(id) {
    try {
      const view = await invoke<AgentView>('agent_get', { id })
      get().edit(view)
      return true
    } catch {
      set({ editor: null })
      return false
    }
  },
  edit(target) {
    const draft = target ? draftOf(target) : emptyDraft()
    set({ editor: { draft, base: target ? draftOf(target) : null, view: target, errors: [], saving: false } })
  },
  closeEditor() {
    set({ editor: null })
  },
  patchDraft(patch) {
    const ed = get().editor
    if (!ed) return
    const touched = Object.keys(patch)
    set({ editor: { ...ed, draft: { ...ed.draft, ...patch }, errors: ed.errors.filter((e) => !touched.includes(e.field)) } })
  },
  dirty() {
    const ed = get().editor
    if (!ed) return false
    // A new agent is unsaved once something was typed, not from the first paint.
    return !sameDraft(ed.draft, ed.base ?? emptyDraft())
  },
  async save() {
    const ed = get().editor
    if (!ed || ed.saving) return null
    const errors = validateDraft(ed.draft)
    if (errors.length) {
      set({ editor: { ...ed, errors } })
      return null
    }
    set({ editor: { ...ed, saving: true, errors: [] } })
    try {
      const view = await invoke<AgentView>('agent_save', { draft: ed.draft })
      const base = draftOf(view)
      set({ editor: { ...get().editor!, draft: base, base, view, saving: false } })
      void get().load()
      return view
    } catch (e) {
      set({ editor: { ...get().editor!, saving: false, errors: saveErrors(e) } })
      return null
    }
  },
  async remove(id) {
    try {
      await invoke<boolean>('agent_delete', { id })
      set({ rows: get().rows.filter((r) => r.id !== id), selectedId: get().selectedId === id ? null : get().selectedId })
      if (get().editor?.view?.id === id) set({ editor: null })
      return true
    } catch (e) {
      set({ error: cmdErrorText(e) })
      return false
    }
  },
  async memorySave(title, body, id) {
    const ed = get().editor
    if (!ed?.view) return null
    try {
      const row = await invoke<MemoryRow>('agent_memory_save', { profile_id: ed.view.id, id: id ?? null, title, body })
      const cur = get().editor
      if (cur?.view) {
        const memories = cur.view.memories.some((m) => m.id === row.id) ? cur.view.memories.map((m) => (m.id === row.id ? row : m)) : [...cur.view.memories, row]
        set({ editor: { ...cur, view: { ...cur.view, memories } } })
      }
      return row
    } catch (e) {
      set({ error: cmdErrorText(e) })
      return null
    }
  },
  async memoryDelete(id) {
    try {
      await invoke<boolean>('agent_memory_delete', { id })
      const cur = get().editor
      if (cur?.view) set({ editor: { ...cur, view: { ...cur.view, memories: cur.view.memories.filter((m) => m.id !== id) } } })
      return true
    } catch (e) {
      set({ error: cmdErrorText(e) })
      return false
    }
  },
  async loadSkills(workspace) {
    const key = workspace ?? ''
    try {
      const list = await invoke<string[]>('skills_list', { workspace })
      const skills = Array.isArray(list) ? list : []
      set({ skills: { ...get().skills, [key]: skills } })
      return skills
    } catch {
      return get().skills[key] ?? []
    }
  },
  async loadMcp(folders) {
    const key = folders.join('\n')
    try {
      const list = await invoke<McpServer[]>('mcp_servers_list', { folders })
      const servers = Array.isArray(list) ? list : []
      set({ mcp: { ...get().mcp, [key]: servers } })
      return servers
    } catch {
      return get().mcp[key] ?? []
    }
  },
}))

/** `agents.changed` carries the list. */
export function connectAgents(): Promise<Unlisten> {
  return listen<AgentSummary[]>(AGENT_EVENTS.changed, (rows) => {
    if (Array.isArray(rows)) useAgents.getState().setRows(rows)
    else void useAgents.getState().load()
  })
}
