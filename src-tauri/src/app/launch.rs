//! Starting sessions: the checks of `spawn` that need other services, the settings file a Claude
//! session starts with (`prepareSettings`), a new session in an existing tab (`replace`), and a
//! restart in the same tab (`restart`).

use std::path::{Path, PathBuf};

use mc_core::error::{AppError, AppResult};
use mc_core::types::{HookNeed, Place, Profile, ResumeConversation, ResumeMode, SessionInfo, SessionRequest, SessionResume};
use serde_json::{Map, Value};

use crate::config_store::write_json_atomic;
use crate::sessions::SpawnPlan;
use crate::state::AppState;
use crate::tab_store::has_transcript;

pub const CLAUDE_NOT_FOUND: &str = "claude was not found. Set its path in Settings.";
const HOOKS_REQUIRED: &str = "The app cannot receive the signals of this session right now. Restart Multi Claude and try again.";

/// How a session of `request` starts: the account must exist, claude must have been found, a
/// token account must have its token, and a Claude session gets its settings file.
pub fn plan_session(state: &AppState, request: &SessionRequest) -> AppResult<SpawnPlan> {
    let profile = state.profiles.get(&request.profile_id).ok_or_else(|| AppError::refused("Profile not found"))?;
    let claude = state.claude_path().ok_or_else(|| AppError::refused(CLAUDE_NOT_FOUND))?;
    if state.profiles.token_missing(&profile.id) {
        return Err(AppError::refused(format!(
            "\"{}\" has no token on this computer. Use \"Replace token\" to enter it again.",
            profile.name
        )));
    }
    let env = state.profiles.env_for(&profile);
    state.sessions.plan(request, &claude, profile.email_hint.as_deref(), env, |session_id, hooks| {
        prepare_settings(state, &profile, session_id, Path::new(&request.cwd), hooks)
    })
}

/// Starts a session and announces it; `place` says where the window should put its tab.
pub fn start_session(state: &AppState, request: &SessionRequest, place: Option<Place>) -> AppResult<SessionInfo> {
    let mut plan = plan_session(state, request)?;
    plan.place = place;
    state.sessions.create(plan)
}

/// Starts a session of `request` in the tab of `old_id` (`sessions.replace(id, request)` of the
/// Electron version): the old session goes once the new one runs.
pub fn replace_session(state: &AppState, old_id: &str, request: &SessionRequest) -> AppResult<SessionInfo> {
    let plan = plan_session(state, request)?;
    state.sessions.replace(old_id, plan)
}

/// `prepareSettings`: writes the settings file a session is started with, `run/<profile>.<session>.settings.json`:
/// the status line tap for its usage numbers and the hooks that report its turns. A tab works
/// without either, as it did before it had them; a session that must report (a workflow step)
/// does not start without its hooks. None when there is nothing to write.
pub fn prepare_settings(state: &AppState, profile: &Profile, session_id: &str, cwd: &Path, hooks: HookNeed) -> AppResult<Option<PathBuf>> {
    let mut settings = Map::new();
    let config_dir = state.profiles.paths().config_dir_of(profile);
    if let Some(Value::Object(tap)) = state.usage.status_line_setting(cwd, &config_dir, &profile.id, session_id, &profile.name) {
        settings.extend(tap);
    }
    if hooks != HookNeed::Off {
        match state.hooks.register(session_id) {
            Some(registration) => {
                if let Value::Object(block) = registration.settings() {
                    settings.extend(block);
                }
            }
            None if hooks == HookNeed::Required => return Err(AppError::refused(HOOKS_REQUIRED)),
            None => {}
        }
    }
    // TODO(Phase 3): `autoMemoryDirectory` for an account that shares the main account's memory
    // (`profile.share_memory && !profile.is_default`), with a notice when that folder is refused.
    if settings.is_empty() {
        return Ok(None);
    }
    let file = state.usage.files_for(&profile.id, session_id).settings_file;
    match write_json_atomic(&file, &Value::Object(settings)) {
        Ok(()) => Ok(Some(file)),
        Err(e) if hooks == HookNeed::Required => Err(e),
        Err(e) => {
            log::warn!("settings file of session {session_id}: {e}");
            // nothing will post to the address
            state.hooks.release(session_id);
            Ok(None)
        }
    }
}

/// `restart`: starts the session again in its tab, on the same conversation if asked. A session
/// that ended before its first message left no conversation to resume by id: then the folder's
/// last conversation is continued.
pub fn restart_session(state: &AppState, id: &str, continue_conversation: bool) -> AppResult<SessionInfo> {
    let info = state.sessions.get(id).ok_or_else(|| AppError::refused("Session not found"))?;
    // its process was ended on purpose; the move starts it again over there
    if info.moving {
        return Err(AppError::refused("This conversation is being moved to another account."));
    }
    let mut request = state.sessions.request_of(id).ok_or_else(|| AppError::refused("Session not found"))?;
    let transcript = state.sessions.transcript_of(id);
    let written = state
        .config_dir_of(&info.profile_id)
        .is_some_and(|dir| has_transcript(&dir, info.claude_session_id.as_deref(), transcript.as_deref()));
    request.resume = match (continue_conversation, info.claude_session_id) {
        (false, _) => None,
        (true, Some(conversation)) if written => Some(SessionResume::Conversation(ResumeConversation { id: conversation })),
        (true, _) => Some(SessionResume::Mode(ResumeMode::Last)),
    };
    request.prompt = None;
    replace_session(state, id, &request)
}
