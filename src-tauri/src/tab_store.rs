//! `tabs.json`: saving and restoring tabs and their arrangement (`src/main/tabStore.ts`).
//! Placeholder: filled in by its Phase 1 package. Keep `TabStore::new` with this signature so
//! `state.rs` keeps compiling; if the service needs more, say so in the report.

pub struct TabStore {
    _root: std::path::PathBuf,
}

impl TabStore {
    pub fn new(root: std::path::PathBuf) -> Self {
        TabStore { _root: root }
    }
}
