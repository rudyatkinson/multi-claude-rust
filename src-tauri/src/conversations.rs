//! The conversation history (`listConversations` and `openConversation` of `src/main/index.ts`):
//! the past conversations of every account, read with the index of `mc_core::conversations`,
//! and opening one in a new tab with the account it belongs to.

use std::path::PathBuf;

use mc_core::error::{AppError, AppResult};
use mc_core::transfer::find_transcript;
use mc_core::types::{ConversationEntry, Place, ResumeConversation, SessionInfo, SessionKind, SessionRequest, SessionResume};

use crate::app::launch::start_session;
use crate::app::remember_folder;
use crate::state::{lock, AppState};

/// The past conversations of every account (Main's in `~/.claude`). Files that did not change
/// since the last listing are not read again.
///
/// The files are read outside the lock of the index, so that opening a conversation of the last
/// listing does not wait for the next one; one listing reads at a time, with what the one before
/// it read, and its index takes the place of the old one at the end.
pub fn list_conversations(state: &AppState) -> Vec<ConversationEntry> {
    let paths = state.profiles.paths();
    let profiles: Vec<(String, PathBuf)> = state.profiles.list().iter().map(|p| (p.id.clone(), paths.config_dir_of(p))).collect();
    let _one_at_a_time = lock(&state.conversation_scan);
    let mut index = lock(&state.conversations).take_reading();
    let entries = index.list(&profiles).to_vec();
    *lock(&state.conversations) = index;
    entries
}

/// Opens a conversation from the last listing in a new tab, with the account it belongs to. The
/// folder comes from the app's own reading of the file, not from the caller.
pub fn open_conversation(state: &AppState, profile_id: &str, id: &str, place: Option<Place>) -> AppResult<SessionInfo> {
    let profile = state.profiles.get(profile_id);
    let entry = lock(&state.conversations).find(profile_id, id).cloned();
    let (Some(profile), Some(entry)) = (profile, entry) else { return Err(gone()) };
    if find_transcript(&state.profiles.paths().config_dir_of(&profile), id, None).is_none() {
        return Err(gone());
    }
    let request = SessionRequest {
        profile_id: profile.id,
        cwd: entry.cwd.clone(),
        kind: SessionKind::Claude,
        resume: Some(SessionResume::Conversation(ResumeConversation { id: id.to_owned() })),
        name: None,
        launch: None,
        prompt: None,
    };
    let info = start_session(state, &request, place)?;
    remember_folder(state, &entry.cwd);
    Ok(info)
}

fn gone() -> AppError {
    AppError::refused("That conversation is no longer there.")
}
