// Sample data for the browser mock and the Playwright fixtures: the Sessions
// stage of docs/wireframes/app-window.html (api-refactor, docs-site,
// invoice-parser, readme-tidy, rate-limiter, webhook-spike, scratch) and the
// session of docs/wireframes/session-detail.html (the auth-test fix, waiting
// on a Bash permission with two agents). Everything is plain JSON so the
// Node side of Playwright can import it; times are computed from `now` so
// elapsed labels stay meaningful in a live browser and exact under a fixed
// test clock (14:03:03 on the wireframe's day).
import type { RosterRow } from '../state/contract'
import type {
  AgentRow,
  AppRosterRow,
  ConnectorRow,
  DiffResult,
  FileRow,
  SessionDetail,
  SessionHeader,
  TranscriptItem,
  TranscriptLine,
  TranscriptPage,
} from '../state/contract-sessions'

/** The wireframes' clock: it is 14:03:03 (session-detail.html assumption 15). */
export const SAMPLE_NOW = '2026-10-21T14:03:03.000Z'

const REF_SECONDS = 14 * 3600 + 3 * 60 + 3

/** An instant `hh:mm:ss` of the wireframe's day, placed relative to `now` (so 14:03:03 is `now`). */
export function at(now: number, hhmmss: string): string {
  const [h, m, s] = hhmmss.split(':').map(Number)
  const secs = h * 3600 + m * 60 + (s || 0)
  return new Date(now - (REF_SECONDS - secs) * 1000).toISOString()
}

function ago(now: number, seconds: number): string {
  return new Date(now - seconds * 1000).toISOString()
}

export const HOME = '/Users/ana'
export const MODE_REPO = `${HOME}/code/mode`
export const DOCS_REPO = `${HOME}/code/docs-site`

/** Base roster row; every sample overrides what it needs. */
function row(over: Partial<AppRosterRow> & Pick<RosterRow, 'id' | 'display_name' | 'state'>, now = Date.parse(SAMPLE_NOW)): AppRosterRow {
  return {
    claude_session_id: `${over.id}-0000-4000-8000-000000000000`,
    started_at: ago(now, 3600),
    last_event_at: ago(now, 60),
    ended_at: null,
    acknowledged_at: null,
    last_assistant_preview: null,
    avatar_color: 'avatar.7',
    avatar_variant: '',
    waiting_kind: null,
    title: null,
    cwd: MODE_REPO,
    repo_root: MODE_REPO,
    git_branch: 'main',
    model: 'claude-opus-5-5',
    current_tool: null,
    lines_added: 0,
    lines_removed: 0,
    files_changed: 0,
    tracking: 'hooked',
    launch_mode: 'external',
    state_seconds: 0,
    folded: false,
    live_agents: 0,
    end_reason: null,
    state_reason: null,
    current_mcp_server: null,
    ...over,
  }
}

export function sampleRosterAgents(now: number): AgentRow[] {
  return [
    {
      id: 'a-reviewer',
      agent_id: 'agent-cr-01',
      agent_type: 'code-reviewer',
      state: 'working',
      state_since: ago(now, 40),
      started_at: ago(now, 40),
      ended_at: null,
      current_tool: 'Read',
      current_tool_arg: 'src/auth/limiter.ts',
      tools_used: 6,
      last_assistant_preview: null,
      transcript_path: null,
    },
    {
      id: 'a-runner',
      agent_id: 'agent-tr-01',
      agent_type: 'test-runner',
      state: 'done',
      state_since: ago(now, 3),
      started_at: ago(now, 15),
      ended_at: ago(now, 3),
      current_tool: 'Bash',
      current_tool_arg: 'pytest -q · 48 passed',
      tools_used: 3,
      last_assistant_preview: null,
      transcript_path: null,
    },
  ]
}

/** app-window.html § Sessions: one row per state group, api-refactor waiting with two agents. */
export function sampleRoster(now: number): AppRosterRow[] {
  return [
    row({
      id: 's-api',
      avatar_variant: 'beret',
      display_name: 'api-refactor',
      state: 'waiting',
      waiting_kind: 'permission',
      git_branch: 'feat/api',
      state_seconds: 15 * 60,
      live_agents: 1,
      tracking: 'launched',
      launch_mode: 'embedded',
      lines_added: 43,
      lines_removed: 18,
      files_changed: 4,
      started_at: ago(now, 14 * 60),
      last_event_at: ago(now, 30),
      claude_session_id: '7f3a1c2e-9b4d-4f1e-8a6c-2d5e7f9a1b3c',
      pending: {
        kind: 'permission',
        tool_name: 'Bash',
        tool_input: { command: 'pytest tests/test_auth.py -q' },
        summary: 'Bash: pytest tests/test_auth.py -q',
        received_at: ago(now, 30),
        answerable: true,
      },
      agents: sampleRosterAgents(now),
    }),
    row({
      id: 's-docs',
      avatar_variant: 'specs',
      display_name: 'docs-site',
      state: 'working',
      avatar_color: 'avatar.5',
      cwd: DOCS_REPO,
      repo_root: DOCS_REPO,
      current_tool: 'Edit',
      state_seconds: 2 * 60,
      started_at: ago(now, 25 * 60),
      last_event_at: ago(now, 5),
    }),
    row({
      id: 's-invoice',
      avatar_variant: 'tophat',
      display_name: 'invoice-parser',
      state: 'failed',
      git_branch: 'feat/invoices',
      state_reason: 'pytest exited 1',
      state_seconds: 3600,
      started_at: ago(now, 2 * 3600),
      last_event_at: ago(now, 3600),
    }),
    row({
      id: 's-readme',
      avatar_variant: 'mustache',
      display_name: 'readme-tidy',
      state: 'unread',
      avatar_color: 'avatar.5',
      cwd: DOCS_REPO,
      repo_root: DOCS_REPO,
      lines_added: 3,
      lines_removed: 1,
      files_changed: 1,
      state_seconds: 8 * 60,
      started_at: ago(now, 30 * 60),
      last_event_at: ago(now, 8 * 60),
      ended_at: ago(now, 8 * 60),
      last_assistant_preview: 'Tidied the README headings and fixed the broken install link.',
    }),
    row({
      id: 's-rate',
      avatar_variant: 'beanie',
      display_name: 'rate-limiter',
      state: 'done',
      git_branch: 'feat/api',
      tracking: 'launched',
      launch_mode: 'embedded',
      lines_added: 43,
      lines_removed: 18,
      files_changed: 4,
      state_seconds: 20 * 60,
      started_at: ago(now, 50 * 60),
      last_event_at: ago(now, 20 * 60),
      ended_at: ago(now, 5 * 60),
      claude_session_id: '0d9e4b21-6c3a-4f7e-9a12-5b8c7d6e5f40',
      last_assistant_preview:
        'Retries now back off exponentially (5 attempts, jittered) and the limiter keeps one bucket per client key. Both test files pass locally; CI still needs the fixture from api-refactor before this can merge.',
    }),
    row({
      id: 's-webhook',
      avatar_variant: 'cap',
      display_name: 'webhook-spike',
      state: 'idle',
      git_branch: 'spike/webhooks',
      state_seconds: 86400,
      started_at: ago(now, 26 * 3600),
      last_event_at: ago(now, 86400),
    }),
    row({
      id: 's-scratch',
      avatar_variant: 'squares',
      display_name: 'scratch',
      state: 'untracked',
      avatar_color: 'avatar.10',
      cwd: `${HOME}/code/scratch`,
      repo_root: null,
      git_branch: null,
      model: null,
      tracking: 'discovered',
      state_seconds: 2 * 86400,
      started_at: ago(now, 2 * 86400 + 600),
      last_event_at: ago(now, 2 * 86400),
      ended_at: ago(now, 2 * 86400),
      end_reason: 'inferred',
    }),
  ]
}

/** History rows (dismissed or ended), for the History toggle. */
export function sampleHistory(now: number): AppRosterRow[] {
  return [
    row({
      id: 's-old-auth',
      avatar_variant: 'monocle',
      display_name: 'auth-cleanup',
      state: 'done',
      git_branch: 'chore/auth',
      lines_added: 12,
      lines_removed: 30,
      files_changed: 3,
      state_seconds: 3 * 86400,
      started_at: ago(now, 3 * 86400 + 1800),
      last_event_at: ago(now, 3 * 86400),
      ended_at: ago(now, 3 * 86400),
      dismissed_at: ago(now, 3 * 86400 - 60),
    }),
  ]
}

function header(now: number, over: Partial<SessionHeader> & Pick<SessionHeader, 'id' | 'display_name' | 'state'>): SessionHeader {
  return {
    claude_session_id: '00000000-0000-4000-8000-000000000000',
    title: null,
    avatar_color: 'avatar.7',
    avatar_variant: '',
    state_since: ago(now, 60),
    state_reason: null,
    waiting_kind: null,
    cwd: MODE_REPO,
    repo_root: MODE_REPO,
    git_branch: 'main',
    model: 'claude-opus-5-5',
    permission_mode: 'default',
    tracking: 'hooked',
    launch_mode: 'external',
    current_tool: null,
    current_mcp_server: null,
    transcript_path: null,
    terminal_app: null,
    last_assistant_preview: null,
    lines_added: 0,
    lines_removed: 0,
    files_changed: 0,
    num_turns: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    context_tokens: null,
    started_at: ago(now, 600),
    last_event_at: ago(now, 60),
    ended_at: null,
    end_reason: null,
    acknowledged_at: null,
    dismissed_at: null,
    ...over,
  }
}

const TRANSCRIPT = `${HOME}/.claude/projects/-Users-ana-code-mode/7f3a1c2e-9b4d-4f1e-8a6c-2d5e7f9a1b3c.jsonl`

export function sampleFiles(now: number): FileRow[] {
  return [
    { id: 'f-fixtures', path: `${MODE_REPO}/src/auth/fixtures.py`, rel_path: 'src/auth/fixtures.py', change_kind: 'modified', old_path: null, lines_added: 19, lines_removed: 7, edit_count: 2, source: 'both', last_seen_at: ago(now, 8 * 60) },
    { id: 'f-test', path: `${MODE_REPO}/tests/test_auth.py`, rel_path: 'tests/test_auth.py', change_kind: 'modified', old_path: null, lines_added: 12, lines_removed: 4, edit_count: 1, source: 'both', last_seen_at: ago(now, 4 * 60) },
    { id: 'f-clock', path: `${MODE_REPO}/src/auth/clock.py`, rel_path: 'src/auth/clock.py', change_kind: 'added', old_path: null, lines_added: 12, lines_removed: 0, edit_count: 1, source: 'both', last_seen_at: ago(now, 6 * 60) },
    { id: 'f-init', path: `${MODE_REPO}/src/auth/__init__.py`, rel_path: 'src/auth/__init__.py', change_kind: 'modified', old_path: null, lines_added: 0, lines_removed: 7, edit_count: 1, source: 'git', last_seen_at: ago(now, 8 * 60) },
  ]
}

export function sampleAgents(now: number): AgentRow[] {
  return [
    {
      id: 'a-cr',
      agent_id: 'agent-cr-01',
      agent_type: 'code-reviewer',
      state: 'working',
      state_since: at(now, '14:02:23'),
      started_at: at(now, '14:02:23'),
      ended_at: null,
      current_tool: 'Grep',
      current_tool_arg: 'Session.shared',
      tools_used: 6,
      last_assistant_preview: null,
      transcript_path: `${HOME}/.claude/projects/-Users-ana-code-mode/7f3a1c2e-9b4d-4f1e-8a6c-2d5e7f9a1b3c/subagents/agent-cr-01.jsonl`,
    },
    {
      id: 'a-tr',
      agent_id: 'agent-tr-01',
      agent_type: 'test-runner',
      state: 'done',
      state_since: at(now, '13:58:32'),
      started_at: at(now, '13:58:20'),
      ended_at: at(now, '13:58:32'),
      current_tool: null,
      tools_used: 3,
      last_assistant_preview: '48 passed in each of ten runs, 0.9 s per run.',
      transcript_path: `${HOME}/.claude/projects/-Users-ana-code-mode/7f3a1c2e-9b4d-4f1e-8a6c-2d5e7f9a1b3c/subagents/agent-tr-01.jsonl`,
    },
  ]
}

export function sampleConnectors(now: number): ConnectorRow[] {
  const seen = at(now, '14:02:33')
  return [
    { kind: 'mcp_server', name: 'github', status: 'connected', detail_json: JSON.stringify({ tools: 12, detail: 'stdio · github-mcp 1.4' }), seen_at: seen },
    { kind: 'mcp_server', name: 'filesystem', status: 'connected', detail_json: JSON.stringify({ tools: 8, detail: 'stdio · ~/code/mode' }), seen_at: seen },
    { kind: 'mcp_server', name: 'linear', status: 'pending', detail_json: JSON.stringify({ tools: 0, detail: 'starting · not signed in yet' }), seen_at: seen },
    { kind: 'mcp_server', name: 'notion', status: 'failed', detail_json: JSON.stringify({ detail: 'auth expired · reconnect in Integrations' }), seen_at: seen },
    { kind: 'tool_rule', name: 'Bash:git', status: 'allowed', detail_json: JSON.stringify({ origin: 'always rule' }), seen_at: seen },
    { kind: 'tool_rule', name: 'Bash:rm', status: 'denied', detail_json: JSON.stringify({ origin: 'denied in Mode' }), seen_at: seen },
    { kind: 'plugin', name: 'code-review', status: 'connected', detail_json: JSON.stringify({ version: '1.4', detail: '1 agent (code-reviewer) · 2 skills' }), seen_at: seen },
    { kind: 'relay', name: 'mode-hook', status: 'connected', detail_json: JSON.stringify({ last_event_at: seen, relay_version: '0.1.0', hooks: 5 }), seen_at: seen },
  ]
}

/** The Overview thread of session-detail.html §3, oldest first. */
export function sampleThread(now: number): TranscriptItem[] {
  return [
    { seq: 1, at: at(now, '13:49:40'), kind: 'user', text: 'Make the auth test deterministic. It fails about one run in five on CI.' },
    {
      seq: 2,
      at: at(now, '13:49:55'),
      kind: 'assistant',
      text: 'The flake comes from a shared fixture: `client` reuses one database across tests and the token clock drifts. I will pin the clock and give each test its own transaction; two files change.',
    },
    {
      seq: 3,
      at: at(now, '13:50:01'),
      kind: 'tool',
      tool_use_id: 'tu-read-1',
      tool_name: 'Read',
      summary: 'tests/test_auth.py',
      meta: '84 lines',
      body: '@pytest.fixture\ndef client():\n    db = Session.shared()\n    yield TestClient(app, db=db)\n    db.rollback()',
    },
    {
      seq: 4,
      at: at(now, '13:51:21'),
      kind: 'tool',
      tool_use_id: 'tu-bash-1',
      tool_name: 'Bash',
      summary: 'for i in $(seq 10); do pytest tests/test_auth.py -q; done',
      meta: '1 failed · 23 passed',
      body: 'FAILED tests/test_auth.py::test_refresh_retries - TimeoutError\n1 failed, 23 passed in 0.94s   (2 of 10 runs)',
    },
    {
      seq: 5,
      at: at(now, '13:52:30'),
      kind: 'tool',
      tool_use_id: 'tu-edit-1',
      tool_name: 'Edit',
      summary: 'src/auth/fixtures.py',
      lines_added: 19,
      lines_removed: 7,
      body: '-def client():\n-    db = Session.shared()\n+def client(clock):\n+    db = Session.begin_nested()',
    },
    {
      seq: 6,
      at: at(now, '13:53:02'),
      kind: 'question',
      tool_use_id: 'tu-ask-1',
      status: 'answered',
      answered_where: 'app',
      answered_at: at(now, '13:54:10'),
      questions: [
        {
          question: 'How should the fixture pin the clock?',
          header: 'Clock',
          options: [{ label: 'Freeze at a fixed instant' }, { label: 'Advance manually per test' }],
          multiSelect: false,
        },
        { question: 'Also apply the clock to other suites?', header: 'Also apply to', options: [{ label: 'Webhook tests' }], multiSelect: true },
      ],
      answers: { 'How should the fixture pin the clock?': 'Freeze at a fixed instant', 'Also apply the clock to other suites?': ['Webhook tests'] },
    },
    {
      seq: 7,
      at: at(now, '14:02:23'),
      kind: 'agent',
      agent_id: 'agent-cr-01',
      agent_type: 'code-reviewer',
      state: 'working',
      started_at: at(now, '14:02:23'),
      ended_at: null,
      tools: [
        { tool_name: 'Read', summary: 'src/auth/fixtures.py' },
        { tool_name: 'Read', summary: 'src/auth/clock.py' },
        { tool_name: 'Read', summary: 'src/auth/__init__.py' },
        { tool_name: 'Read', summary: 'tests/test_auth.py' },
        { tool_name: 'Read', summary: 'tests/test_webhooks.py' },
        { tool_name: 'Grep', summary: 'Session.shared · 2 matches' },
      ],
    },
    { seq: 8, at: at(now, '14:02:33'), kind: 'permission', tool_use_id: null, tool_name: 'Bash', command: 'pytest tests/test_auth.py -q', status: 'open' },
    // The code-reviewer's own thread (the Agents drill-in).
    { seq: 101, at: at(now, '14:02:23'), agent_id: 'agent-cr-01', kind: 'user', text: 'Review the fixture change in src/auth/fixtures.py for state that can leak between tests. Report only, do not edit.' },
    { seq: 102, at: at(now, '14:02:25'), agent_id: 'agent-cr-01', kind: 'tool', tool_use_id: 'cr-r1', tool_name: 'Read', summary: 'src/auth/fixtures.py', meta: '27 lines' },
    { seq: 103, at: at(now, '14:02:26'), agent_id: 'agent-cr-01', kind: 'tool', tool_use_id: 'cr-r2', tool_name: 'Read', summary: 'src/auth/clock.py', meta: '12 lines' },
    { seq: 104, at: at(now, '14:02:27'), agent_id: 'agent-cr-01', kind: 'tool', tool_use_id: 'cr-r3', tool_name: 'Read', summary: 'src/auth/__init__.py', meta: '58 lines' },
    { seq: 105, at: at(now, '14:02:28'), agent_id: 'agent-cr-01', kind: 'tool', tool_use_id: 'cr-r4', tool_name: 'Read', summary: 'tests/test_auth.py', meta: '92 lines' },
    { seq: 106, at: at(now, '14:02:29'), agent_id: 'agent-cr-01', kind: 'tool', tool_use_id: 'cr-r5', tool_name: 'Read', summary: 'tests/test_webhooks.py', meta: '61 lines' },
    {
      seq: 107,
      at: at(now, '14:02:31'),
      agent_id: 'agent-cr-01',
      kind: 'tool',
      tool_use_id: 'cr-g1',
      tool_name: 'Grep',
      summary: 'Session.shared',
      meta: '2 matches',
      body: 'src/auth/__init__.py:41    _db = Session.shared()\ntests/test_webhooks.py:12  db = Session.shared()',
    },
    // The test-runner's thread.
    { seq: 201, at: at(now, '13:58:20'), agent_id: 'agent-tr-01', kind: 'user', text: 'Run the auth and webhook suites ten times each and report any failure.' },
    { seq: 202, at: at(now, '13:58:22'), agent_id: 'agent-tr-01', kind: 'tool', tool_use_id: 'tr-b1', tool_name: 'Bash', summary: 'pytest tests/test_auth.py -q', meta: '24 passed' },
    { seq: 203, at: at(now, '13:58:25'), agent_id: 'agent-tr-01', kind: 'tool', tool_use_id: 'tr-b2', tool_name: 'Bash', summary: 'pytest tests/test_webhooks.py -q', meta: '24 passed' },
    {
      seq: 204,
      at: at(now, '13:58:30'),
      agent_id: 'agent-tr-01',
      kind: 'tool',
      tool_use_id: 'tr-b3',
      tool_name: 'Bash',
      summary: 'for i in $(seq 10); do pytest tests/test_auth.py tests/test_webhooks.py -q; done',
      meta: '48 passed in every run',
    },
    { seq: 205, at: at(now, '13:58:32'), agent_id: 'agent-tr-01', kind: 'assistant', text: '48 passed in each of ten runs, 0.9 s per run. No failure and no clock warning; the fixture no longer leaks state between tests.' },
  ]
}

/** session-detail.html: the waiting embedded session. `display_name` defaults to the roster's api-refactor. */
export function sampleDetail(now: number, over: Partial<SessionHeader> = {}): SessionDetail {
  return {
    session: header(now, {
      id: 's-api',
      avatar_variant: 'beret',
      claude_session_id: '7f3a1c2e-9b4d-4f1e-8a6c-2d5e7f9a1b3c',
      display_name: 'api-refactor',
      title: 'Fix flaky auth test',
      state: 'waiting',
      waiting_kind: 'permission',
      state_since: ago(now, 14 * 60),
      git_branch: 'feat/api',
      tracking: 'launched',
      launch_mode: 'embedded',
      transcript_path: TRANSCRIPT,
      lines_added: 43,
      lines_removed: 18,
      files_changed: 4,
      num_turns: 6,
      input_tokens: 121_000,
      output_tokens: 16_000,
      cache_read_tokens: 840_000,
      cache_write_tokens: 52_000,
      started_at: at(now, '13:49:02'),
      last_event_at: at(now, '14:02:39'),
      ...over,
    }),
    pending: {
      kind: 'permission',
      tool_name: 'Bash',
      tool_input: { command: 'pytest tests/test_auth.py -q' },
      summary: 'Bash: pytest tests/test_auth.py -q',
      received_at: at(now, '14:02:33'),
      answerable: true,
    },
    driver: 'pty',
    alive: true,
    thread: sampleThread(now),
    thread_has_more: false,
    files: sampleFiles(now),
    agents: sampleAgents(now),
    connectors: sampleConnectors(now),
    connectors_source: 'hooks',
    links: [
      { id: 'n-retry', kind: 'note', title: 'Rate limiter retry policy', link_type: 'references', available: true },
      { id: 'm-release', kind: 'message', title: 'Sam · #release · ship v2 behind the flag today?', link_type: 'related', available: true },
    ],
    transcript: { path: TRANSCRIPT, size_bytes: 184_000, exists: true },
  }
}

/** A finished, resumable session: rate-limiter in app-window.html's detail panel. */
export function sampleFinishedDetail(now: number): SessionDetail {
  const files: FileRow[] = [
    { id: 'r-limiter', path: `${MODE_REPO}/src/auth/limiter.ts`, rel_path: 'src/auth/limiter.ts', change_kind: 'modified', old_path: null, lines_added: 22, lines_removed: 9, edit_count: 3, source: 'both', last_seen_at: ago(now, 21 * 60) },
    { id: 'r-bucket', path: `${MODE_REPO}/src/auth/bucket.ts`, rel_path: 'src/auth/bucket.ts', change_kind: 'added', old_path: null, lines_added: 14, lines_removed: 0, edit_count: 1, source: 'both', last_seen_at: ago(now, 22 * 60) },
    { id: 'r-test', path: `${MODE_REPO}/tests/test_limiter.py`, rel_path: 'tests/test_limiter.py', change_kind: 'modified', old_path: null, lines_added: 6, lines_removed: 6, edit_count: 1, source: 'both', last_seen_at: ago(now, 23 * 60) },
    { id: 'r-docs', path: `${MODE_REPO}/docs/api.md`, rel_path: 'docs/api.md', change_kind: 'modified', old_path: null, lines_added: 1, lines_removed: 3, edit_count: 1, source: 'git', last_seen_at: ago(now, 24 * 60) },
  ]
  return {
    session: header(now, {
      id: 's-rate',
      avatar_variant: 'beanie',
      claude_session_id: '0d9e4b21-6c3a-4f7e-9a12-5b8c7d6e5f40',
      display_name: 'rate-limiter',
      state: 'done',
      state_since: ago(now, 20 * 60),
      tracking: 'launched',
      launch_mode: 'embedded',
      git_branch: 'feat/api',
      transcript_path: `${HOME}/.claude/projects/-Users-ana-code-mode/0d9e4b21-6c3a-4f7e-9a12-5b8c7d6e5f40.jsonl`,
      lines_added: 43,
      lines_removed: 18,
      files_changed: 4,
      num_turns: 6,
      input_tokens: 121_000,
      output_tokens: 16_000,
      started_at: ago(now, 50 * 60),
      last_event_at: ago(now, 20 * 60),
      ended_at: ago(now, 5 * 60),
      end_reason: 'prompt_input_exit',
      acknowledged_at: ago(now, 19 * 60),
      last_assistant_preview:
        'Retries now back off exponentially (5 attempts, jittered) and the limiter keeps one bucket per client key. Both test files pass locally; CI still needs the fixture from api-refactor before this can merge.',
    }),
    pending: null,
    driver: 'none',
    alive: false,
    thread: [
      { seq: 1, at: ago(now, 50 * 60), kind: 'start', source: 'startup', model: 'claude-opus-5-5' },
      { seq: 2, at: ago(now, 49 * 60), kind: 'user', text: 'Make the limiter back off on 429s and keep a bucket per client.' },
      { seq: 3, at: ago(now, 21 * 60), kind: 'tool', tool_use_id: 'r1', tool_name: 'Edit', summary: 'src/auth/limiter.ts', lines_added: 22, lines_removed: 9 },
      {
        seq: 4,
        at: ago(now, 20 * 60),
        kind: 'assistant',
        text: 'Retries now back off exponentially (5 attempts, jittered) and the limiter keeps one bucket per client key. Both test files pass locally; CI still needs the fixture from api-refactor before this can merge.',
      },
      { seq: 5, at: ago(now, 5 * 60), kind: 'end', reason: 'prompt_input_exit' },
    ],
    thread_has_more: false,
    files,
    agents: [],
    connectors: [{ kind: 'relay', name: 'mode-hook', status: 'connected', detail_json: JSON.stringify({ last_event_at: ago(now, 20 * 60), hooks: 5 }), seen_at: ago(now, 20 * 60) }],
    connectors_source: 'hooks',
    links: [
      { id: 'n-retry', kind: 'note', title: 'Rate limiter retry policy', link_type: 'references', available: true },
      { id: 'm-release', kind: 'message', title: 'Sam · #release · ship v2 behind the flag today?', link_type: 'related', available: true },
    ],
    transcript: { path: `${HOME}/.claude/projects/-Users-ana-code-mode/0d9e4b21-6c3a-4f7e-9a12-5b8c7d6e5f40.jsonl`, size_bytes: 96_000, exists: true },
  }
}

/** A plain detail built from a roster row, for rows without a hand-written sample. */
export function detailFromRow(now: number, r: AppRosterRow): SessionDetail {
  const ended = r.ended_at ?? null
  return {
    session: header(now, {
      id: r.id,
      claude_session_id: r.claude_session_id ?? r.id,
      display_name: r.display_name,
      title: r.title,
      avatar_color: r.avatar_color,
      avatar_variant: r.avatar_variant,
      state: r.state,
      state_since: ago(now, r.state_seconds),
      state_reason: r.state_reason ?? null,
      waiting_kind: r.waiting_kind,
      cwd: r.cwd,
      repo_root: r.repo_root,
      git_branch: r.git_branch,
      model: r.model,
      tracking: r.tracking,
      launch_mode: r.launch_mode,
      current_tool: r.current_tool,
      lines_added: r.lines_added,
      lines_removed: r.lines_removed,
      files_changed: r.files_changed,
      num_turns: r.state === 'untracked' ? 0 : 3,
      input_tokens: r.state === 'untracked' ? 0 : 42_000,
      output_tokens: r.state === 'untracked' ? 0 : 6_100,
      started_at: r.started_at ?? ago(now, r.state_seconds + 600),
      last_event_at: r.last_event_at ?? ago(now, r.state_seconds),
      ended_at: ended,
      end_reason: r.end_reason ?? null,
      dismissed_at: r.dismissed_at ?? null,
      last_assistant_preview: r.last_assistant_preview ?? null,
      transcript_path: `${HOME}/.claude/projects/-Users-ana-code-${r.display_name}/${r.id}.jsonl`,
    }),
    pending: r.pending ?? null,
    driver: r.launch_mode === 'embedded' && !ended ? 'pty' : 'none',
    alive: !ended,
    thread:
      r.state === 'untracked'
        ? []
        : [
            { seq: 1, at: r.started_at ?? ago(now, r.state_seconds + 600), kind: 'start', source: 'startup', model: r.model },
            ...(r.last_assistant_preview ? [{ seq: 2, at: ago(now, r.state_seconds), kind: 'assistant' as const, text: r.last_assistant_preview }] : []),
          ],
    thread_has_more: false,
    files: [],
    agents: r.agents ?? [],
    connectors: [],
    connectors_source: 'hooks',
    links: [],
    transcript: { path: `${HOME}/.claude/projects/-Users-ana-code-${r.display_name}/${r.id}.jsonl`, size_bytes: 12_000, exists: true },
  }
}

/** session-detail.html's diff of src/auth/fixtures.py. */
export function sampleDiff(): DiffResult {
  return {
    basis: 'working tree vs HEAD',
    lines: [
      { kind: 'hunk', text: '@@ -12,9 +12,19 @@ import pytest' },
      { kind: 'ctx', old_no: 12, new_no: 12, text: '  from app.main import app' },
      { kind: 'ctx', old_no: 13, new_no: 13, text: '  from app.db import Session' },
      { kind: 'add', new_no: 14, text: '+ from app.auth.clock import FrozenClock' },
      { kind: 'ctx', old_no: 14, new_no: 15, text: '  ' },
      { kind: 'ctx', old_no: 15, new_no: 16, text: '  @pytest.fixture' },
      { kind: 'del', old_no: 16, text: '- def client():' },
      { kind: 'del', old_no: 17, text: '-     db = Session.shared()' },
      { kind: 'add', new_no: 17, text: '+ def clock():' },
      { kind: 'add', new_no: 18, text: '+     return FrozenClock("2026-10-21T13:49:00Z")' },
      { kind: 'add', new_no: 19, text: '+ ' },
      { kind: 'add', new_no: 20, text: '+ @pytest.fixture' },
      { kind: 'add', new_no: 21, text: '+ def client(clock):' },
      { kind: 'add', new_no: 22, text: '+     db = Session.begin_nested()' },
      { kind: 'add', new_no: 23, text: '+     app.dependency_overrides[now] = clock.now' },
      { kind: 'ctx', old_no: 18, new_no: 24, text: '      yield TestClient(app, db=db)' },
      { kind: 'del', old_no: 19, text: '-     db.close()' },
      { kind: 'add', new_no: 25, text: '+     db.rollback()' },
      { kind: 'add', new_no: 26, text: '+     clock.reset()' },
      { kind: 'ctx', old_no: 20, new_no: 27, text: '  ' },
      { kind: 'hunk', text: '@@ -41,7 +51,11 @@ def token(client):' },
      { kind: 'ctx', old_no: 41, new_no: 51, text: '      resp = client.post("/auth/token", json=CREDS)' },
      { kind: 'del', old_no: 42, text: '-     assert resp.json()["expires_in"] > 0' },
      { kind: 'add', new_no: 52, text: '+     assert resp.json()["expires_in"] == 3600' },
      { kind: 'ctx', old_no: 43, new_no: 53, text: '      return resp.json()["token"]' },
    ],
  }
}

/** A diff for any other file: the last edit as additions. */
export function genericDiff(file: FileRow): DiffResult {
  const lines = [{ kind: 'hunk' as const, text: `@@ -1,${file.lines_removed} +1,${file.lines_added} @@` }]
  return {
    basis: file.source === 'hook' ? 'last edit' : file.change_kind === 'added' ? 'new file' : 'working tree vs HEAD',
    lines: [
      ...lines,
      ...Array.from({ length: Math.min(file.lines_removed, 6) }, (_, i) => ({ kind: 'del' as const, old_no: i + 1, text: `- line ${i + 1}` })),
      ...Array.from({ length: Math.min(file.lines_added, 6) }, (_, i) => ({ kind: 'add' as const, new_no: i + 1, text: `+ line ${i + 1}` })),
    ],
  }
}

/** The Transcript tab: one block per JSONL line, the subagent files as nested sections. */
export function sampleTranscript(now: number, path: string | null): TranscriptPage {
  const line = (n: number, t: string, type: string, label: string, text: string, extra: Partial<TranscriptLine> = {}): TranscriptLine => ({
    line: n,
    at: at(now, t),
    type,
    label,
    text,
    content: extra.content ?? text,
    raw: JSON.stringify({ type, timestamp: at(now, t), message: { role: type, content: text } }),
    ...extra,
  })
  return {
    path,
    size_bytes: 184_000,
    exists: true,
    has_more: false,
    subagent_files: 2,
    lines: [
      line(1, '13:49:02', 'system', 'system', 'startup · ~/code/mode'),
      line(2, '13:49:40', 'user', 'user', 'Make the auth test deterministic. It fails about one run in five on CI.'),
      line(3, '13:49:55', 'assistant', 'assistant', 'The flake comes from a shared fixture: `client` reuses one database across tests and the token clock drifts.', {
        usage: { input_tokens: 18_200, output_tokens: 410 },
      }),
      line(4, '13:50:01', 'assistant', 'tool_use · Read', 'tests/test_auth.py', { usage: { input_tokens: 18_900, output_tokens: 60 } }),
      line(5, '13:50:01', 'user', 'tool_result', 'Read 84 lines'),
      line(6, '13:51:21', 'assistant', 'tool_use · Bash', 'for i in $(seq 10); do pytest tests/test_auth.py -q; done', { usage: { input_tokens: 20_400, output_tokens: 75 } }),
      line(7, '13:52:01', 'user', 'tool_result', '1 failed, 23 passed (2 of 10 runs)'),
      line(8, '13:53:02', 'assistant', 'tool_use · AskUserQuestion', 'How should the fixture pin the clock? · Also apply to'),
      line(9, '13:54:10', 'user', 'tool_result', 'Freeze at a fixed instant · Webhook tests'),
      line(10, '13:54:48', 'assistant', 'tool_use · Edit', 'src/auth/fixtures.py', { usage: { input_tokens: 22_100, output_tokens: 820 } }),
      line(11, '13:56:10', 'assistant', 'assistant', 'Pinned the clock in the fixture and isolated the database per test; the suite runs in 0.9 s.', {
        usage: { input_tokens: 24_000, output_tokens: 160 },
      }),
      line(12, '13:58:02', 'user', 'user', 'Run the suite once more and add the clock to the webhook tests too.'),
      line(1, '13:58:20', 'user', 'user', 'Run the auth and webhook suites ten times each and report any failure.', { agent_id: 'agent-tr-01', agent_type: 'test-runner' }),
      line(2, '13:58:32', 'assistant', 'assistant', '48 passed in each of ten runs, 0.9 s per run.', { agent_id: 'agent-tr-01', agent_type: 'test-runner', usage: { input_tokens: 9_800, output_tokens: 120 } }),
      line(1, '14:02:23', 'user', 'user', 'Review the fixture change in src/auth/fixtures.py for state that can leak between tests.', { agent_id: 'agent-cr-01', agent_type: 'code-reviewer' }),
    ],
  }
}
