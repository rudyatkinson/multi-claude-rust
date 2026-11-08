//! Continuing a tab's conversation under another account (`transferSession` and
//! `moveConversation` of `src/main/index.ts`): the conversation's files are copied into that
//! account's config dir and the tab starts again there. The source account keeps its copy. The
//! files and the checks are in `mc_core::transfer`.

use mc_core::error::{AppError, AppResult};
use mc_core::transfer::{can_replace, copy_conversation, plan_transfer, PICK_UP_MESSAGE};
use mc_core::types::{
    Arrival, ResumeConversation, SessionInfo, SessionKind, SessionRequest, SessionResume, Turn, TurnKind,
};
use mc_core::usage::used_up;

use crate::app::launch::replace_session;
use crate::app::now_ms;
use crate::claude_binary::is_shim;
use crate::state::AppState;

/// A process that is still writing when it is told to end gets this long.
pub const END_BEFORE_COPY_MS: u64 = 3000;

/// Moves the conversation of the tab `id` to the account `target_profile_id` and gives the
/// session it continues in. `auto`: the app does it by itself, because a usage limit cut the
/// conversation off. Between ending and starting again the tab is marked as moving, so that it
/// is not restarted or moved a second time; the mark goes again when the move fails.
pub async fn transfer_session(state: &AppState, id: &str, target_profile_id: &str, auto: bool) -> AppResult<SessionInfo> {
    if state.sessions.get(id).is_some_and(|info| info.moving) {
        return Err(AppError::refused("This conversation is already being moved."));
    }
    state.sessions.set_moving(id, true);
    let moved = move_conversation(state, id, target_profile_id, auto).await;
    // the tab that arrived over there has another id; this one is only still there when the move failed
    state.sessions.set_moving(id, false);
    moved
}

/// A usage limit cut the last turn off.
fn cut_by_limit(turn: Option<&Turn>) -> bool {
    matches!(turn, Some(Turn { kind: TurnKind::Failed { limit: true, .. }, .. }))
}

async fn move_conversation(state: &AppState, id: &str, target_profile_id: &str, auto: bool) -> AppResult<SessionInfo> {
    let info = state.sessions.get(id).filter(|i| i.kind == SessionKind::Claude).ok_or_else(|| AppError::refused("Session not found"))?;
    let (Some(source), Some(target)) = (state.profiles.get(&info.profile_id), state.profiles.get(target_profile_id)) else {
        return Err(AppError::refused("Profile not found"));
    };
    if source.id == target.id {
        return Err(AppError::refused("The conversation is already on that account."));
    }
    // a session that was started in a special way (a workflow step) stays that way over there
    let request = SessionRequest {
        profile_id: target.id.clone(),
        cwd: info.cwd.clone(),
        kind: SessionKind::Claude,
        resume: None,
        name: info.name.clone(),
        launch: state.sessions.launch_of(id),
        prompt: None,
    };
    let paths = state.profiles.paths();
    let conversation = info.claude_session_id.clone().filter(|c| !c.is_empty());
    let plan = conversation.as_deref().and_then(|conversation| {
        plan_transfer(&paths.config_dir_of(&source), &paths.config_dir_of(&target), conversation, state.sessions.transcript_of(id).as_deref())
    });
    // nothing was said yet: the tab simply starts over on the other account
    let (Some(conversation), Some(plan)) = (conversation, plan) else {
        return replace_session(state, id, &request);
    };
    if !can_replace(&plan).map_err(|e| AppError::io(e.to_string()))? {
        return Err(AppError::refused(format!(
            "\"{}\" already has this conversation with a different continuation. Open it there with \"From history\" instead.",
            target.name
        )));
    }
    // a usage limit cut the last turn off: Claude is asked to go on with it, which the .cmd shim cannot pass on
    let cut = cut_by_limit(info.turn.as_ref());
    let pick_up = cut && state.claude_path().is_some_and(|claude| !is_shim(&claude));
    let limit = used_up(state.usage.usage_of(&source.id).as_ref(), now_ms(), info.model.as_deref())
        .map_or_else(|| "usage limit".to_owned(), |u| u.label);
    // the last lines are only on disk once the process is gone
    state.sessions.end(id, END_BEFORE_COPY_MS).await;
    tauri::async_runtime::spawn_blocking(move || copy_conversation(&plan))
        .await
        .map_err(|e| AppError::io(e.to_string()))?
        .map_err(|e| AppError::io(e.to_string()))?;
    let moved = replace_session(
        state,
        id,
        &SessionRequest {
            resume: Some(SessionResume::Conversation(ResumeConversation { id: conversation })),
            prompt: pick_up.then(|| PICK_UP_MESSAGE.to_owned()),
            ..request
        },
    )?;
    if cut {
        state.sessions.set_arrived(&moved.id, Arrival { from: source.id, at: now_ms(), limit, picked_up: pick_up, auto });
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use mc_core::types::StopError;

    use super::*;

    #[test]
    fn knows_a_turn_that_a_usage_limit_cut_off() {
        let failed = |limit| Some(Turn { at: 1, kind: TurnKind::Failed { error: StopError::RateLimit, text: None, limit, wait: None } });
        assert!(cut_by_limit(failed(true).as_ref()));
        assert!(!cut_by_limit(failed(false).as_ref()));
        assert!(!cut_by_limit(Some(&Turn { at: 1, kind: TurnKind::Working })));
        assert!(!cut_by_limit(None));
    }
}
