//! Commands of the `sessions` package (`sessions:*` and `pty:*` of `src/main/ipc.ts`).
//! Each command: validate with `mc_core::validate`, call the service on `AppState`, return `CmdResult<T>`.
//!
//! Left to integration, because they need other services:
//! - `sessions_create(input)`: `validate::session_input`, profile and claude path, `prepareSettings`,
//!   `SessionManager::plan` + `create` with the place, `rememberFolder`.
//! - `sessions_restart(id, continueLast)`: refuses a `moving` session ("This conversation is being
//!   moved to another account."), `hasTranscript` decides `{ id }` or `last`, then `replace` with
//!   `request_of(id)`.
//! - `sessions_transfer(id, profileId)`: `transferSession` (uses `end`, `set_moving`, `replace`, `set_arrived`).
//! - `sessions_reveal_folder(id)`: `cwd_of(id)` ("Session not found") opened with the opener plugin.
//! - `sessions_open_external(input)`: the external terminal launcher.
//! - `pty_write`: after the write, `if mc_core::terminal_input::is_typing(&data) { sessionTyped(id) }`.

use mc_core::error::AppError;
use mc_core::types::{AttachResult, PtyResize, PtyWrite};
use mc_core::validate;
use serde_json::Value;
use tauri::State;

use crate::error::CmdResult;
use crate::state::AppState;

/// `sessions:attach`: the session, its replay and how much output it has produced; null for an
/// unknown session.
#[tauri::command]
pub fn sessions_attach(state: State<'_, AppState>, id: String) -> CmdResult<Option<AttachResult>> {
    let id = validate::id(&id, "session")?;
    Ok(state.sessions.attach(id))
}

/// `sessions:close`.
#[tauri::command]
pub fn sessions_close(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "session")?;
    state.sessions.close(id);
    Ok(())
}

/// `sessions:closeMany`: at most 500 ids, with one announcement.
#[tauri::command]
pub fn sessions_close_many(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    let ids = validate::ids_many(&ids)?;
    state.sessions.close_many(&ids);
    Ok(())
}

/// `sessions:rename`: `tabName(name)`; a name that is empty once cleaned gives the tab its
/// automatic title back. Anything but a text is `Invalid name`.
#[tauri::command]
pub fn sessions_rename(state: State<'_, AppState>, id: String, name: Option<Value>) -> CmdResult<()> {
    let id = validate::id(&id, "session")?;
    let Some(name) = name.as_ref().and_then(Value::as_str) else {
        return Err(AppError::invalid("Invalid name").into());
    };
    state.sessions.rename(id, validate::tab_name(name));
    Ok(())
}

/// `pty:write` (send): what the user typed, pasted, or the terminal answered.
#[tauri::command]
pub fn pty_write(state: State<'_, AppState>, payload: PtyWrite) -> CmdResult<()> {
    state.sessions.write(&payload.id, &payload.data);
    Ok(())
}

/// `pty:resize` (send): a size that is not whole numbers from 2 to 1000 is ignored.
#[tauri::command]
pub fn pty_resize(state: State<'_, AppState>, payload: PtyResize) -> CmdResult<()> {
    if let Some((cols, rows)) = validate::pty_size(payload.cols, payload.rows) {
        state.sessions.resize(&payload.id, cols, rows);
    }
    Ok(())
}
