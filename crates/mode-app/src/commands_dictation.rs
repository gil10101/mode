//! The dictation commands (XC-37, XC-38; `ui/src/state/contract.ts`
//! "Dictation"): status, the press / release / cancel path the windows use
//! besides the global hold key, the insert round trip, the model catalog
//! and its downloads, the microphone check, and what the app window shows
//! (`app_context_set`, per window, attached to a "Hey Mode" request). The
//! model commands and the microphone check are the app window's; status,
//! press, release, cancel, `dictation_inserted` and `app_context_set` are
//! both windows'.

use std::sync::Arc;
use std::time::Duration;

use mode_core::settings;
use mode_core::speech::{self, DictationState, models};
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::cmd_error::{CmdError, CmdResult};
use crate::hotkeys::{HotkeyKind, Hotkeys};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// How long the microphone check holds the device open.
const MIC_CHECK: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DictationStatus {
    pub enabled: bool,
    /// The speech engine is compiled into this build.
    pub available: bool,
    pub state: DictationState,
    pub model: String,
    pub model_installed: bool,
    pub hotkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn speech_error(e: speech::SpeechError) -> CmdError {
    match e {
        speech::SpeechError::Unavailable => CmdError::new("unavailable", e.to_string()),
        speech::SpeechError::UnknownModel(_) => CmdError::invalid(e.to_string()),
        other => CmdError::new("io", other.to_string()),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_status(app: AppHandle, state: Core<'_>) -> CmdResult<DictationStatus> {
    let core = Arc::clone(&state);
    let hotkey_error = app.try_state::<Hotkeys>().and_then(|h| {
        h.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failed
            .iter()
            .find(|(k, _)| *k == HotkeyKind::Dictate)
            .map(|(_, why)| format!("the dictation key is not registered: {why}"))
    });
    tauri::async_runtime::spawn_blocking(move || {
        let m = crate::speech::model(&core);
        let enabled = settings::get(&core.db, "dictation.enabled")
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let hotkey = settings::get_string(&core.db, "dictation.hotkey")
            .ok()
            .flatten()
            .unwrap_or_else(|| "Ctrl+Alt+D".into());
        let error = if !speech::AVAILABLE {
            Some(speech::SpeechError::Unavailable.to_string())
        } else {
            hotkey_error
        };
        DictationStatus {
            enabled,
            available: speech::AVAILABLE,
            state: core.dictation.state(),
            model: m.id.to_string(),
            model_installed: models::is_installed(&core.data_dir, m),
            hotkey,
            error,
        }
    })
    .await
    .map_err(CmdError::internal)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_press(app: AppHandle) -> CmdResult<()> {
    crate::speech::press(&app);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_release(app: AppHandle) -> CmdResult<()> {
    crate::speech::release(&app);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_cancel(app: AppHandle) -> CmdResult<()> {
    crate::speech::cancel(&app);
    Ok(())
}

/// Dev builds only (08 §12): route `text` exactly as a hold's transcript
/// would be routed (the wake phrase, the focused window, the reply card),
/// so the real app can be exercised without a microphone. A release
/// build answers `unavailable`.
#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_say(app: AppHandle, state: Core<'_>, text: String) -> CmdResult<()> {
    #[cfg(debug_assertions)]
    {
        let core = Arc::clone(&state);
        tauri::async_runtime::spawn_blocking(move || crate::speech::say(&app, &core, text))
            .await
            .map_err(CmdError::internal)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (app, state, text);
        Err(CmdError::new(
            "unavailable",
            "dictation_say exists in dev builds only",
        ))
    }
}

/// After `dictation:text`: `ok = false` means no editable field had focus,
/// so `dictation.unfocused` applies (a captured note, or nothing).
#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_inserted(state: Core<'_>, ok: bool) -> CmdResult<()> {
    crate::speech::inserted(&state, ok);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DictationModels {
    pub models: Vec<models::ModelRow>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_models(state: Core<'_>) -> CmdResult<DictationModels> {
    let dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || DictationModels {
        models: models::rows(&dir),
    })
    .await
    .map_err(CmdError::internal)
}

/// Start the download (returns at once; progress on `dictation:model`).
#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_model_ensure(
    app: AppHandle,
    state: Core<'_>,
    model: String,
) -> CmdResult<()> {
    crate::speech::ensure_model(&app, &state, &model).map_err(speech_error)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_model_remove(state: Core<'_>, model: String) -> CmdResult<()> {
    let core = Arc::clone(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let removed = models::remove(&core.data_dir, &model)?;
        if removed {
            core.dictation.unload();
            tracing::info!(model = %model, "dictation model removed");
        }
        Ok::<_, speech::SpeechError>(())
    })
    .await
    .map_err(CmdError::internal)?
    .map_err(speech_error)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MicCheck {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Ask the OS for the microphone (its stored answer, or the prompt while it
/// is undecided), then open the input device for 200 ms.
#[tauri::command(rename_all = "snake_case")]
pub async fn dictation_mic_check() -> CmdResult<MicCheck> {
    let r = tauri::async_runtime::spawn_blocking(|| {
        crate::speech::mic_allowed(true)
            .and_then(|()| speech::mic_check(MIC_CHECK).map_err(|e| e.to_string()))
    })
    .await
    .map_err(CmdError::internal)?;
    Ok(match r {
        Ok(()) => MicCheck {
            ok: true,
            error: None,
        },
        Err(e) => MicCheck {
            ok: false,
            error: Some(e),
        },
    })
}

/// What the calling window shows (a note, a message, a session, an
/// event), attached to the next "Hey Mode" request; `null` clears that
/// window's item. The overlay's item (its panel open) wins over the app
/// window's.
#[tauri::command(rename_all = "snake_case")]
pub async fn app_context_set(
    window: WebviewWindow,
    state: Core<'_>,
    kind: Option<String>,
    id: Option<String>,
) -> CmdResult<()> {
    let value = match (kind, id) {
        (Some(kind), Some(id)) if !id.trim().is_empty() => {
            if !crate::speech::CONTEXT_KINDS.contains(&kind.as_str()) {
                return Err(CmdError::invalid(format!(
                    "{kind:?} is not one of {}",
                    crate::speech::CONTEXT_KINDS.join(", ")
                )));
            }
            if id.chars().count() > 256 {
                return Err(CmdError::invalid("the id is too long"));
            }
            Some((kind, id))
        }
        _ => None,
    };
    let label = window.label().to_string();
    let mut ctx = CoreState::lock(&state.context);
    match value {
        Some(item) => {
            ctx.insert(label, item);
        }
        None => {
            ctx.remove(&label);
        }
    }
    Ok(())
}
