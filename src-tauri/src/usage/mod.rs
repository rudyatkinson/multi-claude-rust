//! Usage numbers: `/usage` probe queue, status-line watcher, alerts, cache (`src/main/usage/*`).
//! Placeholder: filled in by its Phase 1 package. Keep `UsageTracker::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct UsageTracker {
    _app: tauri::AppHandle, _root: std::path::PathBuf,
}

impl UsageTracker {
    pub fn new(app: tauri::AppHandle, root: std::path::PathBuf) -> Self {
        UsageTracker { _app: app, _root: root }
    }
}
