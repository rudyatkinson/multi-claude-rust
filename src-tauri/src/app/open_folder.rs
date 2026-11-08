//! What has to wait for both `claude` and the window: the tabs of the last run, then the folders
//! that came from outside (`--open-folder`, a second instance). A window that is still loading
//! would see new sessions out of order with its first state and end up with the wrong tab in front.

use std::sync::atomic::Ordering;

use mc_core::types::{OpenFolder, ResumeConversation, SessionKind, SessionRequest, SessionResume};

use crate::app::launch::plan_session;
use crate::app::notice;
use crate::events::{self, NoticeLevel};
use crate::state::{is, lock, AppState};
use crate::tab_store::{plan_restore_on_disk, restored_layout};

/// `requestOpenFolder`: queues a folder for the window, once it is known to be one.
pub fn request_open_folder(state: &AppState, folder: &str) {
    if !std::fs::metadata(folder).is_ok_and(|m| m.is_dir()) {
        notice(state, NoticeLevel::Warn, format!("Folder not found: {folder}"));
        return;
    }
    lock(&state.pending_open_folders).push(folder.to_owned());
    flush_pending(state);
}

/// `flushPending`: once the renderer listens, claude has been looked for and the window has been
/// shown, reopens the saved tabs (once per run) and then delivers the queued folders.
pub fn flush_pending(state: &AppState) {
    let flags = &state.flags;
    if !(is(&flags.renderer_ready) && is(&flags.boot_ready) && is(&flags.window_shown)) {
        return;
    }
    let _one_at_a_time = lock(&state.flush_lock);
    if !flags.tabs_restored.swap(true, Ordering::SeqCst) {
        restore_tabs(state);
    }
    let folders: Vec<String> = lock(&state.pending_open_folders).drain(..).collect();
    for cwd in folders {
        events::emit(&state.app, events::APP_OPEN_FOLDER, OpenFolder { cwd });
    }
}

/// `restoreTabs`: reopens the tabs of the last run. Each continues its conversation, or starts
/// fresh if it never had one; all are announced at once, with the arrangement they belong in.
fn restore_tabs(state: &AppState) {
    if state.settings().restore_tabs == Some(false) || state.claude_path().is_none() {
        return;
    }
    let plan = plan_restore_on_disk(&state.tabs.load(), |id| state.config_dir_of(id));
    if plan.requests.is_empty() {
        return;
    }
    let started: Vec<Option<String>> = plan
        .requests
        .iter()
        .map(|tab| {
            let request = SessionRequest {
                profile_id: tab.profile_id.clone(),
                cwd: tab.cwd.clone(),
                kind: SessionKind::Claude,
                resume: tab.resume.as_ref().map(|r| SessionResume::Conversation(ResumeConversation { id: r.id.clone() })),
                name: tab.name.clone(),
                launch: None,
                prompt: None,
            };
            match plan_session(state, &request).and_then(|plan| state.sessions.start(plan)) {
                Ok(info) => Some(info.id),
                Err(e) => {
                    log::warn!("a tab of the last run could not start: {e}");
                    None
                }
            }
        })
        .collect();
    let live: Vec<String> = state.sessions.list().into_iter().map(|s| s.id).collect();
    state.tabs.note_layout(restored_layout(&plan, &started, &live));
    // the list that introduces the sessions carries the arrangement
    state.flags.layout_to_send.store(true, Ordering::SeqCst);
    state.sessions.announce();
}
