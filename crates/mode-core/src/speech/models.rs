//! The whisper.cpp models dictation can use (XC-37, 03 §8.1 Dictation): the
//! quantized `ggml-<id>-q5_1.bin` files of the whisper.cpp repository on
//! Hugging Face, each pinned by size and sha256 (read from the repository's
//! LFS pointers, 2026-11-03). A model downloads into
//! `<app data>/models/whisper/` when dictation is turned on (or another is
//! picked), is verified before it is used, and can be removed again.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::SpeechError;

/// Where the files come from.
pub const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// The default model (`dictation.model`).
pub const DEFAULT_MODEL: &str = "base.en";

/// One model of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelDef {
    pub id: &'static str,
    pub label: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    /// Understands languages other than English (`dictation.language` applies).
    pub multilingual: bool,
}

impl ModelDef {
    pub fn file_name(&self) -> String {
        format!("ggml-{}-q5_1.bin", self.id)
    }

    pub fn url(&self) -> String {
        format!("{BASE_URL}{}", self.file_name())
    }
}

pub const CATALOGUE: &[ModelDef] = &[
    ModelDef {
        id: "tiny.en",
        label: "Tiny (English)",
        size_bytes: 32_166_155,
        sha256: "c77c5766f1cef09b6b7d47f21b546cbddd4157886b3b5d6d4f709e91e66c7c2b",
        multilingual: false,
    },
    ModelDef {
        id: "base.en",
        label: "Base (English)",
        size_bytes: 59_721_011,
        sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
        multilingual: false,
    },
    ModelDef {
        id: "small.en",
        label: "Small (English)",
        size_bytes: 190_098_681,
        sha256: "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        multilingual: false,
    },
    ModelDef {
        id: "base",
        label: "Base (many languages)",
        size_bytes: 59_707_625,
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        multilingual: true,
    },
];

pub fn find(id: &str) -> Option<&'static ModelDef> {
    CATALOGUE.iter().find(|m| m.id == id)
}

/// `<app data>/models/whisper`.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join("whisper")
}

/// Where `model` lives once installed.
pub fn path(data_dir: &Path, model: &ModelDef) -> PathBuf {
    dir(data_dir).join(model.file_name())
}

/// Installed means the file is there at its pinned size (the digest was
/// checked when it was written).
pub fn is_installed(data_dir: &Path, model: &ModelDef) -> bool {
    std::fs::metadata(path(data_dir, model)).is_ok_and(|m| m.len() == model.size_bytes)
}

/// One row of `dictation_models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelRow {
    pub id: String,
    pub label: String,
    pub size_bytes: u64,
    pub installed: bool,
    pub multilingual: bool,
}

pub fn rows(data_dir: &Path) -> Vec<ModelRow> {
    CATALOGUE
        .iter()
        .map(|m| ModelRow {
            id: m.id.into(),
            label: m.label.into(),
            size_bytes: m.size_bytes,
            installed: is_installed(data_dir, m),
            multilingual: m.multilingual,
        })
        .collect()
}

/// `dictation.model` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelProgress {
    pub model: String,
    pub downloaded: u64,
    pub total: u64,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Remove an installed model (and a partial download).
pub fn remove(data_dir: &Path, id: &str) -> Result<bool, SpeechError> {
    let m = find(id).ok_or_else(|| SpeechError::UnknownModel(id.into()))?;
    let p = path(data_dir, m);
    let _ = std::fs::remove_file(part_path(&p));
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(SpeechError::Io(e.to_string())),
    }
}

fn part_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Fetches a URL in chunks: `on_chunk(bytes)` per chunk, `total` when
/// known. The app passes the core's reqwest client
/// ([`crate::connectors::http::ReqwestClient::download`]); tests pass bytes.
pub trait Fetch: Send + Sync {
    fn fetch(
        &self,
        url: &str,
        on_start: &mut dyn FnMut(Option<u64>),
        on_chunk: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    ) -> Result<(), String>;
}

/// Download `id` unless it is installed: write `<file>.part`, hash as it
/// arrives, check the size and the sha256, then rename into place.
/// `progress` gets at most a few calls a second, and one with `done`.
pub fn ensure(
    data_dir: &Path,
    id: &str,
    fetch: &dyn Fetch,
    progress: &mut dyn FnMut(ModelProgress),
) -> Result<PathBuf, SpeechError> {
    let m = find(id).ok_or_else(|| SpeechError::UnknownModel(id.into()))?;
    let dest = path(data_dir, m);
    let report = |downloaded: u64, done: bool, error: Option<String>| ModelProgress {
        model: m.id.into(),
        downloaded,
        total: m.size_bytes,
        done,
        error,
    };
    if is_installed(data_dir, m) {
        progress(report(m.size_bytes, true, None));
        return Ok(dest);
    }
    let result = download(m, &dest, fetch, &mut |n| progress(report(n, false, None)));
    match &result {
        Ok(()) => progress(report(m.size_bytes, true, None)),
        Err(e) => progress(report(0, true, Some(e.to_string()))),
    }
    result.map(|()| dest)
}

fn download(
    m: &ModelDef,
    dest: &Path,
    fetch: &dyn Fetch,
    progress: &mut dyn FnMut(u64),
) -> Result<(), SpeechError> {
    let io = |e: std::io::Error| SpeechError::Io(e.to_string());
    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(io)?;
    let part = part_path(dest);
    let mut file = std::fs::File::create(&part).map_err(io)?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut last = Instant::now();
    let fetched = fetch.fetch(
        &m.url(),
        &mut |total| {
            if let Some(t) = total
                && t != m.size_bytes
            {
                tracing::warn!(
                    model = m.id,
                    total = t,
                    "model size differs from the catalog"
                );
            }
        },
        &mut |chunk| {
            written += chunk.len() as u64;
            if written > m.size_bytes {
                return Err(std::io::Error::other("larger than the catalog says"));
            }
            hasher.update(chunk);
            file.write_all(chunk)?;
            if last.elapsed() >= Duration::from_millis(250) {
                last = Instant::now();
                progress(written);
            }
            Ok(())
        },
    );
    let finish = (|| {
        fetched.map_err(SpeechError::Download)?;
        file.flush().map_err(io)?;
        file.sync_all().map_err(io)?;
        if written != m.size_bytes {
            return Err(SpeechError::Download(format!(
                "the download stopped at {written} of {} bytes",
                m.size_bytes
            )));
        }
        let digest: String = hasher
            .clone()
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if digest != m.sha256 {
            return Err(SpeechError::Download(
                "the downloaded model does not match its checksum".into(),
            ));
        }
        std::fs::rename(&part, dest).map_err(io)
    })();
    if finish.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    finish
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bytes(Vec<u8>);

    impl Fetch for Bytes {
        fn fetch(
            &self,
            _url: &str,
            on_start: &mut dyn FnMut(Option<u64>),
            on_chunk: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
        ) -> Result<(), String> {
            on_start(Some(self.0.len() as u64));
            for c in self.0.chunks(7) {
                on_chunk(c).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
    }

    #[test]
    fn the_catalogue_names_four_q5_files_with_base_en_as_default() {
        assert_eq!(CATALOGUE.len(), 4);
        assert!(find(DEFAULT_MODEL).is_some());
        assert_eq!(
            find("tiny.en").unwrap().url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en-q5_1.bin"
        );
        for m in CATALOGUE {
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert_eq!(m.multilingual, !m.id.ends_with(".en"));
        }
    }

    #[test]
    fn a_download_that_fails_its_checksum_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let m = find("tiny.en").unwrap();
        let mut seen = Vec::new();
        let err = ensure(dir.path(), "tiny.en", &Bytes(vec![1; 40]), &mut |p| {
            seen.push(p)
        })
        .unwrap_err();
        assert!(matches!(err, SpeechError::Download(_)), "{err}");
        assert!(!path(dir.path(), m).exists());
        assert!(!part_path(&path(dir.path(), m)).exists());
        let last = seen.last().unwrap();
        assert!(last.done && last.error.is_some());
        assert!(!is_installed(dir.path(), m));
        assert!(matches!(
            ensure(dir.path(), "huge", &Bytes(vec![]), &mut |_| {}),
            Err(SpeechError::UnknownModel(_))
        ));
        assert!(!remove(dir.path(), "tiny.en").unwrap());
    }
}
