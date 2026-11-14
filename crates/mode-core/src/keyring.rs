//! The secret store facade (06 §5, 08 §8): every OAuth token, OAuth client
//! and AI key lives in the OS keyring under the service name `Mode`; SQLite
//! stores only the entry name (`accounts.keyring_ref`). The webview never
//! receives a secret: [`Keyring::present`] and [`Keyring::last4`] are all it
//! sees.
//!
//! Stores:
//! * [`Keyring::os`] — macOS Keychain (the `keychain` module of
//!   `apple-native-keyring-store`) or Windows Credential Manager
//!   (`windows-native-keyring-store`), through `keyring-core`. There is no
//!   OS store on Linux (a development and CI target only), so `os()` fails
//!   there and nothing falls back silently.
//! * [`Keyring::file`] — an explicit file-backed store under the data dir
//!   for Linux development and CI, chosen by the caller (and logged as such).
//! * [`Keyring::memory`] — tests.
//!
//! Rules carried here: entry names per 06 §13 ([`names`]); one mutex per
//! entry for read-modify-write (a token refresh, 06 §5 "Rotation"); values
//! longer than [`CHUNK_CHARS`] are split into `<name>:1`, `<name>:2`, … with
//! the head entry holding a split marker (Windows caps a blob at 2 560
//! bytes); errors never carry a secret.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The keyring service name (06 §5).
pub const SERVICE: &str = "Mode";

/// The largest single entry value, in characters. Windows generic
/// credentials hold 2 560 bytes and the native store writes UTF-16, so a
/// value is split well below 1 280 characters.
pub const CHUNK_CHARS: usize = 1_000;

/// The head entry of a split value.
const SPLIT_PREFIX: &str = "mode-split:";

/// Entry names (06 §5, §13).
pub mod names {
    /// `account:<accounts.id>`: the token set of one account.
    pub fn account(id: &str) -> String {
        format!("account:{id}")
    }
    /// `oauth_client:google`: `{client_id, client_secret, project_id}`.
    pub const OAUTH_CLIENT_GOOGLE: &str = "oauth_client:google";
    /// `oauth_client:slack`: the Slack app of the first workspace.
    pub const OAUTH_CLIENT_SLACK: &str = "oauth_client:slack";
    /// `oauth_client:slack:<team_id>` when more than one workspace app exists.
    pub fn oauth_client_slack(team_id: Option<&str>) -> String {
        match team_id {
            Some(t) if !t.is_empty() => format!("{OAUTH_CLIENT_SLACK}:{t}"),
            _ => OAUTH_CLIENT_SLACK.to_string(),
        }
    }
    /// `ai:anthropic_api_key`: the Claude API key string.
    pub const AI_ANTHROPIC_API_KEY: &str = "ai:anthropic_api_key";
    /// `ai:local_token`: an optional bearer for LM Studio / a protected Ollama.
    pub const AI_LOCAL_TOKEN: &str = "ai:local_token";
    /// `ai:provider:<ai_providers.id>`: the key or token of a provider the
    /// user added (06 §10.1A); the built-ins keep the two entries above.
    pub const AI_PROVIDER_PREFIX: &str = "ai:provider:";
    pub fn ai_provider(id: &str) -> String {
        format!("{AI_PROVIDER_PREFIX}{id}")
    }

    /// Whether `name` is one of the entry names Mode writes.
    pub fn is_known(name: &str) -> bool {
        name.starts_with("account:")
            || name == OAUTH_CLIENT_GOOGLE
            || name == OAUTH_CLIENT_SLACK
            || name.starts_with("oauth_client:slack:")
            || name == AI_ANTHROPIC_API_KEY
            || name == AI_LOCAL_TOKEN
            || (name.starts_with(AI_PROVIDER_PREFIX) && name.len() > AI_PROVIDER_PREFIX.len())
    }
}

/// A keyring failure. Never carries a secret value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyringError {
    /// No OS keyring on this platform, or it is locked / refused access.
    #[error("the OS keyring is unavailable: {0}")]
    Unavailable(String),
    /// The store failed (the message is the platform's, scrubbed).
    #[error("the keyring failed: {0}")]
    Store(String),
    /// A stored value is not what Mode wrote (bad JSON, missing chunk).
    #[error("the keyring entry {0} is malformed")]
    Malformed(String),
    #[error("{0} is not a Mode keyring entry")]
    UnknownName(String),
}

pub type Result<T> = std::result::Result<T, KeyringError>;

/// One backing store: single entries, no splitting, no locking.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    /// `true` when an entry existed.
    fn delete(&self, name: &str) -> Result<bool>;
    /// `os`, `file` or `memory` (Diagnostics).
    fn kind(&self) -> &'static str;
}

// ------------------------------------------------------------------ stores

/// In-memory store (tests).
#[derive(Debug, Default)]
pub struct MemoryStore {
    map: Mutex<BTreeMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(lock(&self.map).get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        lock(&self.map).insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<bool> {
        Ok(lock(&self.map).remove(name).is_some())
    }
    fn kind(&self) -> &'static str {
        "memory"
    }
}

/// The explicit development / CI fallback: one JSON object per file,
/// `0600` on unix, written through a temp file and a rename. Not a secure
/// store; the app picks it only where no OS keyring exists (Linux).
#[derive(Debug)]
pub struct FileStore {
    path: PathBuf,
    guard: Mutex<()>,
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> FileStore {
        FileStore {
            path: path.into(),
            guard: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) if s.trim().is_empty() => Ok(BTreeMap::new()),
            Ok(s) => serde_json::from_str(&s)
                .map_err(|_| KeyringError::Malformed(self.path.display().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(KeyringError::Store(e.kind().to_string())),
        }
    }

    fn save(&self, map: &BTreeMap<String, String>) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| KeyringError::Store(e.kind().to_string()))?;
        }
        let tmp = self.path.with_extension("tmp");
        let text = serde_json::to_string_pretty(map)
            .map_err(|_| KeyringError::Store("serialize".into()))?;
        write_private(&tmp, text.as_bytes())
            .map_err(|e| KeyringError::Store(e.kind().to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| KeyringError::Store(e.kind().to_string()))
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

impl SecretStore for FileStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        let _g = lock(&self.guard);
        Ok(self.load()?.get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        let _g = lock(&self.guard);
        let mut m = self.load()?;
        m.insert(name.to_string(), value.to_string());
        self.save(&m)
    }
    fn delete(&self, name: &str) -> Result<bool> {
        let _g = lock(&self.guard);
        let mut m = self.load()?;
        let had = m.remove(name).is_some();
        if had {
            self.save(&m)?;
        }
        Ok(had)
    }
    fn kind(&self) -> &'static str {
        "file"
    }
}

/// The OS keyring through `keyring-core` (macOS Keychain, Windows
/// Credential Manager).
pub struct OsStore {
    store: Arc<keyring_core::CredentialStore>,
}

impl std::fmt::Debug for OsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsStore").finish_non_exhaustive()
    }
}

fn platform_error(e: keyring_core::Error) -> KeyringError {
    use keyring_core::Error as E;
    match e {
        E::NoStorageAccess(_) => KeyringError::Unavailable("access refused (locked?)".into()),
        E::NoDefaultStore => KeyringError::Unavailable("no store".into()),
        E::BadEncoding(_) | E::BadDataFormat(..) => KeyringError::Malformed("entry".into()),
        E::TooLong(attr, limit) => KeyringError::Store(format!("{attr} longer than {limit}")),
        E::Invalid(what, _) => KeyringError::Store(format!("invalid {what}")),
        _ => KeyringError::Store("platform failure".into()),
    }
}

impl OsStore {
    /// The native store of this OS, or `Unavailable` (Linux).
    pub fn new() -> Result<OsStore> {
        #[cfg(target_os = "macos")]
        {
            let s = apple_native_keyring_store::keychain::Store::new().map_err(platform_error)?;
            Ok(OsStore { store: s })
        }
        #[cfg(windows)]
        {
            let s = windows_native_keyring_store::Store::new().map_err(platform_error)?;
            Ok(OsStore { store: s })
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            Err(KeyringError::Unavailable(
                "no OS keyring is wired on this platform; use the explicit file store".into(),
            ))
        }
    }

    fn entry(&self, name: &str) -> Result<keyring_core::Entry> {
        self.store
            .build(SERVICE, name, None)
            .map_err(platform_error)
    }
}

impl SecretStore for OsStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match self.entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(platform_error(e)),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.entry(name)?
            .set_password(value)
            .map_err(platform_error)
    }
    fn delete(&self, name: &str) -> Result<bool> {
        match self.entry(name)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(e) => Err(platform_error(e)),
        }
    }
    fn kind(&self) -> &'static str {
        "os"
    }
}

// ---------------------------------------------------------------- facade

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The facade: splitting, JSON helpers, per-entry locks, display helpers.
pub struct Keyring {
    store: Arc<dyn SecretStore>,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keyring")
            .field("kind", &self.store.kind())
            .finish_non_exhaustive()
    }
}

/// What the webview may know about an entry (06 §5: `present` and, for API
/// keys, the last four characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretStatus {
    pub name: String,
    pub stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last4: Option<String>,
}

impl Keyring {
    pub fn with_store(store: Arc<dyn SecretStore>) -> Keyring {
        Keyring {
            store,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// The OS keyring; `Unavailable` where none exists (Linux).
    pub fn os() -> Result<Keyring> {
        Ok(Keyring::with_store(Arc::new(OsStore::new()?)))
    }

    /// The explicit file-backed store (Linux development and CI).
    pub fn file(path: impl Into<PathBuf>) -> Keyring {
        let path = path.into();
        tracing::warn!(path = %path.display(), "keyring: using the file store (no OS keyring)");
        Keyring::with_store(Arc::new(FileStore::new(path)))
    }

    /// The default file-store location under the data dir.
    pub fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join("secrets-dev.json")
    }

    pub fn memory() -> Keyring {
        Keyring::with_store(Arc::new(MemoryStore::default()))
    }

    /// `os`, `file` or `memory`.
    pub fn kind(&self) -> &'static str {
        self.store.kind()
    }

    /// The per-entry mutex for a read-modify-write (06 §5 "Rotation").
    pub fn entry_lock(&self, name: &str) -> Arc<Mutex<()>> {
        Arc::clone(
            lock(&self.locks)
                .entry(name.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn check(name: &str) -> Result<()> {
        if names::is_known(name) {
            Ok(())
        } else {
            Err(KeyringError::UnknownName(name.to_string()))
        }
    }

    /// The value, reassembled from its chunks when split.
    pub fn get(&self, name: &str) -> Result<Option<String>> {
        Self::check(name)?;
        let Some(head) = self.store.get(name)? else {
            return Ok(None);
        };
        let Some(n) = head.strip_prefix(SPLIT_PREFIX) else {
            return Ok(Some(head));
        };
        let n: usize = n
            .parse()
            .map_err(|_| KeyringError::Malformed(name.into()))?;
        let mut out = String::new();
        for i in 1..=n {
            let part = self
                .store
                .get(&format!("{name}:{i}"))?
                .ok_or_else(|| KeyringError::Malformed(name.into()))?;
            out.push_str(&part);
        }
        Ok(Some(out))
    }

    /// Store `value`, splitting it when longer than [`CHUNK_CHARS`]; stale
    /// chunks of a previous longer value are removed.
    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        Self::check(name)?;
        let old_chunks = self.chunk_count(name)?;
        let chars: Vec<char> = value.chars().collect();
        if chars.len() <= CHUNK_CHARS {
            self.store.set(name, value)?;
            for i in 1..=old_chunks {
                self.store.delete(&format!("{name}:{i}"))?;
            }
            return Ok(());
        }
        let parts: Vec<String> = chars
            .chunks(CHUNK_CHARS)
            .map(|c| c.iter().collect())
            .collect();
        for (i, p) in parts.iter().enumerate() {
            self.store.set(&format!("{name}:{}", i + 1), p)?;
        }
        self.store
            .set(name, &format!("{SPLIT_PREFIX}{}", parts.len()))?;
        for i in parts.len() + 1..=old_chunks {
            self.store.delete(&format!("{name}:{i}"))?;
        }
        Ok(())
    }

    fn chunk_count(&self, name: &str) -> Result<usize> {
        Ok(self
            .store
            .get(name)?
            .and_then(|h| h.strip_prefix(SPLIT_PREFIX).and_then(|n| n.parse().ok()))
            .unwrap_or(0))
    }

    /// Delete the entry and its chunks; `true` when it existed.
    pub fn delete(&self, name: &str) -> Result<bool> {
        Self::check(name)?;
        let chunks = self.chunk_count(name)?;
        for i in 1..=chunks {
            self.store.delete(&format!("{name}:{i}"))?;
        }
        self.store.delete(name)
    }

    pub fn get_json<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        match self.get(name)? {
            None => Ok(None),
            Some(s) => serde_json::from_str(&s)
                .map(Some)
                .map_err(|_| KeyringError::Malformed(name.into())),
        }
    }

    pub fn set_json<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let s = serde_json::to_string(value).map_err(|_| KeyringError::Malformed(name.into()))?;
        self.set(name, &s)
    }

    pub fn present(&self, name: &str) -> bool {
        matches!(self.get(name), Ok(Some(_)))
    }

    /// The last four characters of a stored string (API keys only, 06 §5).
    pub fn last4(&self, name: &str) -> Option<String> {
        let v = self.get(name).ok()??;
        let chars: Vec<char> = v.chars().collect();
        (chars.len() >= 8).then(|| chars[chars.len() - 4..].iter().collect())
    }

    /// What the webview may see.
    pub fn status(&self, name: &str) -> SecretStatus {
        SecretStatus {
            name: name.to_string(),
            stored: self.present(name),
            last4: if name.starts_with("ai:") {
                self.last4(name)
            } else {
                None
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_06_section_13() {
        assert_eq!(names::account("a1"), "account:a1");
        assert_eq!(names::oauth_client_slack(None), "oauth_client:slack");
        assert_eq!(
            names::oauth_client_slack(Some("T1")),
            "oauth_client:slack:T1"
        );
        assert!(names::is_known("ai:anthropic_api_key"));
        assert!(names::is_known("ai:provider:openrouter"));
        assert!(!names::is_known("ai:provider:"));
        assert!(!names::is_known("random"));
    }

    #[test]
    fn round_trip_split_and_delete() {
        let k = Keyring::memory();
        assert_eq!(k.get("ai:local_token").unwrap(), None);
        k.set("ai:local_token", "short").unwrap();
        assert_eq!(k.get("ai:local_token").unwrap().as_deref(), Some("short"));
        let long: String = "x".repeat(CHUNK_CHARS * 2 + 7);
        k.set("account:a", &long).unwrap();
        assert_eq!(k.get("account:a").unwrap().as_deref(), Some(long.as_str()));
        // shorter again: the chunks are gone
        k.set("account:a", "tiny").unwrap();
        assert_eq!(k.get("account:a").unwrap().as_deref(), Some("tiny"));
        assert!(k.delete("account:a").unwrap());
        assert!(!k.delete("account:a").unwrap());
        assert!(matches!(
            k.set("nope", "x"),
            Err(KeyringError::UnknownName(_))
        ));
    }

    #[test]
    fn status_shows_last4_for_ai_keys_only() {
        let k = Keyring::memory();
        k.set(names::AI_ANTHROPIC_API_KEY, "sk-ant-api03-abcdefWXYZ")
            .unwrap();
        let s = k.status(names::AI_ANTHROPIC_API_KEY);
        assert!(s.stored);
        assert_eq!(s.last4.as_deref(), Some("WXYZ"));
        k.set("account:x", r#"{"access_token":"t"}"#).unwrap();
        assert_eq!(k.status("account:x").last4, None);
    }

    #[test]
    fn file_store_persists_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let k = Keyring::with_store(Arc::new(FileStore::new(&path)));
        k.set_json("account:z", &serde_json::json!({"a": 1}))
            .unwrap();
        let k2 = Keyring::with_store(Arc::new(FileStore::new(&path)));
        let v: serde_json::Value = k2.get_json("account:z").unwrap().unwrap();
        assert_eq!(v["a"], 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_silent_fallback_on_linux() {
        assert!(matches!(Keyring::os(), Err(KeyringError::Unavailable(_))));
    }
}
