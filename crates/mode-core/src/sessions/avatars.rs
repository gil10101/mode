//! Session avatars (05 §11.2, ADR-0050): the bot mark wears one accessory per
//! session so sessions running side by side tell apart at a glance. The core
//! picks the variant when a row is created and never changes it.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::sessions::naming::least_used_variant;

/// The variant for a new top-level session: the one the fewest running
/// top-level sessions wear, the earlier one in list order on a tie.
/// Subagents copy their parent's instead.
pub fn next_variant(conn: &Connection) -> rusqlite::Result<String> {
    let mut stmt = conn.prepare(
        "SELECT avatar_variant, COUNT(*) FROM sessions
          WHERE parent_session_id IS NULL AND ended_at IS NULL AND dismissed_at IS NULL
          GROUP BY avatar_variant",
    )?;
    let counts: HashMap<String, i64> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(least_used_variant(|v| counts.get(v).copied().unwrap_or(0)).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::naming::AVATAR_VARIANTS;

    fn table() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, parent_session_id TEXT, ended_at TEXT,
                                    dismissed_at TEXT, avatar_variant TEXT NOT NULL DEFAULT '')",
        )
        .unwrap();
        c
    }

    fn add(c: &Connection, id: &str, variant: &str, parent: Option<&str>, ended: bool) {
        c.execute(
            "INSERT INTO sessions (id, parent_session_id, ended_at, avatar_variant) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, parent, ended.then_some("2026-10-29T00:00:00.000Z"), variant],
        )
        .unwrap();
    }

    #[test]
    fn running_sessions_get_distinct_variants_in_list_order() {
        let c = table();
        for (i, expected) in AVATAR_VARIANTS.iter().enumerate() {
            let v = next_variant(&c).unwrap();
            assert_eq!(&v, expected, "session {i}");
            add(&c, &format!("s{i}"), &v, None, false);
        }
        // All eight worn once: the ninth starts over at the first.
        assert_eq!(next_variant(&c).unwrap(), "beret");
    }

    #[test]
    fn ended_dismissed_and_subagent_rows_free_their_variant() {
        let c = table();
        add(&c, "a", "beret", None, false);
        add(&c, "b", "specs", None, true); // ended: specs is free again
        add(&c, "c", "tophat", None, false);
        c.execute("UPDATE sessions SET dismissed_at = 'x' WHERE id = 'c'", [])
            .unwrap();
        add(&c, "d", "beret", Some("a"), false); // a subagent wearing the parent's beret
        assert_eq!(next_variant(&c).unwrap(), "specs");
    }
}
