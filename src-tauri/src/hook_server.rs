//! Loopback HTTP server receiving Claude Code hooks (`src/main/workflows/hookServer.ts`).
//! Placeholder: filled in by its Phase 1 package. Keep `HookServer::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct HookServer {
    _app: tauri::AppHandle,
}

impl HookServer {
    pub fn new(app: tauri::AppHandle) -> Self {
        HookServer { _app: app }
    }
}
