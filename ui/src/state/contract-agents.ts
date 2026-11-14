// The Agents contract (03 §7E; ADR-0058): the shapes of `mode_core::agents`
// as the app window's commands and the `agents.changed` push carry them. An
// agent profile is a reusable launch configuration: the brain (model,
// reasoning effort, backup models), what it knows (instructions, memories,
// skills, tables), its rules (permissions, notifications) and the advanced
// settings; `sessions.profile_id` says which agent a session ran as.
import type { FieldError, PermissionMode } from './contract-automations'

export const AGENT_EVENTS = {
  /** The list after a save, a delete or a memory change. */
  changed: 'agents.changed',
} as const

/** `--effort`; `null` leaves the CLI's default. */
export type Effort = 'low' | 'medium' | 'high' | 'xhigh' | 'max'
export const EFFORTS: readonly Effort[] = ['low', 'medium', 'high', 'xhigh', 'max']
/** `--fallback-model` takes at most three. */
export const MAX_FALLBACKS = 3
/** Model aliases the CLI accepts, offered as completions; any id is allowed. */
export const MODEL_ALIASES: readonly string[] = ['opus', 'sonnet', 'haiku', 'claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5']

/** What the editor saves (`agent_save`); `id` null on a new agent. */
export interface AgentDraft {
  id: string | null
  name: string
  /** The one-line role under the name. */
  title: string | null
  /** Appended to the system prompt. */
  instructions: string | null
  /** The bot mark's accessory (05 §11.2); '' = the plain mark. */
  avatar_variant: string
  model: string | null
  effort: Effort | null
  fallback_models: string[]
  /** On: Mode offers the switch; off: the CLI switches on its own with a notice. */
  ask_before_switching: boolean
  /** The skills left on; [] = every installed skill. */
  skills: string[]
  /** Folders (or files, read as their folder) the agent may read: `--add-dir`. */
  tables: string[]
  permission_mode: PermissionMode | null
  allowed_tools: string[]
  denied_tools: string[]
  /** Off: the sessions of this agent raise no alerts. */
  notify: boolean
  /** The default folder a launch starts in. */
  workspace: string | null
  claude_path: string | null
  extra_args: string[]
  /** `CLAUDE_CONFIG_DIR` of its own (memory, transcripts, settings). */
  isolate_config: boolean
  /** The provider its sessions run on (`ai_providers.id`, 06 §10.1A); null = Claude Code's own sign-in. */
  provider_id: string | null
  /** Repository folders: a launch with no folder starts in the first; the others are `--add-dir`. */
  repos: string[]
  /** MCP servers by name, from the ones Claude Code has configured; [] = the CLI's own MCP config, else `--mcp-config <file> --strict-mcp-config`. */
  connectors: string[]
  /** Short free-text rules, a "Rules" section of the system prompt. */
  rules: string[]
  /** Off: the memories stay out of the system prompt. */
  use_memories: boolean
}

/** A rule is one short instruction. */
export const MAX_RULE = 500

/** An MCP server Claude Code has configured (names only; definitions stay in the core). */
export interface McpServer {
  name: string
  /** Where Claude Code keeps it: `local` (this folder in ~/.claude.json), `project` (.mcp.json), `user`. */
  scope: 'local' | 'project' | 'user' | string
  transport: 'stdio' | 'http' | 'sse' | string
}

export interface MemoryRow {
  id: string
  title: string
  body: string
  updated_at: string
}

export interface AgentUsage {
  sessions: number
  sessions_7d: number
  tokens: number
  tokens_7d: number
  turns: number
}

/** An automation whose session action runs the agent. */
export interface AutomationUse {
  id: string
  name: string
  enabled: boolean
  last_run_at: string | null
  last_state: 'running' | 'finished' | 'failed' | 'skipped' | string | null
}

/** A row of the list. */
export interface AgentSummary {
  id: string
  name: string
  title: string | null
  avatar_variant: string
  model: string | null
  effort: Effort | null
  notify: boolean
  sessions: number
  /** Sessions still open. */
  live: number
  last_used_at: string | null
  /** An automation runs this agent. */
  automated: boolean
}

/** The edit page (`agent_get`): the draft's fields, flat, plus what the page shows. */
export interface AgentView extends AgentDraft {
  id: string
  memories: MemoryRow[]
  usage: AgentUsage
  automations: AutomationUse[]
  last_used_at: string | null
  created_at: string
  updated_at: string
}

export interface AgentsCommands {
  agents_list: { args: undefined; result: AgentSummary[] }
  agent_get: { args: { id: string }; result: AgentView }
  /** Rejects with `{code: 'invalid', message, fields}` naming the draft's fields. */
  agent_save: { args: { draft: AgentDraft }; result: AgentView }
  /** The agent's sessions keep their rows; automations that ran it run without it. */
  agent_delete: { args: { id: string }; result: boolean }
  agent_memory_save: { args: { profile_id: string; id?: string | null; title: string; body: string }; result: MemoryRow }
  agent_memory_delete: { args: { id: string }; result: boolean }
  /** The skills Claude Code would load for a workspace (`~/.claude/skills`, `<workspace>/.claude/skills`). */
  skills_list: { args: { workspace?: string | null }; result: string[] }
  /** The MCP servers Claude Code has configured, for these folders (the agent's repositories and workspace) and the user scope. */
  mcp_servers_list: { args: { folders?: string[] }; result: McpServer[] }
}

export type { FieldError }
