//! CLAUDE.md and rules per account and project, the load-order view (`src/main/instructions/*`).
//! Placeholder: filled in by its Phase 3 package. Keep `InstructionService::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct InstructionService {
    _app: tauri::AppHandle,
    _root: std::path::PathBuf,
}

impl InstructionService {
    pub fn new(app: tauri::AppHandle, root: std::path::PathBuf) -> Self {
        InstructionService { _app: app, _root: root }
    }
}
