//! Process-wide state handed to commands through `tauri::State<AppState>`. Services keep their own
//! interior mutability (Mutex/RwLock inside), so the fields here are plain. Cross-service wiring
//! (who calls whom when a session exits, a hook arrives, ...) is done in `app::wiring`; the
//! module-level variables of the Electron `src/main/index.ts` are the flags below.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use mc_core::open_folder_args::StartupArgs;
use mc_core::types::{AppSettings, ClaudeBinaryInfo};
use tauri::AppHandle;

pub struct AppState {
    pub app: AppHandle,
    /// `~/.multi-claude`, or `MULTI_CLAUDE_HOME` when set (an isolated instance for development).
    pub root: PathBuf,
    /// Where `claude` was found, its version, or why not.
    pub claude: RwLock<ClaudeBinaryInfo>,
    /// The accounts, and through [`crate::profiles::ProfileService::config`] the one store of
    /// `config.json` (settings, recent folders, folder pins).
    pub profiles: crate::profiles::ProfileService,
    pub sessions: crate::sessions::SessionManager,
    pub hooks: crate::hook_server::HookServer,
    pub alerts: crate::session_alerts::SessionAlerts,
    pub usage: crate::usage::UsageTracker,
    pub tabs: crate::tab_store::TabStore,
    /// The designed workflows (`workflows.json`).
    pub workflows: crate::workflows::WorkflowStore,
    /// The runs of workflows, in memory only.
    pub runs: crate::workflows::WorkflowRuns,
    /// Moves a conversation that a usage limit cut off to another account by itself.
    pub auto_switch: crate::auto_switch::AutoSwitch,
    /// The past conversations of the accounts, as last listed.
    pub conversations: Mutex<mc_core::conversations::ConversationIndex>,
    /// "Open in Multi Claude" in File Explorer.
    pub explorer_menu: crate::explorer_menu::ExplorerMenu,
    // Phase 3: the Library services
    pub skills: crate::skills::SkillService,
    pub agents: crate::agents::AgentService,
    pub memory: crate::memory::MemoryService,
    pub instructions: crate::instructions::InstructionService,
    /// The update the last check found, and which versions the window was told about.
    pub updater: crate::updater::Updates,
    /// Windows build number, for the terminal's ConPTY quirks.
    pub os_build: Option<u32>,
    /// What this process was started for: a folder from the Explorer context menu, the login item.
    pub startup: StartupArgs,
    pub flags: Flags,
    /// Folders to open wait here until the renderer listens and claude has been located.
    pub pending_open_folders: Mutex<Vec<String>>,
    /// When the account statuses were last read (ms since the epoch; 0 for never).
    pub last_status_refresh: AtomicI64,
    /// What has unsaved changes in the window's Library, for the quit question.
    pub unsaved: Mutex<Vec<String>>,
    /// Why the hook server could not start; told once the window listens.
    pub hook_problem: Mutex<Option<String>>,
    /// One turn change at a time: hooks arrive on their own thread, typing on the window's.
    pub turn_lock: Mutex<()>,
    /// One `flush_pending` at a time, so that restored tabs always come before opened folders.
    pub flush_lock: Mutex<()>,
}

/// The moments of the app's life that decide what may happen next.
#[derive(Default)]
pub struct Flags {
    /// The renderer has its listeners in place (`app:ready`); reset when the page loads again.
    pub renderer_ready: AtomicBool,
    /// claude has been looked for.
    pub boot_ready: AtomicBool,
    /// The window has been shown once.
    pub window_shown: AtomicBool,
    /// The tabs of the last run were reopened (once per run of the app, not per page load).
    pub tabs_restored: AtomicBool,
    /// The next session list carries the restored arrangement.
    pub layout_to_send: AtomicBool,
    /// The app is shutting down: ending the sessions must not empty the saved tabs.
    pub quitting: AtomicBool,
    /// The window stays hidden at the start: `--hidden`, and a tray icon to bring it back with.
    pub start_hidden: AtomicBool,
    /// The quit question is open; asking again waits for its answer instead of opening another.
    pub asking_to_quit: AtomicBool,
}

/// Reads a flag.
pub fn is(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The app's data folder and whether it is an isolated instance: `MULTI_CLAUDE_HOME` (made
/// absolute) when set, else `~/.multi-claude`.
pub fn app_root() -> Result<(PathBuf, bool), String> {
    match std::env::var_os("MULTI_CLAUDE_HOME") {
        Some(home) if !home.is_empty() => {
            let home = PathBuf::from(home);
            Ok((std::path::absolute(&home).unwrap_or(home), true))
        }
        _ => Ok((dirs::home_dir().ok_or("no home directory")?.join(".multi-claude"), false)),
    }
}

impl AppState {
    pub fn new(app: AppHandle, root: PathBuf, startup: StartupArgs) -> Result<Self, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&root)?;
        let os_build = windows_build();
        Ok(AppState {
            claude: RwLock::new(ClaudeBinaryInfo::default()),
            profiles: crate::profiles::ProfileService::new(app.clone(), root.clone()),
            sessions: crate::sessions::SessionManager::new(app.clone(), root.clone()),
            hooks: crate::hook_server::HookServer::new(app.clone()),
            alerts: crate::session_alerts::SessionAlerts::new(app.clone()),
            usage: crate::usage::UsageTracker::new(app.clone(), root.clone()),
            tabs: crate::tab_store::TabStore::new(root.clone()),
            workflows: crate::workflows::WorkflowStore::new(crate::paths::Paths::new(root.clone()).workflows_file()),
            runs: crate::workflows::WorkflowRuns::new(Arc::new(crate::workflows::AppRuns(app.clone()))),
            auto_switch: crate::auto_switch::AutoSwitch::new(Arc::new(crate::auto_switch::AppSwitch(app.clone()))),
            conversations: Mutex::default(),
            explorer_menu: crate::explorer_menu::ExplorerMenu::for_this_build(os_build),
            skills: crate::skills::SkillService::new(app.clone(), root.clone()),
            agents: crate::agents::AgentService::new(app.clone(), root.clone()),
            memory: crate::memory::MemoryService::new(app.clone(), root.clone()),
            instructions: crate::instructions::InstructionService::new(app.clone(), root.clone()),
            updater: crate::updater::Updates::default(),
            os_build,
            startup,
            flags: Flags::default(),
            pending_open_folders: Mutex::default(),
            last_status_refresh: AtomicI64::new(0),
            unsaved: Mutex::default(),
            hook_problem: Mutex::default(),
            turn_lock: Mutex::default(),
            flush_lock: Mutex::default(),
            app,
            root,
        })
    }

    /// The claude binary, when one was found.
    pub fn claude_path(&self) -> Option<String> {
        self.claude.read().unwrap_or_else(PoisonError::into_inner).path.clone().filter(|p| !p.is_empty())
    }

    pub fn claude_info(&self) -> ClaudeBinaryInfo {
        self.claude.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn settings(&self) -> AppSettings {
        self.profiles.config().settings()
    }

    /// `effectiveConfigDir` of an account by id; None when the account is gone.
    pub fn config_dir_of(&self, profile_id: &str) -> Option<PathBuf> {
        self.profiles.get(profile_id).map(|p| self.profiles.paths().config_dir_of(&p))
    }

    /// The user's home folder.
    pub fn home_dir(&self) -> &Path {
        self.profiles.paths().home()
    }
}

/// The Windows build number (`CurrentBuildNumber`), None elsewhere.
#[cfg(windows)]
fn windows_build() -> Option<u32> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok()?;
    key.get_value::<String, _>("CurrentBuildNumber").ok()?.trim().parse().ok()
}

#[cfg(not(windows))]
fn windows_build() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn reads_the_windows_build() {
        assert!(super::windows_build().is_some_and(|b| b > 7600));
    }
}
