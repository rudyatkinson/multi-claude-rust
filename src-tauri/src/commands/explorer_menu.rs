//! Commands of the File Explorer entry (`explorerMenu:*` of `src/main/ipc.ts`).

use mc_core::types::ExplorerMenuStatus;
use mc_core::validate;
use serde_json::Value;
use tauri::State;

use crate::app::emit_app_changed;
use crate::error::CmdResult;
use crate::state::AppState;

/// `explorerMenu:status`: whether the entry is there, and which tier enabling would use.
#[tauri::command]
pub async fn explorer_menu_status(state: State<'_, AppState>) -> CmdResult<ExplorerMenuStatus> {
    Ok(state.explorer_menu.status().await)
}

/// `explorerMenu:set`: adds the entry (only for a real `true`) or removes it; the setting follows
/// what is registered afterwards.
#[tauri::command]
pub async fn explorer_menu_set(state: State<'_, AppState>, enabled: Option<Value>) -> CmdResult<ExplorerMenuStatus> {
    let enabled = enabled.as_ref().is_some_and(validate::flag);
    let status = if enabled { state.explorer_menu.enable().await? } else { state.explorer_menu.disable().await? };
    state.profiles.config().update(|c| c.settings.explorer_menu = Some(status.tier.is_some()))?;
    emit_app_changed(&state);
    Ok(status)
}
