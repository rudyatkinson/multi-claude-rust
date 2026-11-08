//! Pty sessions: spawn, write, resize, kill, replay buffer, `pty:data` events (`src/main/sessions.ts`).
//! Placeholder: filled in by its Phase 1 package. Keep `SessionManager::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct SessionManager {
    _app: tauri::AppHandle, _root: std::path::PathBuf,
}

impl SessionManager {
    pub fn new(app: tauri::AppHandle, root: std::path::PathBuf) -> Self {
        SessionManager { _app: app, _root: root }
    }
}
