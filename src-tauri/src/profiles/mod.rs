//! Accounts: create, sync, token, status, delete (`src/main/profiles.ts`).
//! Placeholder: filled in by its Phase 1 package. Keep `ProfileService::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct ProfileService {
    _app: tauri::AppHandle, _root: std::path::PathBuf,
}

impl ProfileService {
    pub fn new(app: tauri::AppHandle, root: std::path::PathBuf) -> Self {
        ProfileService { _app: app, _root: root }
    }
}
