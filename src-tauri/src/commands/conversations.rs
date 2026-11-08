//! Commands of the conversation history (`conversations:*` of `src/main/ipc.ts`).

use mc_core::types::{ConversationEntry, SessionInfo};
use mc_core::validate;
use serde_json::Value;
use tauri::AppHandle;

use crate::app::state as app_state;
use crate::conversations::{list_conversations, open_conversation};
use crate::error::{CmdError, CmdResult};

/// `conversations:list`: the past conversations of every account. The files are read off the
/// window's thread.
#[tauri::command]
pub async fn conversations_list(app: AppHandle) -> CmdResult<Vec<ConversationEntry>> {
    tauri::async_runtime::spawn_blocking(move || app_state(&app).map(|state| list_conversations(&state)).unwrap_or_default())
        .await
        .map_err(|e| CmdError(e.to_string()))
}

/// `conversations:open`: a conversation of the last listing in a new tab, where the window asked
/// for it. Its file is looked for and its process started off the window's thread.
#[tauri::command]
pub async fn conversations_open(app: AppHandle, profile_id: String, id: String, place: Option<Value>) -> CmdResult<SessionInfo> {
    validate::id(&profile_id, "profile")?;
    validate::id(&id, "conversation")?;
    let place = place.as_ref().and_then(validate::place);
    super::blocking(&app, move |state| Ok(open_conversation(state, &profile_id, &id, place)?)).await
}
