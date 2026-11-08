//! Event names sent to the renderer (`listen(name)` on the TS side). They are the Electron
//! event channels verbatim, so `src/shared/ipc.ts` in the renderer keeps its `EventPayloads` map.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub use mc_core::types::{Notice, NoticeLevel};

pub const PTY_DATA: &str = "pty:data";
pub const SESSIONS_CHANGED: &str = "sessions:changed";
pub const PROFILES_CHANGED: &str = "profiles:changed";
pub const USAGE_CHANGED: &str = "usage:changed";
pub const APP_CHANGED: &str = "app:changed";
pub const APP_NOTICE: &str = "app:notice";
pub const APP_OPEN_FOLDER: &str = "app:openFolder";
pub const SESSIONS_ACTIVATE: &str = "sessions:activate";
pub const UI_NEW_SESSION: &str = "ui:newSession";
pub const WORKFLOWS_CHANGED: &str = "workflows:changed";
pub const RUNS_CHANGED: &str = "runs:changed";

/// Sends an event to the main window. Failures are logged, never propagated: the renderer may be
/// reloading, and no backend operation depends on a delivered event.
pub fn emit<T: Serialize + Clone>(app: &AppHandle, name: &str, payload: T) {
    if let Err(e) = app.emit_to("main", name, payload) {
        log::warn!("emit {name}: {e}");
    }
}

/// Sends `app:notice`: a line the window shows as a toast.
pub fn notice(app: &AppHandle, level: NoticeLevel, message: impl Into<String>) {
    emit(app, APP_NOTICE, Notice { level, message: message.into() });
}
