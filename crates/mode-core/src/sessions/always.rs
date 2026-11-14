//! `always` rules (05 §7.2): per session, in memory only, until the session
//! ends or Mode restarts. A rule is derived from an approved call and answers
//! later `PermissionRequest` frames that match before any card is drawn.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

/// The rule key for a tool call (05 §7.2 table).
pub fn rule_key(tool_name: &str, tool_input: Option<&Value>, base: Option<&str>) -> String {
    let get = |k: &str| tool_input.and_then(|i| i.get(k)).and_then(Value::as_str);
    match tool_name {
        "Bash" => match get("command").and_then(program_name) {
            Some(prog) => format!("Bash:{prog}"),
            None => "Bash:*".into(),
        },
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = get("file_path")
                .or_else(|| get("notebook_path"))
                .unwrap_or("");
            let rel = crate::paths::display(path, base);
            let dir = match rel.rfind('/') {
                Some(i) => format!("{}/", &rel[..i]),
                None => "./".into(),
            };
            format!("Edit:{dir}")
        }
        "Read" | "Glob" | "Grep" | "WebFetch" | "WebSearch" => format!("{tool_name}:*"),
        other => other.to_string(),
    }
}

/// The program a shell command runs: environment assignments and leading
/// `sudo` / `env` stripped, the first word of the first pipeline segment.
pub fn program_name(command: &str) -> Option<String> {
    let first_segment = command
        .lines()
        .next()?
        .split(['|', ';', '&'])
        .next()?
        .trim();
    let mut words = first_segment.split_whitespace().peekable();
    while let Some(w) = words.peek() {
        let is_assignment = w.split_once('=').is_some_and(|(k, _)| {
            !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_')
        });
        let base = w.rsplit('/').next().unwrap_or(w);
        if is_assignment || base == "sudo" || base == "env" || w.starts_with('-') && *w != "-" {
            words.next();
        } else {
            break;
        }
    }
    let prog = words.next()?;
    let base = prog.rsplit('/').next().unwrap_or(prog);
    (!base.is_empty()).then(|| base.to_string())
}

/// The rules of every session, keyed by the Mode session id.
#[derive(Debug, Default)]
pub struct Rules {
    by_session: HashMap<String, BTreeSet<String>>,
}

impl Rules {
    /// Remember a rule; returns the key.
    pub fn add(&mut self, session_id: &str, key: String) -> String {
        self.by_session
            .entry(session_id.to_string())
            .or_default()
            .insert(key.clone());
        key
    }

    /// The key that allows this call, when one does.
    pub fn matching(
        &self,
        session_id: &str,
        tool_name: &str,
        tool_input: Option<&Value>,
        base: Option<&str>,
    ) -> Option<String> {
        let rules = self.by_session.get(session_id)?;
        let key = rule_key(tool_name, tool_input, base);
        if rules.contains(&key) {
            return Some(key);
        }
        // `Edit:src/` also covers `Edit:src/deeper/`.
        if let Some(dir) = key.strip_prefix("Edit:") {
            return rules
                .iter()
                .filter_map(|r| r.strip_prefix("Edit:").map(|d| (r, d)))
                .find(|(_, d)| dir.starts_with(*d))
                .map(|(r, _)| r.clone());
        }
        None
    }

    pub fn list(&self, session_id: &str) -> Vec<String> {
        self.by_session
            .get(session_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn remove(&mut self, session_id: &str, key: &str) -> bool {
        self.by_session
            .get_mut(session_id)
            .is_some_and(|s| s.remove(key))
    }

    /// Forget every rule of a session (it ended).
    pub fn clear(&mut self, session_id: &str) {
        self.by_session.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_table() {
        let bash = serde_json::json!({"command": "FOO=1 sudo npm test"});
        assert_eq!(rule_key("Bash", Some(&bash), None), "Bash:npm");
        let bash2 = serde_json::json!({"command": "/usr/bin/env node a.js | grep x"});
        assert_eq!(rule_key("Bash", Some(&bash2), None), "Bash:node");
        let edit = serde_json::json!({"file_path": "/u/mode/src/main.rs"});
        assert_eq!(rule_key("Edit", Some(&edit), Some("/u/mode")), "Edit:src/");
        let write = serde_json::json!({"file_path": "/u/mode/README.md"});
        assert_eq!(rule_key("Write", Some(&write), Some("/u/mode")), "Edit:./");
        assert_eq!(rule_key("WebFetch", None, None), "WebFetch:*");
        assert_eq!(
            rule_key("mcp__github__create_issue", None, None),
            "mcp__github__create_issue"
        );
        assert_eq!(rule_key("Other", None, None), "Other");
        assert_eq!(program_name("ls -la"), Some("ls".into()));
        assert_eq!(program_name(""), None);
    }

    #[test]
    fn rules_match_per_session_and_cover_subdirectories() {
        let mut rules = Rules::default();
        rules.add("s1", "Bash:npm".into());
        rules.add("s1", "Edit:src/".into());
        let run = serde_json::json!({"command": "npm run build"});
        assert_eq!(
            rules.matching("s1", "Bash", Some(&run), None).as_deref(),
            Some("Bash:npm")
        );
        assert_eq!(rules.matching("s2", "Bash", Some(&run), None), None);
        let deep = serde_json::json!({"file_path": "/u/mode/src/a/b.rs"});
        assert_eq!(
            rules
                .matching("s1", "Edit", Some(&deep), Some("/u/mode"))
                .as_deref(),
            Some("Edit:src/")
        );
        let other = serde_json::json!({"file_path": "/u/mode/docs/a.md"});
        assert_eq!(
            rules.matching("s1", "Edit", Some(&other), Some("/u/mode")),
            None
        );
        assert_eq!(
            rules.list("s1"),
            vec!["Bash:npm".to_string(), "Edit:src/".into()]
        );
        assert!(rules.remove("s1", "Bash:npm"));
        assert!(!rules.remove("s1", "Bash:npm"));
        rules.clear("s1");
        assert!(rules.list("s1").is_empty());
    }
}
