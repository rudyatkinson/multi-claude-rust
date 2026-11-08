//! Process-wide state handed to commands through `tauri::State<AppState>`. Each feature package
//! adds its service here behind a lock; Phase 1 integration fills in the wiring between them.

use std::path::PathBuf;

use tauri::AppHandle;

pub struct AppState {
    pub app: AppHandle,
    /// `~/.multi-claude`, or `MULTI_CLAUDE_HOME` when set (an isolated instance for development).
    pub root: PathBuf,
}

impl AppState {
    pub fn new(app: AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let root = match std::env::var_os("MULTI_CLAUDE_HOME") {
            Some(home) if !home.is_empty() => PathBuf::from(home),
            _ => dirs::home_dir().ok_or("no home directory")?.join(".multi-claude"),
        };
        std::fs::create_dir_all(&root)?;
        Ok(AppState { app, root })
    }
}
