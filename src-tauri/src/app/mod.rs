//! The app around its services: what `src/main/index.ts` of the Electron version did besides
//! creating them. [`boot`] starts the app and ends it, [`wiring`] connects the services to each
//! other, [`launch`] starts sessions, [`open_folder`] delivers folders and restores the last tabs,
//! [`window`] shows the window and follows its focus, [`notify`] shows system notifications.
//!
//! Everything here reaches the services through the managed [`AppState`]; handlers set on the
//! services hold an `AppHandle` and look the state up when they run.

pub mod boot;
pub mod launch;
pub mod notify;
pub mod open_folder;
pub mod window;
pub mod wiring;

use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use mc_core::turn::next_turn;
use mc_core::types::{AppChanged, SessionKind, TurnEvent};
use tauri::{AppHandle, Manager};

use crate::events::{self, NoticeLevel};
use crate::state::{lock, AppState};

/// How often at most the account statuses are read again when the window comes to the front.
pub const STATUS_REFRESH_MS: i64 = 30 * 60 * 1000;

/// `Date.now()`.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The managed state, once setup has put it there.
pub fn state(app: &AppHandle) -> Option<tauri::State<'_, AppState>> {
    app.try_state::<AppState>()
}

/// Sends `app:notice`.
pub fn notice(state: &AppState, level: NoticeLevel, message: impl Into<String>) {
    events::notice(&state.app, level, message);
}

/// Sends `app:changed`: the settings, the claude binary, the recent folders and the folder pins.
pub fn emit_app_changed(state: &AppState) {
    let claude = state.claude_info();
    let payload = state.profiles.config().with(|c| AppChanged {
        settings: c.settings.clone(),
        claude,
        recent_folders: c.recent_folders.clone(),
        folder_accounts: c.folder_accounts.clone(),
    });
    events::emit(&state.app, events::APP_CHANGED, payload);
}

/// `rememberFolder`: the folder goes to the top of the recent folders.
pub fn remember_folder(state: &AppState, cwd: &str) {
    if let Err(e) = state.profiles.config().remember_folder(cwd) {
        log::warn!("recent folders: {e}");
    }
    emit_app_changed(state);
}

/// `locateClaude`: looks for claude (the path from the settings first), tells the window, and
/// rewrites the launchers for it.
pub async fn locate_claude(state: &AppState) {
    let preferred = state.profiles.config().with(|c| c.settings.claude_path.clone());
    let info = crate::claude_binary::find_claude(preferred.as_deref()).await;
    match (&info.path, &info.error) {
        (Some(path), _) => log::info!("claude: {path} ({})", info.version.as_deref().unwrap_or("unknown version")),
        (None, error) => log::warn!("claude not found: {}", error.as_deref().unwrap_or("no reason given")),
    }
    *state.claude.write().unwrap_or_else(std::sync::PoisonError::into_inner) = info;
    emit_app_changed(state);
    state.profiles.refresh_launchers(state.claude_path().as_deref());
}

/// Reads every account's status (`profiles.refreshAll`) and notes when.
pub async fn refresh_all_profiles(state: &AppState) {
    state.last_status_refresh.store(now_ms(), Ordering::SeqCst);
    for warning in state.profiles.refresh_all().await {
        log::info!("profiles: {warning}");
    }
}

/// `noteTurn`: moves a Claude session's turn along with what its hooks report and with what is
/// typed into its tab. The auto switch hears of typing and of a turn that a usage limit cut off.
///
/// The alerts hear of the change once the next turn change may go ahead: telling the user asks
/// the window whether it is in front, which waits for the window's thread, and that thread may
/// itself be waiting here (typing comes from there).
pub fn note_turn(state: &AppState, session_id: &str, event: &TurnEvent) {
    let changed = {
        let _one_at_a_time = lock(&state.turn_lock);
        let Some(info) = state.sessions.get(session_id) else { return };
        if info.kind != SessionKind::Claude {
            return;
        }
        let next = next_turn(info.turn.as_ref(), event, now_ms());
        if let Some(next) = &next {
            state.sessions.set_turn(session_id, Some(next.clone()));
        }
        // an event that changes nothing leaves the turn as it was
        let after = next.as_ref().or(info.turn.as_ref());
        crate::auto_switch::note_turn(state, session_id, event, info.turn.as_ref(), after);
        next.is_some()
    };
    if changed {
        if let Some(info) = state.sessions.get(session_id) {
            state.alerts.turn_changed(&info);
        }
    }
}
