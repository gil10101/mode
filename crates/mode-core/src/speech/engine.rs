//! Speech to text (XC-37, ADR-0061): the [`Recognizer`] trait and its
//! whisper.cpp implementation. The model loads once per file, on the first
//! utterance (about half a second); every transcription runs on the
//! `mode-speech` thread with greedy decoding. whisper.cpp's output is
//! already cased and punctuated; there is no clean-up model.

use std::path::Path;

use super::SpeechError;

/// Turns 16 kHz mono samples into text.
pub trait Recognizer: Send + Sync {
    /// `language`: an ISO code, `auto` to detect, `None` for English.
    fn transcribe(&self, pcm16k: &[f32], language: Option<&str>) -> Result<String, SpeechError>;
}

/// Join whisper.cpp segments: drop the non-speech markers it emits for
/// silence (`[BLANK_AUDIO]`, `(wind blowing)`), collapse whitespace.
pub fn tidy<'a>(segments: impl IntoIterator<Item = &'a str>) -> String {
    let joined: Vec<&str> = segments
        .into_iter()
        .flat_map(|s| s.split_whitespace())
        .collect();
    let mut out = String::new();
    let mut skip: Option<char> = None;
    for w in joined {
        if let Some(close) = skip {
            if w.ends_with(close) {
                skip = None;
            }
            continue;
        }
        let open = w.chars().next();
        let close = match open {
            Some('[') => Some(']'),
            Some('(') => Some(')'),
            Some('*') => Some('*'),
            _ => None,
        };
        if let Some(c) = close {
            if !(w.len() > 1 && w.ends_with(c)) {
                skip = Some(c);
            }
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

/// Shortest clip handed to whisper.cpp (it refuses less than a second);
/// shorter ones are padded with silence.
pub const MIN_SAMPLES: usize = 17_600;

#[cfg(feature = "speech")]
pub use whisper::Whisper;

#[cfg(feature = "speech")]
mod whisper {
    use std::path::Path;

    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    use super::{MIN_SAMPLES, Recognizer, tidy};
    use crate::speech::SpeechError;

    /// A loaded whisper.cpp model.
    pub struct Whisper {
        ctx: WhisperContext,
    }

    impl Whisper {
        pub fn load(path: &Path) -> Result<Whisper, SpeechError> {
            // whisper.cpp logs to stderr; without a backend feature the hook
            // discards it.
            whisper_rs::install_logging_hooks();
            let started = std::time::Instant::now();
            // Metal on macOS (`whisper-rs-sys/metal` is on for that target).
            let mut params = WhisperContextParameters::default();
            params.use_gpu(cfg!(target_os = "macos"));
            let ctx = WhisperContext::new_with_params(path, params)
                .map_err(|e| SpeechError::Engine(format!("the model did not load: {e}")))?;
            tracing::info!(
                model = %path.file_name().unwrap_or_default().to_string_lossy(),
                ms = started.elapsed().as_millis() as u64,
                "dictation model loaded"
            );
            Ok(Whisper { ctx })
        }
    }

    impl Recognizer for Whisper {
        fn transcribe(
            &self,
            pcm16k: &[f32],
            language: Option<&str>,
        ) -> Result<String, SpeechError> {
            let engine = |e: whisper_rs::WhisperError| SpeechError::Engine(e.to_string());
            let mut state = self.ctx.create_state().map_err(engine)?;
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some(language.unwrap_or("en")));
            params.set_translate(false);
            params.set_no_context(true);
            params.set_single_segment(false);
            params.set_suppress_blank(true);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            let mut padded;
            let pcm = if pcm16k.len() < MIN_SAMPLES {
                padded = pcm16k.to_vec();
                padded.resize(MIN_SAMPLES, 0.0);
                &padded[..]
            } else {
                pcm16k
            };
            state.full(params, pcm).map_err(engine)?;
            let texts: Vec<String> = state
                .as_iter()
                .map(|seg| {
                    seg.to_str_lossy()
                        .map(|s| s.into_owned())
                        .unwrap_or_default()
                })
                .collect();
            Ok(tidy(texts.iter().map(String::as_str)))
        }
    }
}

/// Load the recognizer for a model file.
pub fn load(path: &Path) -> Result<std::sync::Arc<dyn Recognizer>, SpeechError> {
    #[cfg(feature = "speech")]
    {
        Ok(std::sync::Arc::new(Whisper::load(path)?))
    }
    #[cfg(not(feature = "speech"))]
    {
        let _ = path;
        Err(SpeechError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_drops_markers_and_collapses_whitespace() {
        assert_eq!(
            tidy([" The retry policy", "  is final. "]),
            "The retry policy is final."
        );
        assert_eq!(tidy(["[BLANK_AUDIO]"]), "");
        assert_eq!(
            tidy([" (wind blowing) Hello", "[Music] there"]),
            "Hello there"
        );
        assert_eq!(tidy(["*sigh* fine"]), "fine");
        assert_eq!(tidy(Vec::<&str>::new()), "");
    }
}
