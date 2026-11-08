//! Starting sessions: the checks of `spawn` that need other services, the settings file a Claude
//! session starts with (`prepareSettings`), a new session in an existing tab (`replace`), a
//! restart in the same tab (`restart`), and putting a tab to sleep and waking it.

use std::path::{Path, PathBuf};

use mc_core::error::{AppError, AppResult};
use mc_core::sleep::SLEEP_END_MS;
use mc_core::types::{
    HookNeed, Place, Profile, ResumeConversation, ResumeMode, SessionInfo, SessionRequest, SessionResume, SessionStatus,
};
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
    plan_session_for(state, request, None)
}

/// [`plan_session`], for the session `id` when given (one that wakes keeps its id), else a new one.
fn plan_session_for(state: &AppState, request: &SessionRequest, id: Option<&str>) -> AppResult<SpawnPlan> {
    let profile = state.profiles.get(&request.profile_id).ok_or_else(|| AppError::refused("Profile not found"))?;
    let claude = state.claude_path().ok_or_else(|| AppError::refused(CLAUDE_NOT_FOUND))?;
    if state.profiles.token_missing(&profile.id) {
        return Err(AppError::refused(format!(
            "\"{}\" has no token on this computer. Use \"Replace token\" to enter it again.",
            profile.name
        )));
    }
    let env = state.profiles.env_for(&profile);
    let prepare = |session_id: &str, hooks| prepare_settings(state, &profile, session_id, Path::new(&request.cwd), hooks);
    match id {
        Some(id) => state.sessions.plan_as(id.to_owned(), request, &claude, profile.email_hint.as_deref(), env, prepare),
        None => state.sessions.plan(request, &claude, profile.email_hint.as_deref(), env, prepare),
    }
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
    // An account that shares the main account's memory reads and writes that folder; when the
    // folder cannot be worked out the session keeps its own memory and the user is told.
    match state.memory.auto_memory_directory(&state.profiles.paths().main_claude_dir(), profile, cwd) {
        Ok(Some(dir)) => {
            settings.insert("autoMemoryDirectory".to_owned(), Value::String(dir.to_string_lossy().into_owned()));
        }
        Ok(None) => {}
        Err(warning) => crate::app::notice(state, crate::events::NoticeLevel::Warn, warning),
    }
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
///
/// While it restarts the tab carries the moving mark (not announced: it is over in a moment), so
/// that a move cannot begin in between.
pub fn restart_session(state: &AppState, id: &str, continue_conversation: bool) -> AppResult<SessionInfo> {
    let info = state.sessions.get(id).ok_or_else(|| AppError::refused("Session not found"))?;
    // its process was ended on purpose; the move starts it again over there
    if !state.sessions.try_set_moving(id) {
        return Err(AppError::refused("This conversation is being moved to another account."));
    }
    let restarted = restart_marked(state, id, info, continue_conversation);
    // the new session has an id of its own: the old one is only still there when the restart failed
    state.sessions.set_moving(id, false);
    restarted
}

fn restart_marked(state: &AppState, id: &str, info: SessionInfo, continue_conversation: bool) -> AppResult<SessionInfo> {
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

/// Puts a running Claude tab to sleep ("Sleep now", and the idle check): its process is asked to
/// end, and the tab stays where it is, with its output and its conversation, until it is woken.
/// A tab that a workflow run waits for does not sleep. Resolves once the process is gone.
pub async fn sleep_session(state: &AppState, id: &str) -> AppResult<()> {
    if state.runs.watches(id) {
        return Err(AppError::refused("A tab that a workflow run waits for cannot sleep."));
    }
    let ending = state.sessions.sleep(id, SLEEP_END_MS)?;
    ending.await;
    Ok(())
}

/// Wakes a sleeping tab: its Claude starts again under the same session id, so the tab keeps its
/// place, its name, its launch options and its terminal. It continues its conversation when that
/// is on disk, else the folder's last conversation when the folder has one, else it starts a new
/// one. A tab that is not asleep, or is being woken or moved already, is left as it is.
///
/// While it wakes the tab carries the moving mark (not announced), as for a restart.
pub fn wake_session(state: &AppState, id: &str) -> AppResult<SessionInfo> {
    let info = state.sessions.get(id).ok_or_else(|| AppError::refused("Session not found"))?;
    if info.status != SessionStatus::Sleeping || !state.sessions.try_set_moving(id) {
        return Ok(info);
    }
    let woken = wake_marked(state, id, &info);
    // a woken session has the mark taken off with its new process; this is for a failed start
    state.sessions.set_moving(id, false);
    if let Err(e) = &woken {
        log::warn!("waking a tab: {e}");
    }
    woken
}

fn wake_marked(state: &AppState, id: &str, info: &SessionInfo) -> AppResult<SessionInfo> {
    let mut request = state.sessions.request_of(id).ok_or_else(|| AppError::refused("Session not found"))?;
    let transcript = state.sessions.transcript_of(id);
    let config_dir = state.config_dir_of(&info.profile_id);
    let written = config_dir.as_deref().is_some_and(|dir| has_transcript(dir, info.claude_session_id.as_deref(), transcript.as_deref()));
    // A tab that never got a message starts a new conversation: `-c` would take the folder's last
    // conversation, which may belong to another tab.
    request.resume = match &info.claude_session_id {
        Some(conversation) if written => Some(SessionResume::Conversation(ResumeConversation { id: conversation.clone() })),
        _ => None,
    };
    request.prompt = None;
    let plan = plan_session_for(state, &request, Some(id))?;
    state.sessions.wake(plan)
}
