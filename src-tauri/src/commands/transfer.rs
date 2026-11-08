//! `sessions:transfer`: the conversation of a tab goes on under another account.

use mc_core::types::SessionInfo;
use mc_core::validate;
use tauri::State;

use crate::error::CmdResult;
use crate::state::AppState;
use crate::transfer::transfer_session;

/// `sessions:transfer`: gives the session the conversation continues in.
#[tauri::command]
pub async fn sessions_transfer(state: State<'_, AppState>, id: String, profile_id: String) -> CmdResult<SessionInfo> {
    let id = validate::id(&id, "session")?;
    let profile_id = validate::id(&profile_id, "profile")?;
    Ok(transfer_session(&state, id, profile_id, false).await?)
}
