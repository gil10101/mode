//! The error the newer commands reject with: `{code, message, action?,
//! fields?}`, so the webview can branch on `code` (`not_found`, `invalid`,
//! `read_only`, `io`, `internal`, `unavailable`, `pending`, `busy`, and the
//! PTY host's `refused`, `claude_missing`, `cwd_missing`,
//! `transcript_missing`) and show `message`. `action` (`jump` | `resume`)
//! comes only with a prompt-box refusal of the PTY host (05 §14) and names
//! the button to offer; `fields` (`[{field, message}]`) only with
//! `automation_save`'s `invalid` (`SaveError` in `contract-automations.ts`).

use mode_core::db::DbError;
use mode_core::notes::api::ApiError;
use mode_core::pty::ApiError as PtyError;
use mode_core::scheduler::actions::FieldError;
use mode_core::scheduler::api::SaveError;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CmdError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<FieldError>>,
}

pub type CmdResult<T> = Result<T, CmdError>;

impl CmdError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        CmdError {
            code: code.into(),
            message: message.into(),
            action: None,
            fields: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::new("internal", e.to_string())
    }

    pub fn io(e: impl std::fmt::Display) -> Self {
        Self::new("io", e.to_string())
    }

    /// A feature that arrives with a later package (the AI backends).
    pub fn pending(message: impl Into<String>) -> Self {
        Self::new("pending", message)
    }
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CmdError {}

impl From<ApiError> for CmdError {
    fn from(e: ApiError) -> Self {
        CmdError {
            code: e.code,
            message: e.message,
            action: None,
            fields: None,
        }
    }
}

impl From<PtyError> for CmdError {
    fn from(e: PtyError) -> Self {
        CmdError {
            code: e.code,
            message: e.message,
            action: e.action,
            fields: None,
        }
    }
}

impl From<SaveError> for CmdError {
    fn from(e: SaveError) -> Self {
        CmdError {
            code: e.code,
            message: e.message,
            action: None,
            fields: (!e.fields.is_empty()).then_some(e.fields),
        }
    }
}

impl From<DbError> for CmdError {
    fn from(e: DbError) -> Self {
        ApiError::from(e).into()
    }
}

impl From<rusqlite::Error> for CmdError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::from(e).into()
    }
}

impl From<tauri::Error> for CmdError {
    fn from(e: tauri::Error) -> Self {
        Self::internal(e)
    }
}

impl From<std::io::Error> for CmdError {
    fn from(e: std::io::Error) -> Self {
        Self::io(e)
    }
}

/// Run blocking work (database, files, child processes) on the blocking pool.
pub async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    F: FnOnce() -> CmdResult<T> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(CmdError::internal)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use mode_core::pty::driver::Action;

    #[test]
    fn serialises_as_code_and_message_and_keeps_the_pty_action() {
        let e: CmdError = DbError::NotFound {
            table: "sessions",
            key: "s1".into(),
        }
        .into();
        assert_eq!(e.code, "not_found");
        let v = serde_json::to_value(CmdError::pending("later")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"code": "pending", "message": "later"})
        );
        let e: CmdError = DbError::Invalid {
            what: "preset",
            detail: "nope".into(),
        }
        .into();
        assert_eq!((e.code.as_str(), e.message.as_str()), ("invalid", "nope"));
        let refused: CmdError = PtyError::refused("Claude is working", Some(Action::Jump)).into();
        let v = serde_json::to_value(&refused).unwrap();
        assert_eq!(v["code"], "refused");
        assert_eq!(v["action"], "jump");
        let plain: CmdError = PtyError::not_found("no terminal").into();
        assert!(serde_json::to_value(plain).unwrap().get("action").is_none());
        // automation_save's refusal carries its fields; others none.
        let invalid: CmdError = SaveError {
            code: "invalid".into(),
            message: "Name the automation.".into(),
            fields: vec![mode_core::scheduler::actions::ferr(
                "name",
                "Name the automation.",
            )],
        }
        .into();
        assert_eq!(
            serde_json::to_value(&invalid).unwrap(),
            serde_json::json!({"code": "invalid", "message": "Name the automation.",
                               "fields": [{"field": "name", "message": "Name the automation."}]})
        );
        let gone: CmdError = SaveError::from(ApiError {
            code: "not_found".into(),
            message: "No automation x.".into(),
        })
        .into();
        assert!(serde_json::to_value(gone).unwrap().get("fields").is_none());
    }
}
