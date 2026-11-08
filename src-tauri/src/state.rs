//! Process-wide state handed to commands through `tauri::State<AppState>`. Services keep their own
//! interior mutability (Mutex/RwLock inside), so the fields here are plain. Cross-service wiring
//! (who calls whom when a session exits, a hook arrives, ...) is done in `lib.rs` at integration.

use std::path::PathBuf;
use std::sync::RwLock;

use mc_core::types::ClaudeBinaryInfo;
use tauri::AppHandle;

pub struct AppState {
    pub app: AppHandle,
    /// `~/.multi-claude`, or `MULTI_CLAUDE_HOME` when set (an isolated instance for development).
    pub root: PathBuf,
    /// Where `claude` was found, its version, or why not.
    pub claude: RwLock<ClaudeBinaryInfo>,
    pub config: crate::config_store::ConfigStore,
    pub profiles: crate::profiles::ProfileService,
    pub sessions: crate::sessions::SessionManager,
    pub hooks: crate::hook_server::HookServer,
    pub alerts: crate::session_alerts::SessionAlerts,
    pub usage: crate::usage::UsageTracker,
    pub tabs: crate::tab_store::TabStore,
}

impl AppState {
    pub fn new(app: AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let root = match std::env::var_os("MULTI_CLAUDE_HOME") {
            Some(home) if !home.is_empty() => PathBuf::from(home),
            _ => dirs::home_dir().ok_or("no home directory")?.join(".multi-claude"),
        };
        std::fs::create_dir_all(&root)?;
        Ok(AppState {
            claude: RwLock::new(ClaudeBinaryInfo::default()),
            config: crate::config_store::ConfigStore::new(root.clone()),
            profiles: crate::profiles::ProfileService::new(app.clone(), root.clone()),
            sessions: crate::sessions::SessionManager::new(app.clone(), root.clone()),
            hooks: crate::hook_server::HookServer::new(app.clone()),
            alerts: crate::session_alerts::SessionAlerts::new(app.clone()),
            usage: crate::usage::UsageTracker::new(app.clone(), root.clone()),
            tabs: crate::tab_store::TabStore::new(root.clone()),
            app,
            root,
        })
    }
}
