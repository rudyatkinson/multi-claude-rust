//! Commands of the `usage` package. The renderer has no usage channel of its own: it gets the
//! numbers with `app:getState` and `usage:changed`, and `profiles:refreshStatus` (a profiles
//! command, wired at integration) checks the account and then asks `/usage` again, which is
//! [`usage_refresh`]. It is not registered with the builder.

use tauri::State;

use crate::error::CmdResult;
use crate::state::AppState;

/// Asks `/usage` for one account and waits for the answer: true when fresh numbers came.
#[tauri::command]
pub async fn usage_refresh(state: State<'_, AppState>, profile_id: String) -> CmdResult<bool> {
    let profile_id = mc_core::validate::id(&profile_id, "profile id")?;
    Ok(state.usage.refresh(profile_id).await)
}
