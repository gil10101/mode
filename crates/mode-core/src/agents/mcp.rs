//! The MCP servers Claude Code already has configured (03 §7E.2 Does,
//! "Connectors"): an agent names some of them, and its launch gets a config
//! file holding only those (`--mcp-config <file> --strict-mcp-config`).
//!
//! Claude Code keeps `mcpServers` in three places, the first found wins for
//! a name: the local scope (`projects["<folder>"].mcpServers` of
//! `~/.claude.json`), the project scope (`<folder>/.mcp.json`) and the user
//! scope (the top-level `mcpServers` of `~/.claude.json`). With
//! `CLAUDE_CONFIG_DIR` set the file is `$CLAUDE_CONFIG_DIR/.claude.json`.
//! A missing or unreadable file is no servers.
//!
//! A project's `.mcp.json` arrives with the repository, so Claude Code asks
//! before it runs any of its servers and records the answer in the folder's
//! `projects` entry (`enabledMcpjsonServers`, `disabledMcpjsonServers`,
//! `enableAllProjectMcpServers`). `--mcp-config` would skip that question, so
//! a project-scope server is offered here only once Claude Code has approved
//! it for that folder; an unapproved or rejected one is not listed and never
//! reaches a launch's config file. Only names, scopes and
//! transports leave this module for the UI; definitions (which may carry an
//! environment or headers) go only into the launch's config file.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// One configured server, as the editor lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    /// `local` | `project` | `user`.
    pub scope: String,
    /// `stdio` | `http` | `sse` | the `type` given.
    pub transport: String,
}

/// Claude Code's config file: `$CLAUDE_CONFIG_DIR/.claude.json`, else
/// `~/.claude.json`.
pub fn claude_json() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => dirs::home_dir().map(|h| h.join(".claude.json")),
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The project-scope servers of `folder`'s `.mcp.json` that Claude Code has
/// approved for that folder: listed in `enabledMcpjsonServers`, or all of them
/// under `enableAllProjectMcpServers`, and never one in `disabledMcpjsonServers`.
fn approved_project_servers(folder: &Path, entry: Option<&Value>) -> Map<String, Value> {
    let names = |key: &str| -> Vec<String> {
        entry
            .and_then(|e| e.get(key))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let all = entry
        .and_then(|e| e.get("enableAllProjectMcpServers"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (enabled, disabled) = (
        names("enabledMcpjsonServers"),
        names("disabledMcpjsonServers"),
    );
    servers_of(read_json(&folder.join(".mcp.json")).as_ref())
        .into_iter()
        .filter(|(name, _)| !disabled.contains(name) && (all || enabled.contains(name)))
        .collect()
}

fn servers_of(v: Option<&Value>) -> Map<String, Value> {
    v.and_then(|v| v.get("mcpServers"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Every configured definition by name with its scope, local over project
/// over user; `folders` are the agent's repositories and workspace.
fn definitions(
    claude_json: Option<&Path>,
    folders: &[PathBuf],
) -> BTreeMap<String, (String, Value)> {
    let file = claude_json.and_then(read_json);
    let mut out: BTreeMap<String, (String, Value)> = BTreeMap::new();
    let mut add = |scope: &str, servers: Map<String, Value>| {
        for (name, def) in servers {
            out.entry(name).or_insert_with(|| (scope.to_string(), def));
        }
    };
    let projects = file
        .as_ref()
        .and_then(|f| f.get("projects"))
        .and_then(Value::as_object);
    let entry_of = |folder: &PathBuf| {
        let key = folder.to_string_lossy();
        let key = key.trim_end_matches(['/', '\\']).to_string();
        projects.and_then(|p| p.get(&key))
    };
    for folder in folders {
        add("local", servers_of(entry_of(folder)));
    }
    for folder in folders {
        add(
            "project",
            approved_project_servers(folder, entry_of(folder)),
        );
    }
    add("user", servers_of(file.as_ref()));
    out
}

fn transport(def: &Value) -> String {
    match def.get("type").and_then(Value::as_str) {
        Some(t) => t.to_string(),
        None if def.get("url").is_some() => "http".into(),
        None => "stdio".into(),
    }
}

/// The servers an agent can choose from, sorted by name.
pub fn configured(claude_json: Option<&Path>, folders: &[PathBuf]) -> Vec<McpServer> {
    definitions(claude_json, folders)
        .into_iter()
        .map(|(name, (scope, def))| McpServer {
            transport: transport(&def),
            name,
            scope,
        })
        .collect()
}

/// The `--mcp-config` document holding only the servers named in `names`
/// that are configured; a name no longer configured is left out (with
/// `--strict-mcp-config` the session then has no such server).
pub fn config_for(names: &[String], claude_json: Option<&Path>, folders: &[PathBuf]) -> String {
    let defs = definitions(claude_json, folders);
    let mut servers = Map::new();
    for n in names {
        let n = n.trim();
        if let Some((_, def)) = defs.get(n) {
            servers.insert(n.to_string(), def.clone());
        }
    }
    json!({ "mcpServers": servers }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mode-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_user_local_and_project_scopes_local_first() {
        let d = tmp("scopes");
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let cfg = d.join(".claude.json");
        std::fs::write(
            &cfg,
            json!({
                "mcpServers": { "github": { "command": "gh-mcp" }, "sentry": { "type": "http", "url": "https://u" } },
                "projects": { repo.to_string_lossy(): { "mcpServers": { "sentry": { "type": "sse", "url": "https://l" } }, "enabledMcpjsonServers": ["linear"] } }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(repo.join(".mcp.json"), json!({ "mcpServers": { "linear": { "url": "https://p" }, "github": { "command": "other" } } }).to_string()).unwrap();
        let list = configured(Some(&cfg), std::slice::from_ref(&repo));
        let row = |n: &str| list.iter().find(|s| s.name == n).cloned().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(
            (
                row("sentry").scope.as_str(),
                row("sentry").transport.as_str()
            ),
            ("local", "sse")
        );
        assert_eq!(
            (
                row("linear").scope.as_str(),
                row("linear").transport.as_str()
            ),
            ("project", "http")
        );
        assert_eq!(
            (
                row("github").scope.as_str(),
                row("github").transport.as_str()
            ),
            ("user", "stdio")
        );
        let doc: Value = serde_json::from_str(&config_for(
            &["sentry".into(), "gone".into()],
            Some(&cfg),
            &[repo],
        ))
        .unwrap();
        assert_eq!(
            doc,
            json!({ "mcpServers": { "sentry": { "type": "sse", "url": "https://l" } } })
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn project_servers_need_claude_codes_approval() {
        let d = tmp("approval");
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join(".mcp.json"),
            json!({ "mcpServers": { "evil": { "command": "sh", "args": ["-c", "id"] }, "ok": { "url": "https://p" } } }).to_string(),
        )
        .unwrap();
        let cfg = d.join(".claude.json");
        let names = |entry: Value| {
            std::fs::write(
                &cfg,
                json!({ "projects": { repo.to_string_lossy(): entry } }).to_string(),
            )
            .unwrap();
            let list = configured(Some(&cfg), std::slice::from_ref(&repo));
            list.into_iter().map(|s| s.name).collect::<Vec<_>>()
        };
        assert!(
            names(json!({})).is_empty(),
            "nothing approved: nothing offered"
        );
        assert_eq!(
            names(json!({ "enabledMcpjsonServers": ["ok"] })),
            vec!["ok"]
        );
        assert_eq!(
            names(
                json!({ "enableAllProjectMcpServers": true, "disabledMcpjsonServers": ["evil"] })
            ),
            vec!["ok"]
        );
        std::fs::write(&cfg, "{}").unwrap();
        assert_eq!(
            config_for(&["evil".into()], Some(&cfg), &[repo]),
            r#"{"mcpServers":{}}"#
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_or_broken_file_is_no_servers() {
        let d = tmp("missing");
        assert!(configured(Some(&d.join("nope.json")), &[]).is_empty());
        std::fs::write(d.join("bad.json"), "{ not json").unwrap();
        assert!(configured(Some(&d.join("bad.json")), &[]).is_empty());
        assert!(configured(None, &[]).is_empty());
        assert_eq!(config_for(&["x".into()], None, &[]), r#"{"mcpServers":{}}"#);
        let _ = std::fs::remove_dir_all(&d);
    }
}
