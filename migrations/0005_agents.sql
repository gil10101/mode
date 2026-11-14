-- 0005 — Agent profiles (03 §7E, ADR-0058): a reusable configuration a
-- session is launched with (the brain: model, effort, backup models; what it
-- knows: instructions, memories, skills, tables; its rules: permissions,
-- notifications; the advanced settings), and the profile a session ran as.

CREATE TABLE agent_profiles (
  id                   TEXT PRIMARY KEY,
  name                 TEXT NOT NULL,
  title                TEXT,                                 -- the one-line role under the name
  instructions         TEXT,                                 -- appended to the system prompt
  avatar_variant       TEXT NOT NULL DEFAULT '',             -- the bot mark's accessory (05 §11.2)
  model                TEXT,                                 -- --model; NULL = Claude Code's default
  effort               TEXT CHECK (effort IS NULL OR effort IN ('low','medium','high','xhigh','max')),   -- --effort
  fallback_models_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(fallback_models_json)),            -- --fallback-model, in order, at most 3
  ask_before_switching INTEGER NOT NULL DEFAULT 1,           -- 1: Mode offers the switch; 0: the CLI switches with a notice
  skills_json          TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(skills_json)),    -- the skills left on ([] = all)
  tables_json          TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tables_json)),    -- folders and files the agent may read (--add-dir)
  permission_mode      TEXT,                                 -- --permission-mode; NULL = the launch's choice
  allowed_tools_json   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_tools_json)),   -- --allowedTools rules
  denied_tools_json    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(denied_tools_json)),    -- --disallowedTools rules
  notify               INTEGER NOT NULL DEFAULT 1,           -- 0 mutes the alerts of this agent's sessions
  workspace            TEXT,                                 -- the default folder a launch starts in
  claude_path          TEXT,                                 -- a claude binary of its own
  extra_args_json      TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(extra_args_json)),   -- appended as given
  isolate_config       INTEGER NOT NULL DEFAULT 0,           -- CLAUDE_CONFIG_DIR of its own (memory, transcripts, settings)
  position             INTEGER NOT NULL DEFAULT 0,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL
) STRICT;

CREATE TABLE agent_memories (
  id          TEXT PRIMARY KEY,
  profile_id  TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
  title       TEXT NOT NULL,
  body        TEXT NOT NULL,
  position    INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
) STRICT;
CREATE INDEX agent_memories_profile_idx ON agent_memories(profile_id, position);

ALTER TABLE sessions ADD COLUMN profile_id TEXT REFERENCES agent_profiles(id) ON DELETE SET NULL;   -- the agent the session was launched as
CREATE INDEX sessions_profile_idx ON sessions(profile_id) WHERE profile_id IS NOT NULL;
