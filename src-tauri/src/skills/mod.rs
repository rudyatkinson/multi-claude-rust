//! Skills: roots, scan, read/save, create, rename, duplicate, transfer, delete, archives, Ask Claude (`src/main/skills/*`).
//! Placeholder: filled in by its Phase 3 package. Keep `SkillService::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct SkillService {
    _app: tauri::AppHandle,
    _root: std::path::PathBuf,
}

impl SkillService {
    pub fn new(app: tauri::AppHandle, root: std::path::PathBuf) -> Self {
        SkillService { _app: app, _root: root }
    }
}
