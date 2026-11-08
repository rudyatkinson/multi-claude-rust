//! Commands of the `profiles` package: `profiles:update`, `profiles:setToken`,
//! `profiles:syncFromMain`, `profiles:openConfigDir` and `folders:setAccount`.
//!
//! Left to the integration, as they need the sessions and the usage tracker:
//! - `profiles_create`: `ProfileService::create`, then for a login account a `login` session in
//!   the new config dir (its error becomes a warning); answers
//!   `CreateProfileResult { profile: view(id), loginSession, warnings }`.
//! - `profiles_delete`: `ProfileService::delete` (the sessions are closed through the
//!   `set_close_sessions` hook).
//! - `profiles_login`: refused for a token account ("A token account has no browser login. Use
//!   \"Replace token\" instead."); else a `login` session in the config dir (home for Main).
//! - `profiles_logout`: `ProfileService::logout`.
//! - `profiles_refresh_status`: `ProfileService::refresh_status`, its warning as a `warn` notice,
//!   then the usage refresh of the account.
//!
//! The commands run on the async runtime (`async fn`), not on the main thread: they touch files.

use std::sync::PoisonError;

use mc_core::types::{AppChanged, SyncResult};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::{CmdError, CmdResult};
use crate::events;
use crate::state::AppState;

/// Sends `app:changed`: the settings, the claude binary, the recent folders and the folder pins.
pub fn emit_app_changed(state: &AppState) {
    let claude = state.claude.read().unwrap_or_else(PoisonError::into_inner).clone();
    let payload = state.profiles.config().with(|c| AppChanged {
        settings: c.settings.clone(),
        claude,
        recent_folders: c.recent_folders.clone(),
        folder_accounts: c.folder_accounts.clone(),
    });
    events::emit(&state.app, events::APP_CHANGED, payload);
}

/// `profiles:update`: name, color, memory sharing.
#[tauri::command]
pub async fn profiles_update(state: State<'_, AppState>, id: String, patch: Option<Value>) -> CmdResult<()> {
    let id = validate::id(&id, "profile")?;
    let patch = validate::profile_patch(&patch.unwrap_or(Value::Null));
    state.profiles.update(id, &patch)?;
    Ok(())
}

/// `profiles:setToken`: replaces the token of a token account and reads its status again.
#[tauri::command]
pub async fn profiles_set_token(state: State<'_, AppState>, id: String, token: String) -> CmdResult<()> {
    let id = validate::id(&id, "profile")?;
    let token = validate::str(&token, "token", validate::STR_MAX)?;
    state.profiles.set_token(id, token)?;
    state.profiles.refresh_status(id).await;
    Ok(())
}

/// `profiles:syncFromMain`: links and copies again; `force` (only a real `true`) overwrites local edits.
#[tauri::command]
pub async fn profiles_sync_from_main(state: State<'_, AppState>, id: String, force: Option<Value>) -> CmdResult<SyncResult> {
    let id = validate::id(&id, "profile")?;
    let force = force.as_ref().is_some_and(validate::flag);
    Ok(state.profiles.sync_from_main(id, force)?)
}

/// `profiles:openConfigDir`: the account's config dir in the file manager.
#[tauri::command]
pub async fn profiles_open_config_dir(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "profile")?;
    let profile = state.profiles.get(id).ok_or("Profile not found")?;
    let dir = state.profiles.paths().config_dir_of(&profile);
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| CmdError(e.to_string()))
}

/// `folders:setAccount`: pins a folder to an account, or removes its pin with `null`.
#[tauri::command]
pub async fn folders_set_account(state: State<'_, AppState>, cwd: String, profile_id: Option<String>) -> CmdResult<()> {
    let cwd = validate::str(&cwd, "folder", validate::STR_MAX)?;
    let profile_id = profile_id.as_deref().map(|id| validate::id(id, "profile")).transpose()?;
    state.profiles.set_folder_account(cwd, profile_id)?;
    emit_app_changed(&state);
    Ok(())
}
