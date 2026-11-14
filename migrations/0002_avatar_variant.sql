-- 0002: session avatar variants (05 §11.2, ADR-0050).
-- The bot mark wears one accessory per session. The core picks the variant the
-- fewest running top-level sessions wear when a row is created, a subagent
-- copies its parent's, and the value never changes afterwards; '' draws the
-- plain mark.
ALTER TABLE sessions ADD COLUMN avatar_variant TEXT NOT NULL DEFAULT '';

-- Rows from before this migration: spread over the eight variants by the id's
-- last characters (stable, unlike rowid after a VACUUM), in assignment order,
-- so an existing roster is varied; subagents follow their parent.
UPDATE sessions
   SET avatar_variant = CASE (unicode(substr(id, -1, 1)) + unicode(substr(id, -2, 1)) * 7 + unicode(substr(id, -3, 1)) * 13) % 8
         WHEN 0 THEN 'beret' WHEN 1 THEN 'specs' WHEN 2 THEN 'tophat' WHEN 3 THEN 'mustache'
         WHEN 4 THEN 'beanie' WHEN 5 THEN 'cap' WHEN 6 THEN 'squares' ELSE 'monocle' END
 WHERE parent_session_id IS NULL;
UPDATE sessions
   SET avatar_variant = COALESCE((SELECT p.avatar_variant FROM sessions p WHERE p.id = sessions.parent_session_id), '')
 WHERE parent_session_id IS NOT NULL;
