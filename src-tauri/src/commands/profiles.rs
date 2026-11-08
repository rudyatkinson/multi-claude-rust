//! Commands of the `profiles` package: `profiles:*` and `folders:setAccount`.
//!
//! The commands run on the async runtime (`async fn`), not on the main thread: they touch files
//! and run `claude auth`.

use mc_core::error::AppError;
use mc_core::types::{AuthKind, CreateProfileInput, CreateProfileResult, SessionInfo, SessionKind, SessionRequest, SyncResult};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::app::launch::start_session;
use crate::app::{emit_app_changed, notice};
use crate::error::{CmdError, CmdResult};
use crate::events::NoticeLevel;
use crate::state::AppState;

/// A `login` session for an account: `claude auth login` in a tab.
fn login_request(profile_id: &str, cwd: String) -> SessionRequest {
    SessionRequest { profile_id: profile_id.to_owned(), cwd, kind: SessionKind::Login, resume: None, name: None, launch: None, prompt: None }
}

/// `profiles:create`: makes the account; a browser-login account gets its login tab right away,
/// in its new config dir (when that cannot start, it is one more warning).
#[tauri::command]
pub async fn profiles_create(state: State<'_, AppState>, input: CreateProfileInput) -> CmdResult<CreateProfileResult> {
    let (profile, mut warnings) = state.profiles.create(&input)?;
    let mut login_session = None;
    if profile.auth != Some(AuthKind::Token) {
        let cwd = profile.config_dir.clone().unwrap_or_else(|| state.home_dir().to_string_lossy().into_owned());
        match start_session(&state, &login_request(&profile.id, cwd), None) {
            Ok(info) => login_session = Some(info),
            Err(e) => warnings.push(e.to_string()),
        }
    }
    let profile = state.profiles.view(&profile.id).ok_or("Profile not found")?;
    Ok(CreateProfileResult { profile, login_session, warnings })
}

/// `profiles:update`: name, color, memory sharing.
#[tauri::command]
pub async fn profiles_update(state: State<'_, AppState>, id: String, patch: Option<Value>) -> CmdResult<()> {
    let id = validate::id(&id, "profile")?;
    let patch = validate::profile_patch(&patch.unwrap_or(Value::Null));
    state.profiles.update(id, &patch)?;
    Ok(())
}

/// `profiles:delete`: the account's sessions closed, its login removed, its folder in the trash.
#[tauri::command]
pub async fn profiles_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "profile")?;
    Ok(state.profiles.delete(id).await?)
}

/// `profiles:login`: a login tab for a browser-login account, in its config dir (home for Main).
#[tauri::command]
pub async fn profiles_login(state: State<'_, AppState>, id: String) -> CmdResult<SessionInfo> {
    let id = validate::id(&id, "profile")?;
    let profile = state.profiles.get(id).ok_or("Profile not found")?;
    if profile.auth == Some(AuthKind::Token) {
        return Err(AppError::refused("A token account has no browser login. Use \"Replace token\" instead.").into());
    }
    let cwd = profile.config_dir.clone().unwrap_or_else(|| state.home_dir().to_string_lossy().into_owned());
    Ok(start_session(&state, &login_request(&profile.id, cwd), None)?)
}

/// `profiles:logout`: the account's sessions closed, `claude auth logout`, its status read again.
#[tauri::command]
pub async fn profiles_logout(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "profile")?;
    Ok(state.profiles.logout(id).await?)
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

/// `profiles:refreshStatus`: reads the account's status, warns when another account is signed in
/// to the same one, and asks `/usage` again; the caller shows progress until the numbers are in.
#[tauri::command]
pub async fn profiles_refresh_status(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "profile")?;
    if let Some(warning) = state.profiles.refresh_status(id).await {
        notice(&state, NoticeLevel::Warn, warning);
    }
    state.usage.refresh(id).await;
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
