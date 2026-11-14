//! Transcript tailing (05 §5): locations (`~/.claude/projects/<cwd-with-
//! separators-as-dashes>/<session-id>.jsonl`, subagents under
//! `<session-id>/subagents/agent-<agent-id>.jsonl`), the record parser and
//! extractor ([`records`]), the byte-offset tailer and the database ingest
//! ([`tailer`]), and the debounced file watcher ([`watch`]). Mode opens
//! transcripts read-only and never copies their content into the database.

pub mod page;
pub mod records;
pub mod tailer;
pub mod watch;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use records::{
    Extracted, Record, SubagentSpawn, TranscriptEvent, Usage, UsageIn, extract, extract_text,
    parse_line,
};
pub use tailer::{IngestReport, Tailer, Tailers, ingest};
pub use watch::{WatchMode, Watcher};

/// `~/.claude`.
pub fn claude_home(home: &Path) -> PathBuf {
    home.join(".claude")
}

/// `~/.claude/projects`.
pub fn projects_dir(home: &Path) -> PathBuf {
    claude_home(home).join("projects")
}

/// The projects root of this machine: `$CLAUDE_CONFIG_DIR/projects` when set,
/// else `~/.claude/projects` (05 §5.1).
pub fn default_projects_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("projects"));
    }
    dirs::home_dir().map(|h| projects_dir(&h))
}

/// The project directory name Claude Code derives from a working directory:
/// every character outside `[A-Za-z0-9]` becomes `-` (so `/Users/ana/code/mode`
/// is `-Users-ana-code-mode` and `C:\Users\ana` is `C--Users-ana`).
pub fn project_dir_name(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The main transcript of a session.
pub fn transcript_path(home: &Path, cwd: &str, session_id: &str) -> PathBuf {
    projects_dir(home)
        .join(project_dir_name(cwd))
        .join(format!("{session_id}.jsonl"))
}

/// Where a session's subagent transcripts live.
pub fn subagents_dir(home: &Path, cwd: &str, session_id: &str) -> PathBuf {
    projects_dir(home)
        .join(project_dir_name(cwd))
        .join(session_id)
        .join("subagents")
}

/// The subagents directory next to a session transcript
/// (`<dir>/<session-id>/subagents`).
pub fn subagents_dir_of(transcript: &Path) -> Option<PathBuf> {
    let stem = transcript.file_stem()?;
    Some(transcript.parent()?.join(stem).join("subagents"))
}

/// The session id a transcript path names (`<id>.jsonl`), when it looks like one.
pub fn session_id_from_path(path: &Path) -> Option<String> {
    if path.extension()? != "jsonl" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    (!stem.is_empty()).then(|| stem.to_string())
}

/// `agent-<id>.jsonl` (observed on 2.1.305), else `<id>.jsonl`.
pub fn subagent_file(dir: &Path, agent_id: &str) -> Option<PathBuf> {
    let first = dir.join(format!("agent-{agent_id}.jsonl"));
    if first.is_file() {
        return Some(first);
    }
    let second = dir.join(format!("{agent_id}.jsonl"));
    second.is_file().then_some(second)
}

/// The agent id a subagent transcript names (`agent-<id>.jsonl` or `<id>.jsonl`).
pub fn agent_id_from_path(path: &Path) -> Option<String> {
    let stem = session_id_from_path(path)?;
    Some(stem.strip_prefix("agent-").unwrap_or(&stem).to_string())
}

/// `agent-<id>.meta.json` next to a subagent transcript.
pub fn meta_path(agent_transcript: &Path) -> PathBuf {
    agent_transcript.with_extension("meta.json")
}

/// The sidecar Claude Code writes next to a subagent transcript (observed on
/// 2.1.305: `agentType`, `description`, `toolUseId`, `finished`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentMeta {
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub finished: bool,
}

/// Read the `.meta.json` of a subagent transcript, when present and valid.
pub fn read_meta(agent_transcript: &Path) -> Option<SubagentMeta> {
    let text = std::fs::read_to_string(meta_path(agent_transcript)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Every `*.jsonl` in a subagents directory (sorted), with its agent id.
pub fn list_subagents(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|p| agent_id_from_path(&p).map(|id| (id, p)))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_cwd_to_project_dir_and_back_to_session_id() {
        assert_eq!(
            project_dir_name("/Users/ana/code/mode"),
            "-Users-ana-code-mode"
        );
        assert_eq!(
            project_dir_name(r"C:\Users\ana\my.project"),
            "C--Users-ana-my-project"
        );
        let home = Path::new("/Users/ana");
        let p = transcript_path(home, "/Users/ana/code/mode", "7f3a");
        assert_eq!(
            p,
            PathBuf::from("/Users/ana/.claude/projects/-Users-ana-code-mode/7f3a.jsonl")
        );
        assert_eq!(session_id_from_path(&p).as_deref(), Some("7f3a"));
        assert_eq!(session_id_from_path(Path::new("/x/notes.md")), None);
        assert!(subagents_dir(home, "/Users/ana/code/mode", "7f3a").ends_with("7f3a/subagents"));
        assert_eq!(
            subagents_dir_of(&p).unwrap(),
            PathBuf::from("/Users/ana/.claude/projects/-Users-ana-code-mode/7f3a/subagents")
        );
        assert_eq!(
            agent_id_from_path(Path::new("/x/subagents/agent-a1b2.jsonl")).as_deref(),
            Some("a1b2")
        );
        assert_eq!(
            meta_path(Path::new("/x/subagents/agent-a1b2.jsonl")),
            PathBuf::from("/x/subagents/agent-a1b2.meta.json")
        );
    }

    #[test]
    fn subagent_files_prefer_the_agent_prefix() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(subagent_file(dir.path(), "a1"), None);
        std::fs::write(dir.path().join("a1.jsonl"), "").unwrap();
        assert!(
            subagent_file(dir.path(), "a1")
                .unwrap()
                .ends_with("a1.jsonl")
        );
        std::fs::write(dir.path().join("agent-a1.jsonl"), "").unwrap();
        assert!(
            subagent_file(dir.path(), "a1")
                .unwrap()
                .ends_with("agent-a1.jsonl")
        );
        std::fs::write(
            dir.path().join("agent-a1.meta.json"),
            r#"{"agentType":"general-purpose","description":"d","toolUseId":"t","finished":true,"spawnDepth":1}"#,
        )
        .unwrap();
        let meta = read_meta(&dir.path().join("agent-a1.jsonl")).unwrap();
        assert_eq!(meta.agent_type.as_deref(), Some("general-purpose"));
        assert!(meta.finished);
        let listed = list_subagents(dir.path());
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "a1");
        assert!(list_subagents(&dir.path().join("missing")).is_empty());
    }
}
