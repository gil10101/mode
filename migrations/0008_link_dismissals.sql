-- 0008 — Suggested links the user removed (03 §4.1, §6.6; 09 §5.5): the
-- thread and event panels suggest sessions, notes and messages by mention
-- and by attendee; removing a suggestion records it here so it is not
-- suggested again. Adding the same link by hand deletes the row. Local to
-- the device (class L): a dismissal is a view preference, not shared data.

CREATE TABLE link_dismissals (
  anchor_kind  TEXT NOT NULL CHECK (anchor_kind IN ('thread','calendar_event')),
  anchor_id    TEXT NOT NULL,
  target_kind  TEXT NOT NULL CHECK (target_kind IN ('session','note','message')),
  target_id    TEXT NOT NULL,
  dismissed_at TEXT NOT NULL,
  PRIMARY KEY (anchor_kind, anchor_id, target_kind, target_id)
) STRICT;
