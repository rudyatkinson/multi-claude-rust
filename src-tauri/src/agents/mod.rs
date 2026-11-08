//! Subagent files per account and project (`src/main/agents/*`).
//! Placeholder: filled in by its Phase 3 package. Keep `AgentService::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct AgentService {
    _app: tauri::AppHandle,
    _root: std::path::PathBuf,
}

impl AgentService {
    pub fn new(app: tauri::AppHandle, root: std::path::PathBuf) -> Self {
        AgentService { _app: app, _root: root }
    }
}
