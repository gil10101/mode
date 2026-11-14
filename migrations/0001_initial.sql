-- 0001_initial: the complete v0.1 schema of docs/09-data-model.md (§5 tables, §6 FTS5,
-- §7 indexes, §9 change_log triggers, §12 seeds).
--
-- Conventions (09 §4): UUID v7 text ids, ISO-8601 UTC millisecond timestamps, local
-- dates as YYYY-MM-DD, booleans as INTEGER CHECK (x IN (0,1)), enums as TEXT CHECK,
-- JSON as TEXT CHECK (json_valid(x)), STRICT tables, plural snake_case names.
-- Sync class S tables end with created_at, updated_at, rev, device_id, deleted_at and
-- write to change_log through the triggers at the end of this file.
--
-- Tokens replaced by the migrator before execution (never bind parameters: a migration
-- is executed as one batch): __DEVICE_ID__, __DEVICE_NAME__, __PROFILE_NAME__,
-- __CAPTURE_HOTKEY__, __TOGGLE_HOTKEY__, __PIN_HOTKEY__. The checksum ledgered in
-- schema_migrations is over this text as embedded, before substitution.

-- ---------------------------------------------------------------------------
-- 5.6 Infrastructure
-- ---------------------------------------------------------------------------

CREATE TABLE schema_migrations (
  version      INTEGER PRIMARY KEY,                -- 1, 2, 3 … matches the file prefix
  name         TEXT NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('ddl','data')),
  checksum     TEXT NOT NULL,                      -- sha256 of the migration text as embedded in the binary
  app_version  TEXT NOT NULL,                      -- Mode version that applied it
  applied_at   TEXT NOT NULL
) STRICT;

CREATE TABLE change_log (
  seq         INTEGER PRIMARY KEY,                 -- global, monotonic; the future sync cursor
  table_name  TEXT NOT NULL,
  row_id      TEXT NOT NULL,
  op          TEXT NOT NULL CHECK (op IN ('insert','update','delete')),
  rev         INTEGER NOT NULL,
  device_id   TEXT NOT NULL,                       -- the row's writer (local device, or a remote device when applied by sync)
  changed_at  TEXT NOT NULL,
  row_json    TEXT CHECK (row_json IS NULL OR json_valid(row_json)),  -- snapshot of syncable columns; NULL for delete
  pushed_at   TEXT                                 -- set by the future sync client
) STRICT;
CREATE INDEX change_log_pending_idx ON change_log(device_id, seq) WHERE pushed_at IS NULL;
CREATE INDEX change_log_row_idx     ON change_log(table_name, row_id);

-- ---------------------------------------------------------------------------
-- 5.5 settings (S for scope = 'user', L for scope = 'device'; rev/device only)
-- ---------------------------------------------------------------------------

CREATE TABLE settings (
  key         TEXT PRIMARY KEY,                    -- dotted, lowercase
  value_json  TEXT NOT NULL CHECK (json_valid(value_json)),
  scope       TEXT NOT NULL CHECK (scope IN ('device','user')),   -- device: never leaves this machine
  updated_at  TEXT NOT NULL,
  rev         INTEGER NOT NULL DEFAULT 1,
  device_id   TEXT NOT NULL
) STRICT;

-- ---------------------------------------------------------------------------
-- 5.1 Notes domain
-- ---------------------------------------------------------------------------

CREATE TABLE folders (
  id          TEXT PRIMARY KEY,
  parent_id   TEXT REFERENCES folders(id) ON DELETE CASCADE,
  path        TEXT NOT NULL,                       -- vault-relative, '/'-separated, no leading or trailing '/'
  name        TEXT NOT NULL,                       -- last path segment
  kind        TEXT NOT NULL DEFAULT 'normal' CHECK (kind IN ('normal','capture','templates')),
  position    INTEGER NOT NULL DEFAULT 0,          -- manual sort order among siblings
  icon        TEXT,
  color       TEXT,                                -- theme token name or #rrggbb
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  rev         INTEGER NOT NULL DEFAULT 1,
  device_id   TEXT NOT NULL,
  deleted_at  TEXT
) STRICT;
CREATE UNIQUE INDEX folders_path_uq ON folders(path) WHERE deleted_at IS NULL;
CREATE INDEX folders_parent_idx ON folders(parent_id);

CREATE TABLE notes (
  seq               INTEGER PRIMARY KEY,           -- rowid alias; FTS5 content_rowid
  id                TEXT NOT NULL UNIQUE,          -- UUID v7, identical to frontmatter `id`
  folder_id         TEXT REFERENCES folders(id) ON DELETE SET NULL,
  path              TEXT NOT NULL,                 -- vault-relative file path including '.md'
  title             TEXT NOT NULL,                 -- first H1, else filename without extension
  status            TEXT NOT NULL DEFAULT 'todo',  -- 'todo' | 'doing' | 'done' | custom slug (settings 'notes.statuses')
  urgent            INTEGER NOT NULL DEFAULT 0 CHECK (urgent IN (0,1)),
  when_raw          TEXT,                          -- frontmatter `when` as authored: 'today' | 'tomorrow' | 'YYYY-MM-DD' | NULL
  when_date         TEXT,                          -- resolved local date 'YYYY-MM-DD'; NULL = none
  body              TEXT NOT NULL DEFAULT '',      -- cache of the markdown body (frontmatter stripped)
  tags_text         TEXT NOT NULL DEFAULT '',      -- cache: space-joined tag names, for FTS
  excerpt           TEXT NOT NULL DEFAULT '',      -- first 200 chars of body, markdown stripped
  word_count        INTEGER NOT NULL DEFAULT 0,
  task_total        INTEGER NOT NULL DEFAULT 0,    -- task lines in body
  task_done         INTEGER NOT NULL DEFAULT 0,    -- task lines with the ✓ glyph
  pinned            INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0,1)),
  archived_at       TEXT,                          -- archive is a flag, not a folder move; NULL = active
  source_kind       TEXT NOT NULL DEFAULT 'manual'
                    CHECK (source_kind IN ('manual','capture','message','session','calendar','import','chat')),
  file_mtime        TEXT,                          -- mtime at last reconcile, ISO-8601 ms
  file_size         INTEGER,
  content_hash      TEXT,                          -- sha256 of the whole file at last reconcile
  frontmatter_dirty INTEGER NOT NULL DEFAULT 0 CHECK (frontmatter_dirty IN (0,1)),
  indexed_at        TEXT,
  created_at        TEXT NOT NULL,                 -- frontmatter `created`
  updated_at        TEXT NOT NULL,                 -- frontmatter `updated`
  rev               INTEGER NOT NULL DEFAULT 1,    -- frontmatter `rev`
  device_id         TEXT NOT NULL,
  deleted_at        TEXT
) STRICT;
CREATE UNIQUE INDEX notes_path_uq     ON notes(path) WHERE deleted_at IS NULL;
CREATE INDEX notes_folder_idx         ON notes(folder_id) WHERE deleted_at IS NULL;
CREATE INDEX notes_status_idx         ON notes(status, urgent) WHERE deleted_at IS NULL AND archived_at IS NULL;
CREATE INDEX notes_when_idx           ON notes(when_date) WHERE deleted_at IS NULL AND archived_at IS NULL AND when_date IS NOT NULL;
CREATE INDEX notes_updated_idx        ON notes(updated_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX notes_dirty_idx          ON notes(frontmatter_dirty) WHERE frontmatter_dirty = 1;

CREATE TABLE note_revisions (
  id            TEXT PRIMARY KEY,
  note_id       TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  rev           INTEGER NOT NULL,                  -- the note's rev after this change
  origin        TEXT NOT NULL CHECK (origin IN ('mode','external','restore','import')),
  content       TEXT NOT NULL,                     -- full file text (frontmatter + body) at this rev
  content_hash  TEXT NOT NULL,
  device_id     TEXT NOT NULL,
  created_at    TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX note_revisions_uq ON note_revisions(note_id, rev);

CREATE TABLE tags (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,                       -- display form, e.g. 'Groceries'
  name_norm   TEXT NOT NULL,                       -- lowercase, trimmed, internal spaces collapsed; identity
  color       TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  rev         INTEGER NOT NULL DEFAULT 1,
  device_id   TEXT NOT NULL,
  deleted_at  TEXT
) STRICT;
CREATE UNIQUE INDEX tags_name_uq ON tags(name_norm) WHERE deleted_at IS NULL;

CREATE TABLE note_tags (
  note_id   TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  tag_id    TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  position  INTEGER NOT NULL DEFAULT 0,            -- order as written in frontmatter
  PRIMARY KEY (note_id, tag_id)
) WITHOUT ROWID, STRICT;
CREATE INDEX note_tags_tag_idx ON note_tags(tag_id);

CREATE TABLE groupings (
  id               TEXT PRIMARY KEY,
  name             TEXT NOT NULL,
  slug             TEXT NOT NULL,                  -- 'todo','doing','urgent','done','today','tomorrow' or custom
  kind             TEXT NOT NULL CHECK (kind IN ('premade','custom')),
  filter_json      TEXT NOT NULL CHECK (json_valid(filter_json)),
  sort_json        TEXT NOT NULL DEFAULT '[{"by":"updated_at","dir":"desc"}]' CHECK (json_valid(sort_json)),
  glyph            TEXT,                           -- '○' '◑' '✱' '✓' or any single grapheme
  color            TEXT,
  position         INTEGER NOT NULL DEFAULT 0,
  show_in_overlay  INTEGER NOT NULL DEFAULT 0 CHECK (show_in_overlay IN (0,1)),
  show_in_sidebar  INTEGER NOT NULL DEFAULT 1 CHECK (show_in_sidebar IN (0,1)),
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  rev              INTEGER NOT NULL DEFAULT 1,
  device_id        TEXT NOT NULL,
  deleted_at       TEXT
) STRICT;
CREATE UNIQUE INDEX groupings_slug_uq ON groupings(slug) WHERE deleted_at IS NULL;

-- ---------------------------------------------------------------------------
-- 5.2 Sessions domain
-- ---------------------------------------------------------------------------

CREATE TABLE sessions (
  id                     TEXT PRIMARY KEY,         -- Mode id (UUID v7)
  claude_session_id      TEXT NOT NULL,            -- Claude Code `session_id`
  parent_session_id      TEXT REFERENCES sessions(id) ON DELETE CASCADE,   -- set for subagents
  agent_id               TEXT,                     -- SubagentStart `agent_id`
  agent_type             TEXT,                     -- SubagentStart `agent_type`
  tracking               TEXT NOT NULL CHECK (tracking IN ('hooked','launched','discovered')),
  launch_mode            TEXT NOT NULL DEFAULT 'external' CHECK (launch_mode IN ('external','embedded','headless')),  -- embedded = Mode's PTY
  state                  TEXT NOT NULL CHECK (state IN ('working','waiting','failed','unread','idle','done','untracked')),
  state_since            TEXT NOT NULL,
  state_reason           TEXT,                     -- last event that set the state, e.g. 'Notification:permission_prompt'
  waiting_kind           TEXT CHECK (waiting_kind IN ('permission','question','idle_prompt','elicitation','agent_needs_input')),
  pending_json           TEXT CHECK (pending_json IS NULL OR json_valid(pending_json)),  -- display copy of the open PermissionRequest / AskUserQuestion
  display_name           TEXT NOT NULL,            -- roster name: session_title, else repo folder name
  title                  TEXT,                     -- Claude Code `session_title` as last seen
  avatar_color           TEXT NOT NULL,            -- theme token, derived from hash(repo_root or cwd)
  cwd                    TEXT NOT NULL,
  repo_root              TEXT,                     -- git toplevel of cwd, NULL outside a repo
  git_branch             TEXT,
  model                  TEXT,
  permission_mode        TEXT,                     -- default|plan|acceptEdits|auto|dontAsk|bypassPermissions
  current_tool           TEXT,                     -- tool running right now (NULL when none)
  current_tool_use_id    TEXT,
  current_mcp_server     TEXT,
  last_prompt_preview    TEXT,                     -- first 500 chars of the last UserPromptSubmit (optional, see §11)
  last_assistant_preview TEXT,                     -- first 2 000 chars of Stop.last_assistant_message
  transcript_path        TEXT,
  terminal_app           TEXT,                     -- terminal.app|iterm2|warp|ghostty|windows_terminal|powershell|wsl
  terminal_ref           TEXT,                     -- opaque window/tab handle for jump-to-terminal
  lines_added            INTEGER NOT NULL DEFAULT 0,
  lines_removed          INTEGER NOT NULL DEFAULT 0,
  files_changed          INTEGER NOT NULL DEFAULT 0,
  num_turns              INTEGER NOT NULL DEFAULT 0,
  input_tokens           INTEGER NOT NULL DEFAULT 0,
  output_tokens          INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens      INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens     INTEGER NOT NULL DEFAULT 0,
  context_tokens         INTEGER,                  -- last known context size (SessionStart resume / transcript)
  cost_usd               REAL,                     -- stored from the headless `result.total_cost_usd` event only; not displayed in v1 (ADR-0032)
  started_at             TEXT NOT NULL,
  last_event_at          TEXT NOT NULL,
  ended_at               TEXT,
  end_reason             TEXT,                     -- SessionEnd.reason | 'exit' | 'signal' | 'inferred' | 'subagent_stop'
  acknowledged_at        TEXT,                     -- unread → done
  dismissed_at           TEXT,                     -- removed from roster by the user
  created_at             TEXT NOT NULL,
  updated_at             TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX sessions_claude_uq ON sessions(claude_session_id, COALESCE(agent_id, ''));
CREATE INDEX sessions_state_idx       ON sessions(state, last_event_at DESC) WHERE parent_session_id IS NULL;
CREATE INDEX sessions_ended_idx       ON sessions(ended_at) WHERE ended_at IS NOT NULL;
CREATE INDEX sessions_parent_idx      ON sessions(parent_session_id);
CREATE INDEX sessions_repo_idx        ON sessions(repo_root);

CREATE TABLE session_events (
  id            TEXT PRIMARY KEY,
  session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  seq           INTEGER NOT NULL,                  -- per-session, monotonic, assigned by the core
  source        TEXT NOT NULL CHECK (source IN ('hook','transcript','pty','headless','mode')),
  event         TEXT NOT NULL,                     -- hook_event_name, or 'transcript.assistant', 'mode.approve', ...
  tool_name     TEXT,
  tool_use_id   TEXT,
  agent_id      TEXT,
  state_before  TEXT,
  state_after   TEXT,                              -- NULL when the event did not change state
  summary       TEXT,                              -- one line for the Overview thread, e.g. 'Edit src/main.rs (+12 −3)'
  payload_json  TEXT CHECK (payload_json IS NULL OR json_valid(payload_json)),   -- trimmed, see §10 and §11
  payload_bytes INTEGER NOT NULL DEFAULT 0,        -- size before trimming, for diagnostics
  compacted     INTEGER NOT NULL DEFAULT 0 CHECK (compacted IN (0,1)),
  occurred_at   TEXT NOT NULL,                     -- from the producer when it carries a time, else = received_at
  received_at   TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX session_events_seq_uq ON session_events(session_id, seq);
CREATE INDEX session_events_time_idx     ON session_events(received_at);
CREATE INDEX session_events_tool_idx     ON session_events(tool_use_id) WHERE tool_use_id IS NOT NULL;

CREATE TABLE file_changes (
  id                TEXT PRIMARY KEY,
  session_id        TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  path              TEXT NOT NULL,                 -- absolute, normalized
  rel_path          TEXT,                          -- relative to sessions.repo_root when inside it
  change_kind       TEXT NOT NULL CHECK (change_kind IN ('added','modified','deleted','renamed','untracked')),
  old_path          TEXT,                          -- for 'renamed'
  lines_added       INTEGER NOT NULL DEFAULT 0,
  lines_removed     INTEGER NOT NULL DEFAULT 0,
  edit_count        INTEGER NOT NULL DEFAULT 0,    -- Edit/Write/MultiEdit tool calls that touched it
  source            TEXT NOT NULL CHECK (source IN ('hook','git','both')),
  last_tool_use_id  TEXT,
  first_seen_at     TEXT NOT NULL,
  last_seen_at      TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX file_changes_path_uq ON file_changes(session_id, path);

CREATE TABLE session_connectors (
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL CHECK (kind IN ('mcp_server','tool_rule','plugin','relay')),
  name         TEXT NOT NULL,                       -- server name, rule key ('Bash:npm'), plugin name, or 'mode-hook'
  status       TEXT NOT NULL CHECK (status IN ('connected','pending','failed','allowed','denied')),
  detail_json  TEXT CHECK (detail_json IS NULL OR json_valid(detail_json)),   -- {"tools": 12} | {"origin": "always rule"} | {"version": "…"} | {"last_event_at": "…", "relay_version": "…"}
  seen_at      TEXT NOT NULL,                       -- last time the source confirmed the row
  PRIMARY KEY (session_id, kind, name)
) WITHOUT ROWID, STRICT;

-- ---------------------------------------------------------------------------
-- 5.3 Connectors and inbox domain (contacts before messages: messages references them)
-- ---------------------------------------------------------------------------

CREATE TABLE accounts (
  id             TEXT PRIMARY KEY,
  provider       TEXT NOT NULL CHECK (provider IN ('google','slack')),
  external_id    TEXT NOT NULL,                    -- google: OIDC `sub`; slack: '<team_id>:<user_id>'
  display_name   TEXT NOT NULL,
  email          TEXT,
  handle         TEXT,                             -- slack user id (U…) / display name
  team_id        TEXT,                             -- slack workspace id
  team_name      TEXT,
  avatar_url     TEXT,
  services_json  TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(services_json)),   -- ["gmail","calendar"] | ["slack"]
  scopes_json    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(scopes_json)),     -- granted scopes, verbatim
  status         TEXT NOT NULL CHECK (status IN ('active','paused','needs_reauth','revoked','error')),
  status_detail  TEXT,                             -- last error text, user-visible
  keyring_ref    TEXT NOT NULL,                    -- keyring entry name, e.g. 'account:<id>'; never the token
  cursor_json    TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(cursor_json)),     -- {"gmail_history_id":"…","calendar":{"<calendarId>":"<syncToken>"}}
  color          TEXT,                             -- account chip color
  position       INTEGER NOT NULL DEFAULT 0,
  connected_at   TEXT NOT NULL,
  last_sync_at   TEXT,
  last_error_at  TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX accounts_ext_uq ON accounts(provider, external_id);

CREATE TABLE account_sources (
  id              TEXT PRIMARY KEY,
  account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  kind            TEXT NOT NULL CHECK (kind IN ('gmail_inbox','gmail_label','slack_dm','slack_mpim','slack_channel','slack_mentions','calendar')),
  external_id     TEXT NOT NULL,                   -- label id, conversation id, calendar id ('INBOX', 'primary', ...)
  name            TEXT NOT NULL,
  enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  vip             INTEGER NOT NULL DEFAULT 0 CHECK (vip IN (0,1)),   -- VIP conversation: every message here gets tier 'vip' (07 §5)
  poll_seconds    INTEGER,                         -- NULL = provider default (gmail 60, slack 30)
  cursor          TEXT,                            -- slack: latest message ts seen; gmail/calendar: unused (account-level cursor)
  last_polled_at  TEXT,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX account_sources_uq ON account_sources(account_id, kind, external_id);

CREATE TABLE contacts (
  id             TEXT PRIMARY KEY,
  display_name   TEXT NOT NULL,
  primary_email  TEXT,
  avatar_url     TEXT,
  company        TEXT,
  vip            INTEGER NOT NULL DEFAULT 0 CHECK (vip IN (0,1)),
  vip_note       TEXT,                             -- becomes the `why` line: 'VIP: your manager'
  notes          TEXT,
  message_count  INTEGER NOT NULL DEFAULT 0,
  last_seen_at   TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL,
  rev            INTEGER NOT NULL DEFAULT 1,
  device_id      TEXT NOT NULL,
  deleted_at     TEXT
) STRICT;
CREATE INDEX contacts_vip_idx ON contacts(vip) WHERE vip = 1 AND deleted_at IS NULL;

CREATE TABLE contact_identities (
  id          TEXT PRIMARY KEY,
  contact_id  TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('email','slack_user')),
  value       TEXT NOT NULL,                       -- lowercase email, or slack user id
  scope       TEXT NOT NULL DEFAULT '',            -- '' for email; slack team_id for slack_user
  is_primary  INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0,1)),
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  rev         INTEGER NOT NULL DEFAULT 1,
  device_id   TEXT NOT NULL,
  deleted_at  TEXT
) STRICT;
CREATE UNIQUE INDEX contact_identities_uq ON contact_identities(kind, scope, value) WHERE deleted_at IS NULL;

CREATE TABLE threads (
  id                 TEXT PRIMARY KEY,
  account_id         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  provider           TEXT NOT NULL CHECK (provider IN ('google','slack')),
  external_id        TEXT NOT NULL,                -- gmail threadId; slack '<channel_id>:<thread_ts>' or '<channel_id>' for top-level
  kind               TEXT NOT NULL CHECK (kind IN ('email','dm','group_dm','channel','channel_thread')),
  title              TEXT NOT NULL,                -- email subject; slack channel/DM name
  channel_id         TEXT,
  channel_name       TEXT,
  participants_json  TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(participants_json)),  -- [{"name":..,"address":..,"contact_id":..}]
  message_count      INTEGER NOT NULL DEFAULT 0,
  unread_count       INTEGER NOT NULL DEFAULT 0,
  last_message_at    TEXT,
  last_snippet       TEXT,
  urgency_tier       TEXT CHECK (urgency_tier IN ('vip','direct','ai-urgent','ai-normal','low','unscored')),  -- highest tier among unread messages
  is_archived        INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0,1)),
  source_url         TEXT,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX threads_ext_uq   ON threads(account_id, external_id);
CREATE INDEX threads_recent_idx      ON threads(last_message_at DESC) WHERE is_archived = 0;

CREATE TABLE messages (
  seq                 INTEGER PRIMARY KEY,         -- rowid alias; FTS5 content_rowid
  id                  TEXT NOT NULL UNIQUE,
  account_id          TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  thread_id           TEXT REFERENCES threads(id) ON DELETE SET NULL,
  source_id           TEXT REFERENCES account_sources(id) ON DELETE SET NULL,
  provider            TEXT NOT NULL CHECK (provider IN ('google','slack')),
  external_id         TEXT NOT NULL,               -- gmail message id; slack '<channel_id>:<ts>'
  rfc_message_id      TEXT,                        -- Message-ID header, lowercase; NULL for Slack
  dedup_key           TEXT NOT NULL,               -- 07 §7.1 key: the Message-ID for email, 'team_id:channel_id:ts' for Slack
  kind                TEXT NOT NULL CHECK (kind IN ('email','dm','group_dm','channel','channel_thread')),
  sender_contact_id   TEXT REFERENCES contacts(id) ON DELETE SET NULL,
  sender_name         TEXT NOT NULL,
  sender_address      TEXT NOT NULL,               -- email address (lowercase) or slack user id
  sender_avatar_url   TEXT,
  channel_id          TEXT,
  channel_name        TEXT,
  subject             TEXT,                        -- email only
  snippet             TEXT NOT NULL,               -- first 500 chars of plain text, whitespace collapsed; the scoring input
  body_text           TEXT,                        -- plain text, ≤ 32 KB; NULL once outside the retention window
  body_truncated      INTEGER NOT NULL DEFAULT 0 CHECK (body_truncated IN (0,1)),
  body_retained_until TEXT,                        -- NULL when body_text is NULL
  has_attachments     INTEGER NOT NULL DEFAULT 0 CHECK (has_attachments IN (0,1)),
  labels_json         TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(labels_json)),   -- gmail label ids / slack reactions
  is_direct           INTEGER NOT NULL DEFAULT 0 CHECK (is_direct IN (0,1)),        -- DM, @mention, or sole To: recipient
  is_read             INTEGER NOT NULL DEFAULT 0 CHECK (is_read IN (0,1)),
  read_at             TEXT,
  read_origin         TEXT CHECK (read_origin IN ('mode','provider')),
  is_archived         INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0,1)),      -- archive; written back to the provider when scopes allow
  archived_at         TEXT,
  urgency_tier        TEXT NOT NULL DEFAULT 'unscored'
                      CHECK (urgency_tier IN ('vip','direct','ai-urgent','ai-normal','low','unscored')),
  urgency_why         TEXT,                        -- one line shown next to the badge
  urgency_source      TEXT CHECK (urgency_source IN ('rule','ai','user')),
  urgency_model       TEXT,                        -- model id plus prompt version, e.g. 'claude-haiku-5-5+p1', when urgency_source = 'ai'
  scored_at           TEXT,
  source_url          TEXT NOT NULL,               -- deep link for "open in source"
  received_at         TEXT NOT NULL,
  fetched_at          TEXT NOT NULL,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX messages_ext_uq      ON messages(account_id, external_id);
CREATE INDEX messages_thread_idx         ON messages(thread_id, received_at);
CREATE INDEX messages_recent_idx         ON messages(received_at DESC);
CREATE INDEX messages_inbox_idx          ON messages(is_read, is_archived, urgency_tier, received_at DESC);
CREATE INDEX messages_sender_idx         ON messages(sender_contact_id);
CREATE INDEX messages_unscored_idx       ON messages(received_at) WHERE urgency_tier = 'unscored';
CREATE INDEX messages_body_ret_idx       ON messages(body_retained_until) WHERE body_text IS NOT NULL;
CREATE INDEX messages_dedup_idx          ON messages(dedup_key);

CREATE TABLE connector_writebacks (
  id               TEXT PRIMARY KEY,
  account_id       TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  target_kind      TEXT NOT NULL CHECK (target_kind IN ('message','thread')),
  target_id        TEXT NOT NULL,
  action           TEXT NOT NULL CHECK (action IN ('mark_read','mark_unread','archive','unarchive')),
  external_id      TEXT NOT NULL,                  -- provider id of the target
  attempts         INTEGER NOT NULL DEFAULT 0,
  next_attempt_at  TEXT,
  last_error       TEXT,
  created_at       TEXT NOT NULL
) STRICT;
CREATE INDEX connector_writebacks_due_idx ON connector_writebacks(next_attempt_at);

-- ---------------------------------------------------------------------------
-- 5.4 Calendar domain
-- ---------------------------------------------------------------------------

CREATE TABLE calendar_events (
  id                   TEXT PRIMARY KEY,
  account_id           TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  calendar_id          TEXT NOT NULL,
  calendar_name        TEXT,
  external_id          TEXT NOT NULL,              -- Google event id (instance id for recurring instances)
  recurring_event_id   TEXT,
  ical_uid             TEXT,                       -- Google iCalUID, for cross-account de-duplication (07 §7.1)
  title                TEXT NOT NULL,
  description          TEXT,                       -- plain text, ≤ 2 KB
  location             TEXT,
  starts_at            TEXT NOT NULL,              -- UTC instant (all-day: local midnight converted at sync)
  ends_at              TEXT NOT NULL,
  all_day              INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0,1)),
  start_date           TEXT,                       -- 'YYYY-MM-DD' for all-day events
  end_date             TEXT,                       -- exclusive, Google semantics
  timezone             TEXT,                       -- IANA zone of the event
  status               TEXT NOT NULL CHECK (status IN ('confirmed','tentative','cancelled')),
  my_response          TEXT CHECK (my_response IN ('accepted','declined','tentative','needs_action')),
  organizer            TEXT,
  attendees_json       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(attendees_json)),
  join_url             TEXT,
  conference           TEXT CHECK (conference IN ('meet','zoom','teams','other')),
  html_link            TEXT,
  etag                 TEXT,
  synced_at            TEXT NOT NULL,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX calendar_events_ext_uq ON calendar_events(account_id, calendar_id, external_id);
CREATE INDEX calendar_events_time_idx      ON calendar_events(starts_at, ends_at) WHERE status <> 'cancelled';

-- ---------------------------------------------------------------------------
-- 5.4A Automations domain
-- ---------------------------------------------------------------------------

CREATE TABLE automations (
  id                 TEXT PRIMARY KEY,
  name               TEXT NOT NULL,
  enabled            INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  trigger_kind       TEXT NOT NULL DEFAULT 'schedule' CHECK (trigger_kind IN ('schedule')),   -- v1: schedule only (ADR-0033)
  schedule_json      TEXT NOT NULL CHECK (json_valid(schedule_json)),   -- {"kind":"daily"|"weekdays"|"weekly"|"cron","time":"08:00","tz":"Europe/Madrid","weekday":1,"cron":"0 8 * * 1-5"}
  action_kind        TEXT NOT NULL CHECK (action_kind IN ('start_session','create_note','ask_chat')),
  action_json        TEXT NOT NULL CHECK (json_valid(action_json)),
  notify_on_failure  INTEGER NOT NULL DEFAULT 1 CHECK (notify_on_failure IN (0,1)),
  keep_runs          INTEGER NOT NULL DEFAULT 20,
  last_run_at        TEXT,
  next_run_at        TEXT,                          -- UTC instant computed in schedule_json.tz; NULL while disabled
  position           INTEGER NOT NULL DEFAULT 0,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,
  rev                INTEGER NOT NULL DEFAULT 1,
  device_id          TEXT NOT NULL,
  deleted_at         TEXT
) STRICT;
CREATE INDEX automations_due_idx ON automations(next_run_at) WHERE enabled = 1 AND deleted_at IS NULL;

CREATE TABLE automation_runs (
  id             TEXT PRIMARY KEY,
  automation_id  TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
  trigger_kind   TEXT NOT NULL DEFAULT 'schedule' CHECK (trigger_kind IN ('schedule','manual')),   -- manual = "Run now"
  started_at     TEXT NOT NULL,
  finished_at    TEXT,
  state          TEXT NOT NULL CHECK (state IN ('running','finished','failed','skipped')),
  result_kind    TEXT CHECK (result_kind IN ('session','note')),
  result_id      TEXT,                              -- sessions.id or notes.id (polymorphic, no FK; "unavailable" once pruned)
  error          TEXT,                              -- failure text, or the skip reason for 'skipped'
  created_at     TEXT NOT NULL
) STRICT;
CREATE INDEX automation_runs_auto_idx ON automation_runs(automation_id, started_at DESC);
CREATE INDEX automation_runs_running_idx ON automation_runs(automation_id) WHERE state = 'running';

-- ---------------------------------------------------------------------------
-- 5.5 Cross-cutting tables
-- ---------------------------------------------------------------------------

CREATE TABLE links (
  id          TEXT PRIMARY KEY,
  from_kind   TEXT NOT NULL CHECK (from_kind IN ('note','message','thread','session','calendar_event','contact')),
  from_id     TEXT NOT NULL,
  to_kind     TEXT NOT NULL CHECK (to_kind IN ('note','message','thread','session','calendar_event','contact','url')),
  to_id       TEXT NOT NULL,                       -- row id, or the URL itself when to_kind = 'url'
  link_type   TEXT NOT NULL CHECK (link_type IN ('created_from','references','prepare_for','related','attachment')),
  label       TEXT,                                -- display text for url links
  created_by  TEXT NOT NULL DEFAULT 'user' CHECK (created_by IN ('user','ai','system')),
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  rev         INTEGER NOT NULL DEFAULT 1,
  device_id   TEXT NOT NULL,
  deleted_at  TEXT
) STRICT;
CREATE UNIQUE INDEX links_uq     ON links(from_kind, from_id, to_kind, to_id, link_type) WHERE deleted_at IS NULL;
CREATE INDEX links_from_idx      ON links(from_kind, from_id);
CREATE INDEX links_to_idx        ON links(to_kind, to_id);

CREATE TABLE snoozes (
  id            TEXT PRIMARY KEY,
  target_kind   TEXT NOT NULL CHECK (target_kind IN ('message','thread','note','session','calendar_event')),
  target_id     TEXT NOT NULL,
  until         TEXT NOT NULL,                     -- UTC
  preset        TEXT CHECK (preset IN ('later_today','tomorrow','next_week','pick_date','until_reply')),
  wake_on_reply INTEGER NOT NULL DEFAULT 1 CHECK (wake_on_reply IN (0,1)),   -- a reply in the thread ends the snooze early (07 §6)
  fired_reason  TEXT CHECK (fired_reason IN ('time','reply')),
  fired_at      TEXT,                              -- when the item resurfaced
  cancelled_at  TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  rev           INTEGER NOT NULL DEFAULT 1,
  device_id     TEXT NOT NULL,
  deleted_at    TEXT
) STRICT;
CREATE INDEX snoozes_active_idx ON snoozes(until) WHERE fired_at IS NULL AND cancelled_at IS NULL AND deleted_at IS NULL;
CREATE INDEX snoozes_target_idx ON snoozes(target_kind, target_id);

CREATE TABLE alert_rules (
  id                TEXT PRIMARY KEY,
  name              TEXT NOT NULL,
  kind              TEXT NOT NULL CHECK (kind IN ('premade','custom')),
  enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  trigger_json      TEXT NOT NULL CHECK (json_valid(trigger_json)),
  channels_json     TEXT NOT NULL DEFAULT '["overlay"]' CHECK (json_valid(channels_json)),  -- subset of ["overlay","os_notification","sound"]
  sound             TEXT,                           -- cue id, 'pack:<slug>:<cue id>' or 'user:<sha256>' (10 §10.2)
  respect_dnd       INTEGER NOT NULL DEFAULT 1 CHECK (respect_dnd IN (0,1)),
  quiet_hours       INTEGER NOT NULL DEFAULT 1 CHECK (quiet_hours IN (0,1)),   -- obey settings 'notifications.quiet_hours'
  cooldown_seconds  INTEGER NOT NULL DEFAULT 0,
  position          INTEGER NOT NULL DEFAULT 0,
  last_fired_at     TEXT,
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL,
  rev               INTEGER NOT NULL DEFAULT 1,
  device_id         TEXT NOT NULL,
  deleted_at        TEXT
) STRICT;

CREATE TABLE alert_log (
  id                TEXT PRIMARY KEY,
  rule_id           TEXT NOT NULL,                  -- alert_rules.id (no FK: the log outlives rule edits)
  event             TEXT NOT NULL,                  -- trigger event name
  item_key          TEXT NOT NULL,                  -- 07 §7.1 item key
  cooldown_key      TEXT,
  channels_json     TEXT NOT NULL CHECK (json_valid(channels_json)),
  suppressed_reason TEXT,                           -- NULL when delivered; else the 07 §8.4 row
  fired_at          TEXT,
  created_at        TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX alert_log_uq ON alert_log(rule_id, event, item_key);
CREATE INDEX alert_log_time_idx ON alert_log(created_at);

CREATE TABLE themes (
  id           TEXT PRIMARY KEY,
  slug         TEXT NOT NULL,                      -- 'system','paper','gruvbox-dark','gruvbox-light','cursor-dark','nord','catppuccin-mocha', user slugs
  name         TEXT NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('builtin','imported','custom')),
  appearance   TEXT NOT NULL CHECK (appearance IN ('light','dark','auto')),
  version      TEXT NOT NULL DEFAULT '1',
  author       TEXT,
  tokens_json  TEXT NOT NULL CHECK (json_valid(tokens_json)),   -- the theme pack document from 10-ui-ux.md
  source_path  TEXT,                               -- import origin for 'imported'
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  rev          INTEGER NOT NULL DEFAULT 1,
  device_id    TEXT NOT NULL,
  deleted_at   TEXT
) STRICT;
CREATE UNIQUE INDEX themes_slug_uq ON themes(slug) WHERE deleted_at IS NULL;

CREATE TABLE hook_installs (
  id             TEXT PRIMARY KEY,
  scope          TEXT NOT NULL CHECK (scope IN ('user','project','local')),   -- ~/.claude/settings.json, .claude/settings.json, .claude/settings.local.json
  settings_path  TEXT NOT NULL,                    -- absolute path of the settings file Mode edited
  relay_path     TEXT NOT NULL,                    -- absolute path of mode-hook(.exe)
  relay_version  TEXT NOT NULL,
  events_json    TEXT NOT NULL CHECK (json_valid(events_json)),   -- ["SessionStart","PermissionRequest",...]
  block_hash     TEXT NOT NULL,                    -- sha256 of the hook entries Mode wrote, to detect user edits
  backup_path    TEXT,                             -- copy of settings file taken before the first edit
  status         TEXT NOT NULL CHECK (status IN ('installed','stale','missing','conflict','removed')),
  status_detail  TEXT,
  installed_at   TEXT NOT NULL,
  verified_at    TEXT,                             -- last time the file was re-read and matched
  removed_at     TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX hook_installs_path_uq ON hook_installs(settings_path);

CREATE TABLE usage_samples (
  id                  TEXT PRIMARY KEY,
  sampled_at          TEXT NOT NULL,
  source              TEXT NOT NULL CHECK (source IN ('claude_code','claude_api','ollama','lm_studio')),
  purpose             TEXT NOT NULL CHECK (purpose IN ('session','scoring','summary','chat','title')),
  session_id          TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  model               TEXT NOT NULL,
  input_tokens        INTEGER NOT NULL DEFAULT 0,
  output_tokens       INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens   INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens  INTEGER NOT NULL DEFAULT 0,
  cost_usd            REAL,                        -- NULL for local backends and for claude_code without a headless result event; stored, not displayed in v1 (ADR-0032)
  turn_ref            TEXT,                        -- transcript message uuid or API request id, for de-duplication
  created_at          TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX usage_samples_turn_uq ON usage_samples(source, turn_ref) WHERE turn_ref IS NOT NULL;
CREATE INDEX usage_samples_time_idx       ON usage_samples(sampled_at);
CREATE INDEX usage_samples_session_idx    ON usage_samples(session_id);

CREATE TABLE chat_turns (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,                  -- UUID v7 of the first turn's conversation
  role             TEXT NOT NULL CHECK (role IN ('user','assistant','tool')),
  content          TEXT NOT NULL,
  citations_json   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(citations_json)),  -- [{"kind":"note","id":"…"}, …]
  model            TEXT,
  created_at       TEXT NOT NULL
) STRICT;
CREATE INDEX chat_turns_conv_idx ON chat_turns(conversation_id, created_at);

CREATE TABLE activity_days (
  day               TEXT PRIMARY KEY,               -- local date 'YYYY-MM-DD'
  sessions_started  INTEGER NOT NULL DEFAULT 0,     -- top-level sessions whose started_at falls on the day
  files_changed     INTEGER NOT NULL DEFAULT 0,     -- file_changes rows whose first_seen_at falls on the day
  tokens            INTEGER NOT NULL DEFAULT 0,     -- claude_code tokens of the day (tooltip only, never the value)
  rolled_at         TEXT NOT NULL
) WITHOUT ROWID, STRICT;

-- ---------------------------------------------------------------------------
-- 6. Full-text search (external-content FTS5 + sync triggers)
-- ---------------------------------------------------------------------------

CREATE VIRTUAL TABLE notes_fts USING fts5(
  title, body, tags_text, path UNINDEXED,
  content = 'notes', content_rowid = 'seq',
  tokenize = 'unicode61 remove_diacritics 2',
  prefix = '2 3'
);

CREATE TRIGGER notes_fts_ai AFTER INSERT ON notes BEGIN
  INSERT INTO notes_fts(rowid, title, body, tags_text, path)
  VALUES (new.seq, new.title, new.body, new.tags_text, new.path);
END;

CREATE TRIGGER notes_fts_ad AFTER DELETE ON notes BEGIN
  INSERT INTO notes_fts(notes_fts, rowid, title, body, tags_text, path)
  VALUES ('delete', old.seq, old.title, old.body, old.tags_text, old.path);
END;

CREATE TRIGGER notes_fts_au AFTER UPDATE OF title, body, tags_text, path ON notes BEGIN
  INSERT INTO notes_fts(notes_fts, rowid, title, body, tags_text, path)
  VALUES ('delete', old.seq, old.title, old.body, old.tags_text, old.path);
  INSERT INTO notes_fts(rowid, title, body, tags_text, path)
  VALUES (new.seq, new.title, new.body, new.tags_text, new.path);
END;

CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject, snippet, body_text, sender_name,
  content = 'messages', content_rowid = 'seq',
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, subject, snippet, body_text, sender_name)
  VALUES (new.seq, new.subject, new.snippet, new.body_text, new.sender_name);
END;

CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, snippet, body_text, sender_name)
  VALUES ('delete', old.seq, old.subject, old.snippet, old.body_text, old.sender_name);
END;

CREATE TRIGGER messages_fts_au AFTER UPDATE OF subject, snippet, body_text, sender_name ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, snippet, body_text, sender_name)
  VALUES ('delete', old.seq, old.subject, old.snippet, old.body_text, old.sender_name);
  INSERT INTO messages_fts(rowid, subject, snippet, body_text, sender_name)
  VALUES (new.seq, new.subject, new.snippet, new.body_text, new.sender_name);
END;

-- ---------------------------------------------------------------------------
-- 9. change_log triggers for every sync-class S table
--    AFTER INSERT; AFTER UPDATE only when rev changed or deleted_at flipped;
--    AFTER DELETE only WHEN old.deleted_at IS NULL (purging a tombstone is not a change).
-- ---------------------------------------------------------------------------

-- folders
CREATE TRIGGER folders_log_ai AFTER INSERT ON folders BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('folders', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'parent_id', new.parent_id, 'path', new.path, 'name', new.name,
                      'kind', new.kind, 'position', new.position, 'icon', new.icon, 'color', new.color,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER folders_log_au AFTER UPDATE ON folders
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('folders', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'parent_id', new.parent_id, 'path', new.path, 'name', new.name,
                      'kind', new.kind, 'position', new.position, 'icon', new.icon, 'color', new.color,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER folders_log_ad AFTER DELETE ON folders WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('folders', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- notes: the sync unit is the markdown file, so row_json carries only the locator and the SQLite-only flags
CREATE TRIGGER notes_log_ai AFTER INSERT ON notes BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('notes', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'path', new.path, 'pinned', new.pinned, 'archived_at', new.archived_at,
                      'source_kind', new.source_kind, 'content_hash', new.content_hash,
                      'rev', new.rev, 'updated_at', new.updated_at, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER notes_log_au AFTER UPDATE ON notes
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('notes', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'path', new.path, 'pinned', new.pinned, 'archived_at', new.archived_at,
                      'source_kind', new.source_kind, 'content_hash', new.content_hash,
                      'rev', new.rev, 'updated_at', new.updated_at, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER notes_log_ad AFTER DELETE ON notes WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('notes', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- tags
CREATE TRIGGER tags_log_ai AFTER INSERT ON tags BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('tags', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'name', new.name, 'name_norm', new.name_norm, 'color', new.color,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER tags_log_au AFTER UPDATE ON tags
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('tags', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'name', new.name, 'name_norm', new.name_norm, 'color', new.color,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER tags_log_ad AFTER DELETE ON tags WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('tags', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- groupings
CREATE TRIGGER groupings_log_ai AFTER INSERT ON groupings BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('groupings', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'name', new.name, 'slug', new.slug, 'kind', new.kind,
                      'filter_json', json(new.filter_json), 'sort_json', json(new.sort_json),
                      'glyph', new.glyph, 'color', new.color, 'position', new.position,
                      'show_in_overlay', new.show_in_overlay, 'show_in_sidebar', new.show_in_sidebar,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER groupings_log_au AFTER UPDATE ON groupings
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('groupings', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'name', new.name, 'slug', new.slug, 'kind', new.kind,
                      'filter_json', json(new.filter_json), 'sort_json', json(new.sort_json),
                      'glyph', new.glyph, 'color', new.color, 'position', new.position,
                      'show_in_overlay', new.show_in_overlay, 'show_in_sidebar', new.show_in_sidebar,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER groupings_log_ad AFTER DELETE ON groupings WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('groupings', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- contacts
CREATE TRIGGER contacts_log_ai AFTER INSERT ON contacts BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('contacts', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'display_name', new.display_name, 'primary_email', new.primary_email,
                      'avatar_url', new.avatar_url, 'company', new.company, 'vip', new.vip, 'vip_note', new.vip_note,
                      'notes', new.notes, 'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER contacts_log_au AFTER UPDATE ON contacts
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('contacts', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'display_name', new.display_name, 'primary_email', new.primary_email,
                      'avatar_url', new.avatar_url, 'company', new.company, 'vip', new.vip, 'vip_note', new.vip_note,
                      'notes', new.notes, 'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER contacts_log_ad AFTER DELETE ON contacts WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('contacts', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- contact_identities
CREATE TRIGGER contact_identities_log_ai AFTER INSERT ON contact_identities BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('contact_identities', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'contact_id', new.contact_id, 'kind', new.kind, 'value', new.value,
                      'scope', new.scope, 'is_primary', new.is_primary,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER contact_identities_log_au AFTER UPDATE ON contact_identities
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('contact_identities', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'contact_id', new.contact_id, 'kind', new.kind, 'value', new.value,
                      'scope', new.scope, 'is_primary', new.is_primary,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER contact_identities_log_ad AFTER DELETE ON contact_identities WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('contact_identities', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- links
CREATE TRIGGER links_log_ai AFTER INSERT ON links BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('links', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'from_kind', new.from_kind, 'from_id', new.from_id,
                      'to_kind', new.to_kind, 'to_id', new.to_id, 'link_type', new.link_type,
                      'label', new.label, 'created_by', new.created_by,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER links_log_au AFTER UPDATE ON links
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('links', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'from_kind', new.from_kind, 'from_id', new.from_id,
                      'to_kind', new.to_kind, 'to_id', new.to_id, 'link_type', new.link_type,
                      'label', new.label, 'created_by', new.created_by,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER links_log_ad AFTER DELETE ON links WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('links', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- snoozes
CREATE TRIGGER snoozes_log_ai AFTER INSERT ON snoozes BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('snoozes', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'target_kind', new.target_kind, 'target_id', new.target_id,
                      'until', new.until, 'preset', new.preset, 'wake_on_reply', new.wake_on_reply,
                      'fired_reason', new.fired_reason, 'fired_at', new.fired_at, 'cancelled_at', new.cancelled_at,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER snoozes_log_au AFTER UPDATE ON snoozes
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('snoozes', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'target_kind', new.target_kind, 'target_id', new.target_id,
                      'until', new.until, 'preset', new.preset, 'wake_on_reply', new.wake_on_reply,
                      'fired_reason', new.fired_reason, 'fired_at', new.fired_at, 'cancelled_at', new.cancelled_at,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER snoozes_log_ad AFTER DELETE ON snoozes WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('snoozes', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- alert_rules
CREATE TRIGGER alert_rules_log_ai AFTER INSERT ON alert_rules BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('alert_rules', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'name', new.name, 'kind', new.kind, 'enabled', new.enabled,
                      'trigger_json', json(new.trigger_json), 'channels_json', json(new.channels_json),
                      'sound', new.sound, 'respect_dnd', new.respect_dnd, 'quiet_hours', new.quiet_hours,
                      'cooldown_seconds', new.cooldown_seconds, 'position', new.position,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER alert_rules_log_au AFTER UPDATE ON alert_rules
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('alert_rules', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'name', new.name, 'kind', new.kind, 'enabled', new.enabled,
                      'trigger_json', json(new.trigger_json), 'channels_json', json(new.channels_json),
                      'sound', new.sound, 'respect_dnd', new.respect_dnd, 'quiet_hours', new.quiet_hours,
                      'cooldown_seconds', new.cooldown_seconds, 'position', new.position,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER alert_rules_log_ad AFTER DELETE ON alert_rules WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('alert_rules', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- themes: only user-made packs (kind <> 'builtin') are user data
CREATE TRIGGER themes_log_ai AFTER INSERT ON themes WHEN new.kind <> 'builtin' BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('themes', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'slug', new.slug, 'name', new.name, 'kind', new.kind,
                      'appearance', new.appearance, 'version', new.version, 'author', new.author,
                      'tokens_json', json(new.tokens_json), 'source_path', new.source_path,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER themes_log_au AFTER UPDATE ON themes
WHEN new.kind <> 'builtin' AND (new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL)) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('themes', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'slug', new.slug, 'name', new.name, 'kind', new.kind,
                      'appearance', new.appearance, 'version', new.version, 'author', new.author,
                      'tokens_json', json(new.tokens_json), 'source_path', new.source_path,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER themes_log_ad AFTER DELETE ON themes WHEN old.kind <> 'builtin' AND old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('themes', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- automations
CREATE TRIGGER automations_log_ai AFTER INSERT ON automations BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('automations', new.id, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('id', new.id, 'name', new.name, 'enabled', new.enabled, 'trigger_kind', new.trigger_kind,
                      'schedule_json', json(new.schedule_json), 'action_kind', new.action_kind,
                      'action_json', json(new.action_json), 'notify_on_failure', new.notify_on_failure,
                      'keep_runs', new.keep_runs, 'position', new.position,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at));
END;
CREATE TRIGGER automations_log_au AFTER UPDATE ON automations
WHEN new.rev <> old.rev OR (new.deleted_at IS NULL) <> (old.deleted_at IS NULL) BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('automations', new.id, CASE WHEN new.deleted_at IS NOT NULL THEN 'delete' ELSE 'update' END,
          new.rev, new.device_id, new.updated_at,
          CASE WHEN new.deleted_at IS NOT NULL THEN NULL ELSE
          json_object('id', new.id, 'name', new.name, 'enabled', new.enabled, 'trigger_kind', new.trigger_kind,
                      'schedule_json', json(new.schedule_json), 'action_kind', new.action_kind,
                      'action_json', json(new.action_json), 'notify_on_failure', new.notify_on_failure,
                      'keep_runs', new.keep_runs, 'position', new.position,
                      'created_at', new.created_at, 'updated_at', new.updated_at,
                      'rev', new.rev, 'deleted_at', new.deleted_at) END);
END;
CREATE TRIGGER automations_log_ad AFTER DELETE ON automations WHEN old.deleted_at IS NULL BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('automations', old.id, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- settings: S for scope = 'user' only; a reset (DELETE) is the tombstone
CREATE TRIGGER settings_log_ai AFTER INSERT ON settings WHEN new.scope = 'user' BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('settings', new.key, 'insert', new.rev, new.device_id, new.updated_at,
          json_object('key', new.key, 'value_json', json(new.value_json), 'scope', new.scope,
                      'updated_at', new.updated_at, 'rev', new.rev));
END;
CREATE TRIGGER settings_log_au AFTER UPDATE ON settings WHEN new.scope = 'user' AND new.rev <> old.rev BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('settings', new.key, 'update', new.rev, new.device_id, new.updated_at,
          json_object('key', new.key, 'value_json', json(new.value_json), 'scope', new.scope,
                      'updated_at', new.updated_at, 'rev', new.rev));
END;
CREATE TRIGGER settings_log_ad AFTER DELETE ON settings WHEN old.scope = 'user' BEGIN
  INSERT INTO change_log(table_name, row_id, op, rev, device_id, changed_at, row_json)
  VALUES ('settings', old.key, 'delete', old.rev + 1, old.device_id, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
END;

-- ---------------------------------------------------------------------------
-- 12. Seeds. Premade rows use fixed ids so every device agrees on them
--     (groupings 00000000-0000-7000-8000-0000000001NN, alert rules …0002NN, themes …0003NN).
--     The Rust side (`db::seeds`) holds the same definitions and re-applies them on each start.
-- ---------------------------------------------------------------------------

-- 5.1 premade groupings; Today and Urgent show in the overlay (02 §7.3)
INSERT INTO groupings (id, name, slug, kind, filter_json, sort_json, glyph, color, position, show_in_overlay, show_in_sidebar,
                       created_at, updated_at, rev, device_id, deleted_at) VALUES
  ('00000000-0000-7000-8000-000000000101', 'Todo',     'todo',     'premade', '{"status":{"in":["todo"]}}',
   '[{"by":"updated_at","dir":"desc"}]', '○', NULL, 0, 0, 1,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000102', 'Doing',    'doing',    'premade', '{"status":{"in":["doing"]}}',
   '[{"by":"updated_at","dir":"desc"}]', '◑', NULL, 1, 0, 1,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000103', 'Urgent',   'urgent',   'premade', '{"urgent":true,"status":{"not_in":["closed"]}}',
   '[{"by":"when_date","dir":"asc"},{"by":"updated_at","dir":"desc"}]', '✱', NULL, 2, 1, 1,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000104', 'Done',     'done',     'premade', '{"status":{"in":["done"]}}',
   '[{"by":"updated_at","dir":"desc"}]', '✓', NULL, 3, 0, 1,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000105', 'Today',    'today',    'premade', '{"when":{"in":["overdue","today"]},"status":{"not_in":["closed"]}}',
   '[{"by":"urgent","dir":"desc"},{"by":"when_date","dir":"asc"},{"by":"updated_at","dir":"desc"}]', '▦', NULL, 4, 1, 1,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000106', 'Tomorrow', 'tomorrow', 'premade', '{"when":{"in":["tomorrow"]},"status":{"not_in":["closed"]}}',
   '[{"by":"urgent","dir":"desc"},{"by":"when_date","dir":"asc"},{"by":"updated_at","dir":"desc"}]', '▦', NULL, 5, 0, 1,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL);

-- 5.5 premade alert rules (07 §8.2 plus the ADR-0033 "Automation failed" row of 09 §5.5); overlay only
INSERT INTO alert_rules (id, name, kind, enabled, trigger_json, channels_json, sound, respect_dnd, quiet_hours, cooldown_seconds, position,
                         last_fired_at, created_at, updated_at, rev, device_id, deleted_at) VALUES
  ('00000000-0000-7000-8000-000000000201', 'Session needs you', 'premade', 1,
   '{"event":"session.waiting","filters":{"waiting_kind":["permission","question","elicitation","agent_needs_input"]}}',
   '["overlay"]', 'waiting', 1, 1, 0, 0, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000202', 'Session failed', 'premade', 1,
   '{"event":"session.failed","filters":{}}',
   '["overlay"]', 'failed', 1, 1, 0, 1, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000203', 'Session finished', 'premade', 1,
   '{"event":"session.finished","filters":{"min_duration_seconds":0}}',
   '["overlay"]', 'finished', 1, 1, 0, 2, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000204', 'Urgent message', 'premade', 1,
   '{"event":"message.received","filters":{"tier":["vip","direct","ai-urgent"]}}',
   '["overlay"]', 'urgent', 1, 1, 120, 3, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000205', 'Meeting starting', 'premade', 1,
   '{"event":"calendar.starting","filters":{"minutes_before":5,"requires_join_url":false}}',
   '["overlay"]', 'starting', 1, 1, 0, 4, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000206', 'Due today', 'premade', 1,
   '{"event":"note.due","filters":{"at":"start_of_day","urgent_only":false}}',
   '["overlay"]', NULL, 1, 1, 0, 5, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000207', 'AI budget at 80 %', 'premade', 0,
   '{"event":"usage.threshold","filters":{"percent":80}}',
   '["overlay"]', NULL, 1, 1, 0, 6, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000208', 'Automation failed', 'premade', 1,
   '{"event":"automation.failed","filters":{}}',
   '["overlay"]', NULL, 1, 1, 0, 7, NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL);

-- 5.5 built-in theme packs (10 §5): header rows; the token documents are filled by the theme
-- loader through the re-seed path (builtin rows are not user data and never reach change_log)
INSERT INTO themes (id, slug, name, kind, appearance, version, author, tokens_json, source_path,
                    created_at, updated_at, rev, device_id, deleted_at) VALUES
  ('00000000-0000-7000-8000-000000000301', 'system',          'System',          'builtin', 'auto',  '1', 'Mode',
   '{"format":1,"slug":"system","name":"System","author":"Mode","version":"1","appearance":"auto"}', NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000302', 'paper',           'Paper',           'builtin', 'auto',  '1', 'Mode',
   '{"format":1,"slug":"paper","name":"Paper","author":"Mode","version":"1","appearance":"auto"}', NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000303', 'gruvbox-dark',    'Gruvbox Dark',    'builtin', 'dark',  '1', 'Mode',
   '{"format":1,"slug":"gruvbox-dark","name":"Gruvbox Dark","author":"Mode","version":"1","appearance":"dark"}', NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000304', 'gruvbox-light',   'Gruvbox Light',   'builtin', 'light', '1', 'Mode',
   '{"format":1,"slug":"gruvbox-light","name":"Gruvbox Light","author":"Mode","version":"1","appearance":"light"}', NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000305', 'cursor-dark',     'Cursor Dark',     'builtin', 'dark',  '1', 'Mode',
   '{"format":1,"slug":"cursor-dark","name":"Cursor Dark","author":"Mode","version":"1","appearance":"dark"}', NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000306', 'nord',            'Nord',            'builtin', 'dark',  '1', 'Mode',
   '{"format":1,"slug":"nord","name":"Nord","author":"Mode","version":"1","appearance":"dark"}', NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL),
  ('00000000-0000-7000-8000-000000000307', 'catppuccin-mocha', 'Catppuccin Mocha', 'builtin', 'dark', '1', 'Mode',
   '{"format":1,"slug":"catppuccin-mocha","name":"Catppuccin Mocha","author":"Mode","version":"1","appearance":"dark"}', NULL,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__', NULL);

-- 5.5 settings: every key of the inventory whose default is a value. Keys whose default is
-- "none", "first detected", "first listed model" or "chosen in onboarding" are not seeded
-- (a missing row is the default). Environment-derived values arrive through the tokens.
INSERT INTO settings (key, value_json, scope, updated_at, rev, device_id) VALUES
  ('device.id',                                json_quote('__DEVICE_ID__'),      'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('device.name',                              json_quote('__DEVICE_NAME__'),    'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.vault_path',                         '"~/Mode/Notes"',                 'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.statuses',                           '[{"slug":"todo","label":"Todo","glyph":"○","color":null,"closed":false},{"slug":"doing","label":"Doing","glyph":"◑","color":null,"closed":false},{"slug":"done","label":"Done","glyph":"✓","color":null,"closed":true}]', 'user', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.touchup_delay_seconds',              '30',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.pinned',                           'false',                          'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.peek_delay_ms',                    '150',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('capture.hotkey',                           json_quote('__CAPTURE_HOTKEY__'), 'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('sessions.fold_after_minutes',              '10',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('sessions.store_prompt_previews',           'false',                          'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('polling.gmail_seconds',                    '60',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('polling.slack_seconds',                    '30',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('polling.slack_budget_per_minute',          '40',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.backend',                               '"claude_api"',                   'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.local_only',                            'false',                          'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.model_chat',                            '"claude-opus-5-5"',              'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.model_scoring',                         '"claude-haiku-5-5"',             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.scoring_chars',                         '500',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.budget_tokens_month',                   '2000000',                        'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('urgency.keyword_rules',                    '[]',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notifications.quiet_hours',                '{"enabled":false,"from":"22:00","to":"08:00"}', 'user', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('retention.session_events_days',            '7',                              'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('retention.sessions_days',                  '90',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('retention.messages_days',                  '90',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('retention.message_body_days',              '7',                              'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('retention.usage_days',                     '180',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('usage.budget_tokens_week',                 'null',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('usage.reset_weekday',                      'null',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('theme.active',                             '"system"',                       'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('theme.appearance',                         '"auto"',                         'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('theme.typography_style',                   '"pack"',                         'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('theme.font_scale',                         '1.0',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('theme.terminal_pack',                      '"follow"',                       'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('sounds.enabled',                           'false',                          'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('sounds.volume',                            '60',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.expand_on_hover',                  'true',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.expand_hover_ms',                  '600',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.collapse_delay_ms',                '2500',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.auto_collapse_ms',                 '8000',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.pill_min_ms',                      '2000',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.max_rows',                         '6',                              'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.default_tab',                      '"auto"',                         'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.display',                          '"main"',                         'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.fullscreen',                       '"presentation-hides"',           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.right_edge_dock',                  '"float"',                        'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.motion',                           '"system"',                       'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.show_mascot',                      'true',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.scale',                            '100',                            'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.toggle_hotkey',                    json_quote('__TOGGLE_HOTKEY__'),  'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.pin_hotkey',                       json_quote('__PIN_HOTKEY__'),     'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('overlay.capture_sound',                    'null',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('onboarding.steps',                         '{}',                             'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('onboarding.completed_at',                  'null',                           'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('window.sidebar_collapsed',                 'false',                          'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('window.detail_open',                       'true',                           'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('window.last_route',                        '"sessions"',                     'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('general.launch_at_login',                  'false',                          'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('general.status_item',                      '{"badge":true,"pause_entry":true}', 'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('general.compact_rows',                     'false',                          'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('general.auto_update',                      'true',                           'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('general.language',                         '"en"',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('sessions.filter',                          '{}',                             'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('sessions.group_by_repo',                   'false',                          'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('sessions.claude_path',                     'null',                           'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('hooks.decision_timeout_seconds',           '120',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('hooks.async_timeout_seconds',              '5',                              'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('inbox.saved_filters',                      '[]',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.sort',                               '{}',                             'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.board_groupings',                    '["todo","doing","urgent","done"]', 'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.rename_with_title',                  'false',                          'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.touchup_enabled',                    'true',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.week_start',                         '"monday"',                       'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.autosave_delay_ms',                  '500',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notes.show_archived_in_search',            'false',                          'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('cards.open',                               '[]',                             'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('polling.calendar_seconds',                 '300',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('gmail.initial_sync_days',                  '14',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('gmail.initial_sync_max',                   '500',                            'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('gmail.body_fetch',                         '"auto"',                         'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('slack.initial_sync_days',                  '7',                              'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('slack.search_mentions_seconds',            '60',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('connectors.oauth_ports',                   '[48311,48312,48313,48314,48315]', 'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.local_url',                             '"http://127.0.0.1:11434"',       'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.model_summary',                         '"claude-haiku-5-5"',             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('ai.timeout_seconds',                       '{"scoring":30,"summary":120,"chat":300}', 'user', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('urgency.direct_email_requires_known_contact', 'true',                        'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('urgency.pill_message_max_age_hours',       '24',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notifications.max_alert_age_minutes',      '30',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notifications.snooze_wake_on_reply',       'true',                           'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notifications.pause_until',                'null',                           'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notifications.os_preview',                 '"full"',                         'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notifications.os_sound',                   'false',                          'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('notifications.focus_detection',            '{"macos_assertions":true,"windows_wnf":true}', 'user', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('profile.name',                             json_quote('__PROFILE_NAME__'),   'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('profile.picture_path',                     'null',                           'device', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('shortcuts.bindings',                       '{}',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__'),
  ('automations.catch_up_minutes',             '30',                             'user',   strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, '__DEVICE_ID__');
