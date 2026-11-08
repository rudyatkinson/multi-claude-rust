//! Commands of the `sessions` package (`sessions:*` and `pty:*` of `src/main/ipc.ts`).
//! Each command: validate with `mc_core::validate`, call the service on `AppState`, return `CmdResult<T>`.
//!
//! `sessions:transfer` (moving a conversation to another account) is in `commands::transfer`.

use mc_core::error::AppError;
use mc_core::terminal_input::is_typing;
use mc_core::types::{AttachResult, PtyResize, PtyWrite, SessionInfo, SessionKind, SessionRequest, SessionResume, TurnEvent};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::app::launch::{restart_session, sleep_session, start_session, wake_session};
use crate::app::{note_turn, remember_folder};
use crate::error::{CmdError, CmdResult};
use crate::external_terminal::open_external_session;
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

/// `sessions:openExternal`: the account's Claude in a terminal of its own, in a folder that exists.
/// The folder goes to the top of the recent folders.
#[tauri::command]
pub async fn sessions_open_external(state: State<'_, AppState>, input: Option<Value>) -> CmdResult<()> {
    let input = validate::session_input(&input.unwrap_or(Value::Null))?;
    open_external_session(&state, &input)?;
    remember_folder(&state, &input.cwd);
    Ok(())
}

/// `sessions:restart`: the session again in its tab; `continueLast` (only a real `true`)
/// continues its conversation.
#[tauri::command]
pub async fn sessions_restart(state: State<'_, AppState>, id: String, continue_last: Option<Value>) -> CmdResult<SessionInfo> {
    let id = validate::id(&id, "session")?;
    let continue_last = continue_last.as_ref().is_some_and(validate::flag);
    Ok(restart_session(&state, id, continue_last)?)
}

/// `sessions:sleep` ("Sleep now"): ends the process of a running Claude tab and keeps the tab, with
/// its output and its conversation, until it is woken. Resolves once the process is gone.
#[tauri::command]
pub async fn sessions_sleep(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "session")?;
    Ok(sleep_session(&state, id).await?)
}

/// `sessions:wake`: starts a sleeping tab's Claude again under the same session id, continuing
/// its conversation. A tab that is awake is given back as it is.
#[tauri::command]
pub async fn sessions_wake(state: State<'_, AppState>, id: String) -> CmdResult<SessionInfo> {
    let id = validate::id(&id, "session")?;
    Ok(wake_session(&state, id)?)
}

/// `sessions:setKeepAwake`: a tab with it is never put to sleep for being idle; `on` is only a
/// real `true`.
#[tauri::command]
pub fn sessions_set_keep_awake(state: State<'_, AppState>, id: String, on: Option<Value>) -> CmdResult<()> {
    let id = validate::id(&id, "session")?;
    let on = on.as_ref().is_some_and(validate::flag);
    Ok(state.sessions.set_keep_awake(id, on)?)
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

/// `sessions:close`. Closing a pseudoconsole and cleaning up the session's files may take a moment:
/// off the window's thread.
#[tauri::command]
pub async fn sessions_close(app: AppHandle, id: String) -> CmdResult<()> {
    validate::id(&id, "session")?;
    super::blocking(&app, move |state| {
        state.sessions.close(&id);
        Ok(())
    })
    .await
}

/// `sessions:closeMany`: at most 500 ids, with one announcement.
#[tauri::command]
pub async fn sessions_close_many(app: AppHandle, ids: Vec<String>) -> CmdResult<()> {
    validate::ids_many(&ids)?;
    super::blocking(&app, move |state| {
        state.sessions.close_many(&ids);
        Ok(())
    })
    .await
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
        state.runs.input(&payload.id);
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
