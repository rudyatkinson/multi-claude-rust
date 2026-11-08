//! Commands of the `sessions` package (`sessions:*` and `pty:*` of `src/main/ipc.ts`).
//! Each command: validate with `mc_core::validate`, call the service on `AppState`, return `CmdResult<T>`.
//!
//! `sessions:transfer` (moving a conversation to another account) and `sessions:openExternal`
//! (the external terminal) are Phase 2; see `commands::stubs`.

use mc_core::error::AppError;
use mc_core::terminal_input::is_typing;
use mc_core::types::{AttachResult, PtyResize, PtyWrite, SessionInfo, SessionKind, SessionRequest, SessionResume, TurnEvent};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::app::launch::{restart_session, start_session};
use crate::app::{note_turn, remember_folder};
use crate::error::{CmdError, CmdResult};
use crate::state::AppState;

/// `sessions:create`: a Claude tab in a folder that exists, for an account that exists, where the
/// window asked for it. The folder goes to the top of the recent folders.
#[tauri::command]
pub async fn sessions_create(state: State<'_, AppState>, input: Option<Value>) -> CmdResult<SessionInfo> {
    let input = validate::session_input(&input.unwrap_or(Value::Null))?;
    let request = SessionRequest {
        profile_id: input.profile_id,
        cwd: input.cwd,
        kind: SessionKind::Claude,
        resume: input.resume.map(SessionResume::Mode),
        name: None,
        launch: None,
        prompt: None,
    };
    let info = start_session(&state, &request, input.place)?;
    remember_folder(&state, &request.cwd);
    Ok(info)
}

/// `sessions:restart`: the session again in its tab; `continueLast` (only a real `true`)
/// continues its conversation.
#[tauri::command]
pub async fn sessions_restart(state: State<'_, AppState>, id: String, continue_last: Option<Value>) -> CmdResult<SessionInfo> {
    let id = validate::id(&id, "session")?;
    let continue_last = continue_last.as_ref().is_some_and(validate::flag);
    Ok(restart_session(&state, id, continue_last)?)
}

/// `sessions:revealFolder`: the session's folder in the file manager. The folder comes from the
/// session, not from the caller.
#[tauri::command]
pub async fn sessions_reveal_folder(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "session")?;
    let cwd = state.sessions.cwd_of(id).ok_or("Session not found")?;
    app.opener().open_path(cwd, None::<&str>).map_err(|e| CmdError(e.to_string()))
}

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
    // the terminal's own reports travel this way too: its focus, the mouse, its answers
    if is_typing(&payload.data) {
        // Phase 2: `workflowRuns.input(id)`
        note_turn(&state, &payload.id, &TurnEvent::Typed);
    }
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
