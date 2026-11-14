-- 0006 — AI providers and routes (06 §10.1A, ADR-0059): the endpoints Mode's
-- own features and an agent's sessions may call, one route per feature with
-- backups, and the provider an agent profile runs its sessions on.

CREATE TABLE ai_providers (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('claude_api','openai_compat','ollama','lm_studio','bedrock','vertex','foundry')),
  name        TEXT NOT NULL,                              -- the label ("Claude API", "OpenRouter")
  base_url    TEXT,                                       -- NULL: the kind's default (the built-ins read ai.local_url)
  auth        TEXT NOT NULL DEFAULT 'api_key' CHECK (auth IN ('none','api_key','bearer')),   -- how the keyring secret is sent
  models_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(models_json)),   -- the models the user named, first = the default
  extra_json  TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(extra_json)),    -- region, project_id, resource, custom_headers
  enabled     INTEGER NOT NULL DEFAULT 1,
  builtin     INTEGER NOT NULL DEFAULT 0,                 -- the three built-ins cannot be deleted
  position    INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
) STRICT;

INSERT INTO ai_providers (id, kind, name, base_url, auth, models_json, extra_json, enabled, builtin, position, created_at, updated_at) VALUES
  ('claude_api', 'claude_api', 'Claude API', NULL, 'api_key', '[]', '{}', 1, 1, 0, strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  ('ollama',     'ollama',     'Ollama',     NULL, 'bearer',  '[]', '{}', 1, 1, 1, strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  ('lm_studio',  'lm_studio',  'LM Studio',  NULL, 'bearer',  '[]', '{}', 1, 1, 2, strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'));

CREATE TABLE ai_routes (
  purpose        TEXT PRIMARY KEY CHECK (purpose IN ('chat','scoring','summary')),
  provider_id    TEXT REFERENCES ai_providers(id) ON DELETE SET NULL,     -- NULL: the selected backend (ai.backend)
  model          TEXT,                                                     -- NULL: the provider's default model
  fallbacks_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(fallbacks_json)),   -- [{"provider_id": …, "model": …}], in order, at most 3
  updated_at     TEXT NOT NULL
) STRICT;

ALTER TABLE agent_profiles ADD COLUMN provider_id TEXT REFERENCES ai_providers(id) ON DELETE SET NULL;   -- the sessions' endpoint; NULL = Claude Code's own sign-in

-- usage_samples.source gains 'openai_compat' (a CHECK cannot be altered in place: the table is rebuilt, columns in their order).
CREATE TABLE usage_samples_new (
  id                  TEXT PRIMARY KEY,
  sampled_at          TEXT NOT NULL,
  source              TEXT NOT NULL CHECK (source IN ('claude_code','claude_api','openai_compat','ollama','lm_studio')),
  purpose             TEXT NOT NULL CHECK (purpose IN ('session','scoring','summary','chat','title')),
  session_id          TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  model               TEXT NOT NULL,
  input_tokens        INTEGER NOT NULL DEFAULT 0,
  output_tokens       INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens   INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens  INTEGER NOT NULL DEFAULT 0,
  cost_usd            REAL,
  turn_ref            TEXT,
  created_at          TEXT NOT NULL,
  cache_write_1h_tokens INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT INTO usage_samples_new SELECT id, sampled_at, source, purpose, session_id, model, input_tokens, output_tokens,
  cache_read_tokens, cache_write_tokens, cost_usd, turn_ref, created_at, cache_write_1h_tokens FROM usage_samples;
DROP TABLE usage_samples;
ALTER TABLE usage_samples_new RENAME TO usage_samples;
CREATE UNIQUE INDEX usage_samples_turn_uq ON usage_samples(source, turn_ref) WHERE turn_ref IS NOT NULL;
CREATE INDEX usage_samples_time_idx       ON usage_samples(sampled_at);
CREATE INDEX usage_samples_session_idx    ON usage_samples(session_id);
