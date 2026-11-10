//! The main window: its first appearance, bringing it to the front, and what its focus and its
//! closing set off (`createWindow` and `bringToFront` of `src/main/index.ts`). Closing it hides it
//! while there is a tray icon, and quits (after asking) when there is none.

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{AppHandle, Manager, UserAttentionType, WebviewWindow, Window, WindowEvent};

use crate::app::open_folder::flush_pending;
use crate::app::quit::{hide_to_tray, request_quit};
use crate::app::{now_ms, recheck_claude, refresh_all_profiles, state as app_state, CLAUDE_RECHECK_MS, STATUS_REFRESH_MS};
use crate::state::{is, AppState};

pub const MAIN: &str = "main";
/// A window that the renderer never reports ready for is shown after this long anyway.
pub const SHOW_FALLBACK: Duration = Duration::from_secs(10);

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN)
}

/// Whether the main window has the keyboard focus.
pub fn is_focused(app: &AppHandle) -> bool {
    main_window(app).is_some_and(|w| w.is_focused().unwrap_or(false))
}

/// Electron's `flashFrame(true)`: the taskbar button asks for attention until the window is focused.
pub fn flash(app: &AppHandle) {
    if let Some(win) = main_window(app) {
        let _ = win.request_user_attention(Some(UserAttentionType::Informational));
    }
}

/// Shows the window. Its first appearance lets the restored tabs and the queued folders through.
pub fn show(state: &AppState) {
    if let Some(win) = main_window(&state.app) {
        if let Err(e) = win.show() {
            log::warn!("showing the window: {e}");
        }
    }
    if !state.flags.window_shown.swap(true, Ordering::SeqCst) {
        flush_pending(state);
    }
}

/// The renderer reported ready: the window appears the first time, unless the app was started
/// hidden (`--hidden`) with a tray icon to bring it back.
pub fn first_show(state: &AppState) {
    if !is(&state.flags.window_shown) && !is(&state.flags.start_hidden) {
        show(state);
    }
}

/// `bringToFront`: shown, restored, in front and focused. Windows may refuse the focus change;
/// then at least the taskbar button flashes.
pub fn bring_to_front(state: &AppState) {
    let Some(win) = main_window(&state.app) else { return };
    if win.is_minimized().unwrap_or(false) {
        let _ = win.unminimize();
    }
    show(state);
    let _ = win.set_focus();
    let app = state.app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(win) = main_window(&app) {
            if !win.is_focused().unwrap_or(true) {
                let _ = win.request_user_attention(Some(UserAttentionType::Informational));
            }
        }
    });
}

/// The window events of the main window.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    let app = window.app_handle();
    let Some(state) = app_state(app) else { return };
    match event {
        WindowEvent::Focused(true) => focused(&state),
        WindowEvent::CloseRequested { api, .. } => {
            // the app is on its way out
            if is(&state.flags.quitting) {
                return;
            }
            api.prevent_close();
            if crate::tray::exists(app) {
                // the sessions keep running; the tray icon brings the window back
                hide_to_tray(&state);
            } else {
                request_quit(app);
            }
        }
        _ => {}
    }
}

/// The window came to the front: the taskbar stops flashing, the sync badges are read again
/// (they depend on files that change outside the app), the account statuses once every 30
/// minutes, and usage numbers older than 5 minutes.
fn focused(state: &AppState) {
    if let Some(win) = main_window(&state.app) {
        let _ = win.request_user_attention(None);
    }
    // marked at once, so that a second focus right after does not read them again
    let last = state.last_status_refresh.load(Ordering::SeqCst);
    let now = now_ms();
    let refresh = now - last > STATUS_REFRESH_MS
        && state.last_status_refresh.compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst).is_ok();
    // Claude Code updates itself in the background: the version in the footer follows, every 10 min
    let last_check = state.last_claude_check.load(Ordering::SeqCst);
    let recheck = now - last_check > CLAUDE_RECHECK_MS
        && state.last_claude_check.compare_exchange(last_check, now, Ordering::SeqCst, Ordering::SeqCst).is_ok();
    let app = state.app.clone();
    tauri::async_runtime::spawn(async move {
        // the badges hash files: off the window's thread
        let badges = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            if let Some(state) = app_state(&badges) {
                state.profiles.emit_changed();
            }
        })
        .await;
        if recheck {
            if let Some(state) = app_state(&app) {
                recheck_claude(&state).await;
            }
        }
        if refresh {
            if let Some(state) = app_state(&app) {
                refresh_all_profiles(&state).await;
            }
        }
    });
    state.usage.window_focused();
}
