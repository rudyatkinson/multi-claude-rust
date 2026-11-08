//! `config.json` with atomic writes (`src/main/configStore.ts`).
//! Placeholder: filled in by its Phase 1 package. Keep `ConfigStore::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct ConfigStore {
    _root: std::path::PathBuf,
}

impl ConfigStore {
    pub fn new(root: std::path::PathBuf) -> Self {
        ConfigStore { _root: root }
    }
}
