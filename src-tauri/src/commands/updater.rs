//! Commands of the updater (`updater:*`): the window's "Check now", "Install and restart" and
//! "Later". The work is done in [`crate::updater`].

use mc_core::validate;
use tauri::{AppHandle, State};

use crate::error::CmdResult;
use crate::state::AppState;
use crate::updater::UpdateInfo;

/// `updater:check`: looks for a newer version now; null when this one is the newest.
#[tauri::command]
pub async fn updater_check(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    crate::updater::check(&app).await
}

/// `updater:install`: downloads the update the last check found, ends the app as quitting does,
/// installs it and starts the new version. Returns only when that failed.
#[tauri::command]
pub async fn updater_install(app: AppHandle) -> CmdResult<()> {
    crate::updater::install(&app).await
}

/// `updater:dismiss`: "Later"; the version is not announced again until the app starts anew.
#[tauri::command]
pub fn updater_dismiss(state: State<'_, AppState>, version: String) -> CmdResult<()> {
    let version = validate::id(&version, "version")?;
    crate::updater::dismiss(&state, version);
    Ok(())
}
