//! Commands of the `attention` package: `session:attention`.

use mc_core::types::SessionAttention;
use mc_core::validate;
use tauri::State;

use crate::error::CmdResult;
use crate::state::AppState;

/// `session:attention` (send): a session stopped working while the user was not looking at it.
/// The window decides that (`shouldAlert`, `callsForYou`); the backend decides whether and how
/// the user hears of it.
#[tauri::command]
pub fn session_attention(payload: SessionAttention, state: State<'_, AppState>) -> CmdResult<()> {
    state.alerts.attention(&payload.id, validate::attention_title(&payload.title));
    Ok(())
}
