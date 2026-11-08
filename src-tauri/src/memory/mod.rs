//! Auto memory per account and project, sharing and adopting (`src/main/memory/*`).
//! Placeholder: filled in by its Phase 3 package. Keep `MemoryService::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct MemoryService {
    _app: tauri::AppHandle,
    _root: std::path::PathBuf,
}

impl MemoryService {
    pub fn new(app: tauri::AppHandle, root: std::path::PathBuf) -> Self {
        MemoryService { _app: app, _root: root }
    }
}
