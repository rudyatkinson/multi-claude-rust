//! Notifications when a session stops and waits (`src/main/sessionAlerts.ts`).
//! Placeholder: filled in by its Phase 1 package. Keep `SessionAlerts::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct SessionAlerts {
    _app: tauri::AppHandle,
}

impl SessionAlerts {
    pub fn new(app: tauri::AppHandle) -> Self {
        SessionAlerts { _app: app }
    }
}
