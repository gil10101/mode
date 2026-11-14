-- 0004 — Startup reconciliation (05 §4.2 "Mode starts", ADR-0054): the
-- `claude` process a session's relay (or Mode's PTY host) last reported and
-- the machine boot that pid belongs to, so a restarted Mode can probe what it
-- knew and a rebooted machine can close what cannot have survived.

ALTER TABLE sessions ADD COLUMN pid     INTEGER;   -- the relay's ppid, or the PTY host's child; NULL until a frame carries one
ALTER TABLE sessions ADD COLUMN boot_id TEXT;      -- the boot the pid belongs to (sessions::liveness::boot_id)
