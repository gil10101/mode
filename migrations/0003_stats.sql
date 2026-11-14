-- 0003 — Stats (03 §7A, ADR-0051, ADR-0052): the share of a message's cache
-- writes that went to the 1-hour cache (priced differently), year-long
-- roll-ups per model and per hour next to `activity_days`, richer day
-- roll-ups, and an `origin` so history imported from Claude Code's own stats
-- cache never overwrites what Mode counted itself.

ALTER TABLE usage_samples ADD COLUMN cache_write_1h_tokens INTEGER NOT NULL DEFAULT 0;   -- part of cache_write_tokens (usage.cache_creation.ephemeral_1h_input_tokens)

ALTER TABLE activity_days ADD COLUMN messages          INTEGER NOT NULL DEFAULT 0;   -- claude_code assistant messages of the day
ALTER TABLE activity_days ADD COLUMN agents_started    INTEGER NOT NULL DEFAULT 0;   -- subagent sessions whose started_at falls on the day
ALTER TABLE activity_days ADD COLUMN lines_added       INTEGER NOT NULL DEFAULT 0;   -- file_changes first seen on the day
ALTER TABLE activity_days ADD COLUMN lines_removed     INTEGER NOT NULL DEFAULT 0;
ALTER TABLE activity_days ADD COLUMN longest_session_s INTEGER NOT NULL DEFAULT 0;   -- longest top-level session started on the day, seconds
ALTER TABLE activity_days ADD COLUMN origin            TEXT    NOT NULL DEFAULT 'mode';   -- 'mode' (rolled from the live tables) | 'claude_stats' (imported, 09 §5.5)

CREATE TABLE activity_models (
  day                 TEXT    NOT NULL,                -- local date 'YYYY-MM-DD'
  source              TEXT    NOT NULL,                -- usage_samples.source
  model               TEXT    NOT NULL,
  messages            INTEGER NOT NULL DEFAULT 0,      -- usage_samples rows
  input_tokens        INTEGER NOT NULL DEFAULT 0,
  output_tokens       INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens   INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens  INTEGER NOT NULL DEFAULT 0,
  spend_usd           REAL,                            -- at list price when rolled (ADR-0051); NULL when the model has no price
  origin              TEXT    NOT NULL DEFAULT 'mode',
  rolled_at           TEXT    NOT NULL,
  PRIMARY KEY (day, source, model)
) WITHOUT ROWID, STRICT;

CREATE TABLE activity_hours (
  day       TEXT    NOT NULL,                          -- local date 'YYYY-MM-DD'
  hour      INTEGER NOT NULL CHECK (hour BETWEEN 0 AND 23),   -- local hour
  messages  INTEGER NOT NULL DEFAULT 0,                -- claude_code assistant messages
  tokens    INTEGER NOT NULL DEFAULT 0,
  origin    TEXT    NOT NULL DEFAULT 'mode',
  rolled_at TEXT    NOT NULL,
  PRIMARY KEY (day, hour)
) WITHOUT ROWID, STRICT;
