//! CLAUDE.md and rules per account and project, and the load-order view
//! (`src/main/instructions/store.ts`, and its wiring in `src/main/index.ts`).
//!
//! A file is always named by its scope and its place within it; the place has to be one of the
//! fixed files or a rule the scope really lists, so nothing that comes from the window is ever
//! used as a path as it stands.
//!
//! What the service needs from the rest of the app comes through [`InstructionDeps`]: the app's
//! own ([`InstructionService::new`]) reads the accounts, the sessions and `config.json` from
//! `AppState` at the time of each call. One hook is for the integration to set:
//! [`InstructionService::set_memory_dir`], the memory folder a session of an account reads in a
//! folder (the main account's for an account that shares memory). Without it the account's own
//! `projects/<project>/memory` is shown.

mod scan;
#[cfg(test)]
mod tests;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use mc_core::error::{AppError, AppResult};
use mc_core::folders::folder_key;
use mc_core::instructions::{fixed_files, rule_name_problem, rule_title, rules_dir, starter_text};
use mc_core::jsstr::longer16;
use mc_core::text_file::{create_text, read_text, save_text, TextRead, GONE};
use mc_core::types::{
    InstructionDetail, InstructionFile, InstructionKind, InstructionRef, InstructionSaveResult, InstructionScope, InstructionScopeKind,
    InstructionTarget, LoadStep, SaveOutcome, SessionKind, SKILL_LIMITS,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

pub use scan::{find_rules, find_scopes, list_files, load_order, InstructionAccount, LoadInputs, ScopeSpec};

use crate::library::trash::{delete_item, system_trash, TrashFn};
use crate::state::AppState;

const WINDOWS: bool = cfg!(windows);

/// The project folders the Library keeps in `settings.skillFolders`, at most.
const PINNED_MAX: usize = 30;
/// How long the name of a project's folder under `projects` may be; a longer one carries a hash
/// that is not reproduced here.
const PROJECT_KEY_MAX: usize = 200;

const SCOPE_GONE: &str = "That folder is no longer in the list.";

/// What the service needs from the rest of the app (`InstructionDeps` of the Electron version).
pub trait InstructionDeps: Send + Sync {
    fn accounts(&self) -> Vec<InstructionAccount>;
    /// The project folders in sight.
    fn folders(&self) -> Vec<String>;
    fn home_dir(&self) -> PathBuf;
    /// The organisation's own CLAUDE.md, where this system keeps one.
    fn managed_file(&self) -> Option<PathBuf>;
    /// The folder above which no project files are looked for; the top of the drive when None.
    fn top(&self) -> Option<PathBuf> {
        None
    }
    /// The memory folder a session of the account reads in a folder, when it has one.
    fn memory_dir(&self, profile_id: &str, cwd: &Path) -> Option<PathBuf>;
    /// Moves a file to the system's bin. Fails when there is none for that drive.
    fn trash(&self, file: &Path) -> io::Result<()>;
    /// A file of an account was written: what is shown about syncing may have changed.
    fn account_changed(&self);
}

/// The memory folder a session of an account (by id) reads in a folder.
pub type MemoryDirFn = Arc<dyn Fn(&str, &Path) -> Option<PathBuf> + Send + Sync>;

/// Everything the instructions editor does on disk.
pub struct InstructionService {
    /// For the file manager and the folder dialog; None in tests.
    app: Option<AppHandle>,
    deps: Arc<dyn InstructionDeps>,
    memory_dir: RwLock<Option<MemoryDirFn>>,
}

impl InstructionService {
    /// The app's service. `root` (the app's own folder) holds nothing of it: the files are the
    /// accounts' and the projects'.
    pub fn new(app: AppHandle, root: PathBuf) -> Self {
        let _ = root;
        InstructionService { deps: Arc::new(AppDeps { app: app.clone() }), app: Some(app), memory_dir: RwLock::new(None) }
    }

    /// A service on other deps, without the file manager or the folder dialog (tests).
    pub fn with_deps(deps: Arc<dyn InstructionDeps>) -> Self {
        InstructionService { app: None, deps, memory_dir: RwLock::new(None) }
    }

    /// Replaces how the memory folder of a session is found (`memoryIndex` of the Electron deps,
    /// as a folder): the integration points an account that shares memory at the main
    /// account's folder here. Its answer is final: None shows no MEMORY.md.
    pub fn set_memory_dir(&self, f: impl Fn(&str, &Path) -> Option<PathBuf> + Send + Sync + 'static) {
        *self.memory_dir.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(f));
    }

    fn specs(&self) -> Vec<ScopeSpec> {
        find_scopes(&self.deps.accounts(), &self.deps.folders())
    }

    fn scope(&self, scope_id: &str) -> AppResult<ScopeSpec> {
        self.specs().into_iter().find(|s| s.scope.id == scope_id).ok_or_else(|| AppError::refused(SCOPE_GONE))
    }

    /// The file of a scope, once the scope's own listing has it.
    fn locate(&self, r: &InstructionRef) -> AppResult<(ScopeSpec, PathBuf)> {
        let scope = self.scope(&r.scope_id)?;
        let dir = PathBuf::from(&scope.scope.dir);
        if !list_files(scope.scope.kind, &dir).iter().any(|f| f.file == r.file && f.exists) {
            return Err(AppError::refused(GONE));
        }
        let abs = scan::within(&dir, &r.file);
        Ok((scope, abs))
    }

    fn detail(r: &InstructionRef, abs: &Path, on_disk: TextRead) -> InstructionDetail {
        InstructionDetail {
            scope_id: r.scope_id.clone(),
            file: r.file.clone(),
            text: on_disk.text,
            hash: on_disk.hash,
            path: abs.to_string_lossy().into_owned(),
            read_only: on_disk.read_only,
        }
    }

    /// The accounts, then the project folders in sight (without their resolved paths).
    pub fn scopes(&self) -> Vec<InstructionScope> {
        self.specs().into_iter().map(|s| s.scope).collect()
    }

    pub fn files(&self, scope_id: &str) -> AppResult<Vec<InstructionFile>> {
        let scope = self.scope(scope_id)?;
        Ok(list_files(scope.scope.kind, Path::new(&scope.scope.dir)))
    }

    /// The folder of a scope, or one of its files, for the file manager.
    pub fn path_of(&self, scope_id: &str, file: Option<&str>) -> AppResult<PathBuf> {
        match file {
            None => Ok(PathBuf::from(self.scope(scope_id)?.scope.dir)),
            Some(file) => Ok(self.locate(&InstructionRef { scope_id: scope_id.to_owned(), file: file.to_owned() })?.1),
        }
    }

    pub fn read(&self, r: &InstructionRef) -> AppResult<InstructionDetail> {
        let (_, abs) = self.locate(r)?;
        Ok(Self::detail(r, &abs, read_text(&abs, SKILL_LIMITS.file_bytes)?))
    }

    /// Writes a file against the state the editor started from; see `save_text`. A file of an
    /// account tells the accounts' views.
    pub fn save(&self, r: &InstructionRef, text: &str, base_hash: Option<&str>) -> AppResult<InstructionSaveResult> {
        let (scope, abs) = self.locate(r)?;
        match save_text(&abs, text, base_hash)? {
            SaveOutcome::Conflict(on_disk) => Ok(SaveOutcome::Conflict(Self::detail(r, &abs, on_disk))),
            SaveOutcome::Saved(on_disk) => {
                if scope.scope.kind == InstructionScopeKind::Account {
                    self.deps.account_changed();
                }
                Ok(SaveOutcome::Saved(Self::detail(r, &abs, on_disk)))
            }
        }
    }

    /// Makes one of the fixed files of a scope that is not there yet, or a rule: for a rule its
    /// file is `<name>.md` in the scope's rules folder.
    pub fn create(&self, scope_id: &str, target: &InstructionTarget) -> AppResult<InstructionRef> {
        let scope = self.scope(scope_id)?;
        let kind = scope.scope.kind;
        let dir = PathBuf::from(&scope.scope.dir);
        let file = match target {
            InstructionTarget::Rule(name) => {
                if let Some(problem) = rule_name_problem(name) {
                    return Err(AppError::invalid(problem));
                }
                let file = format!("{}/{name}.md", rules_dir(kind));
                create_text(&scan::within(&dir, &file), starter_text(InstructionKind::Rule, kind, &rule_title(name)))?;
                file
            }
            InstructionTarget::File(name) => {
                let fixed = fixed_files(kind).iter().find(|f| f.file == name.as_str()).ok_or_else(|| AppError::refused("That file cannot be made here."))?;
                let title = match kind {
                    InstructionScopeKind::Project => dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    InstructionScopeKind::Account => String::new(),
                };
                create_text(&scan::within(&dir, fixed.file), starter_text(fixed.kind, kind, &title))?;
                fixed.file.to_owned()
            }
        };
        if kind == InstructionScopeKind::Account {
            self.deps.account_changed();
        }
        Ok(InstructionRef { scope_id: scope_id.to_owned(), file })
    }

    /// Puts an instruction file into the bin; a link is only taken away. A file that cannot go to
    /// the bin is never deleted for good instead.
    pub fn delete(&self, r: &InstructionRef) -> AppResult<()> {
        let (scope, abs) = self.locate(r)?;
        let deps = Arc::clone(&self.deps);
        let trash: TrashFn = Arc::new(move |file: &Path| deps.trash(file));
        delete_item(&trash, &abs, "file")?;
        if scope.scope.kind == InstructionScopeKind::Account {
            self.deps.account_changed();
        }
        Ok(())
    }

    /// What a session of an account reads when it starts in a project folder, in order.
    pub fn view(&self, profile_id: &str, scope_id: &str) -> AppResult<Vec<LoadStep>> {
        let account = self.deps.accounts().into_iter().find(|a| a.id == profile_id).ok_or_else(|| AppError::refused("Profile not found"))?;
        let scope = self.scope(scope_id)?;
        if scope.scope.kind != InstructionScopeKind::Project {
            return Err(AppError::refused("Choose a project folder to see what a session reads there."));
        }
        let cwd = PathBuf::from(&scope.scope.dir);
        let hook = self.memory_dir.read().unwrap_or_else(PoisonError::into_inner).clone();
        let memory_dir = match hook {
            Some(hook) => hook(profile_id, &cwd),
            None => self.deps.memory_dir(profile_id, &cwd),
        };
        Ok(load_order(&LoadInputs {
            config_dir: account.config_dir,
            cwd,
            home_dir: self.deps.home_dir(),
            managed_file: self.deps.managed_file(),
            memory_dir,
            top: self.deps.top(),
        }))
    }

    fn app(&self) -> AppResult<&AppHandle> {
        self.app.as_ref().ok_or_else(|| AppError::refused("Not available here."))
    }

    /// Shows a file of a scope in the file manager, or opens the scope's folder.
    pub fn reveal(&self, scope_id: &str, file: Option<&str>) -> AppResult<()> {
        let target = self.path_of(scope_id, file)?;
        let opener = self.app()?.opener();
        let shown = if file.is_some() { opener.reveal_item_in_dir(&target) } else { opener.open_path(target.to_string_lossy(), None::<&str>) };
        shown.map_err(|e| AppError::io(e.to_string()))
    }

    /// Asks for a project folder and keeps it in the Library's list; the id of its instruction
    /// files, None when the dialog was cancelled or the folder is an account's own. Blocks until
    /// the dialog closes: call it off the main thread.
    pub fn add_folder(&self) -> AppResult<Option<String>> {
        let Some(folder) = pick_library_folder(self.app()?)? else { return Ok(None) };
        let key = folder_key(&folder, WINDOWS);
        Ok(self.scopes().into_iter().find(|s| s.kind == InstructionScopeKind::Project && folder_key(&s.dir, WINDOWS) == key).map(|s| s.id))
    }
}

/// `pickLibraryFolder` of `src/main/index.ts`, kept here so that this package stands on its own:
/// asks for a project folder and pins it into `settings.skillFolders` (the last 30).
fn pick_library_folder(app: &AppHandle) -> AppResult<Option<String>> {
    let Some(win) = crate::app::window::main_window(app) else { return Ok(None) };
    let picked = app.dialog().file().set_title("Choose a project folder").set_parent(&win).blocking_pick_folder();
    let Some(folder) = picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()).filter(|f| !f.is_empty()) else {
        return Ok(None);
    };
    let Some(state) = app.try_state::<AppState>() else { return Ok(Some(folder)) };
    let key = folder_key(&folder, WINDOWS);
    let config = state.profiles.config();
    let pinned = |c: &mc_core::types::AppConfig| c.settings.skill_folders.iter().flatten().any(|f| folder_key(f, WINDOWS) == key);
    if !config.with(pinned) {
        config.update(|c| {
            let mut next = c.settings.skill_folders.clone().unwrap_or_default();
            next.push(folder.clone());
            let extra = next.len().saturating_sub(PINNED_MAX);
            next.drain(..extra);
            c.settings.skill_folders = Some(next);
        })?;
    }
    Ok(Some(folder))
}

// ---------- the app's deps ----------

/// Where an organisation puts a CLAUDE.md for every user of the machine.
pub fn managed_claude_md() -> PathBuf {
    if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles").filter(|v| !v.is_empty()).map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
        program_files.join("ClaudeCode").join("CLAUDE.md")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode/CLAUDE.md")
    } else {
        PathBuf::from("/etc/claude-code/CLAUDE.md")
    }
}

/// `sanitizePath`: every UTF-16 unit that is not an ASCII letter or digit becomes a hyphen.
fn sanitize_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.extend(std::iter::repeat_n('-', c.len_utf16()));
        }
    }
    out
}

/// `sharedMemoryDir(configDir, cwd)` of `src/main/memory/share.ts` for an account's own folder,
/// kept here so that this package stands on its own: `<config>/projects/<project>/memory`, the
/// project named after the top of the git repository (or the folder), spelled as on disk when it
/// is there. None for a folder that is not absolute or whose name would be too long.
pub fn own_memory_dir(config_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    if !cwd.is_absolute() {
        return None;
    }
    let root = scan::git_root_of(cwd).unwrap_or_else(|| scan::resolve(cwd));
    let key = sanitize_path(&root.to_string_lossy());
    if longer16(&key, PROJECT_KEY_MAX) {
        return None;
    }
    let projects = config_dir.join("projects");
    let lower = key.to_lowercase();
    let on_disk = fs::read_dir(&projects)
        .ok()
        .and_then(|read| read.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.to_lowercase() == lower));
    Some(projects.join(on_disk.unwrap_or(key)).join("memory"))
}


/// The deps of the app: everything read from `AppState` at the time of the call.
struct AppDeps {
    app: AppHandle,
}

impl AppDeps {
    fn state(&self) -> Option<tauri::State<'_, AppState>> {
        self.app.try_state::<AppState>()
    }
}

impl InstructionDeps for AppDeps {
    fn accounts(&self) -> Vec<InstructionAccount> {
        let Some(state) = self.state() else { return Vec::new() };
        let paths = state.profiles.paths();
        state
            .profiles
            .list()
            .into_iter()
            .map(|p| InstructionAccount {
                config_dir: paths.config_dir_of(&p),
                synced_claude_md: !p.is_default && p.share.files.iter().any(|f| f == "CLAUDE.md"),
                id: p.id,
            })
            .collect()
    }

    /// `knownFolders`: where sessions run, what was opened lately, what was added to the Library.
    fn folders(&self) -> Vec<String> {
        let Some(state) = self.state() else { return Vec::new() };
        let mut folders: Vec<String> = state.sessions.list().into_iter().filter(|s| s.kind == SessionKind::Claude).map(|s| s.cwd).collect();
        state.profiles.config().with(|c| {
            folders.extend(c.recent_folders.iter().cloned());
            folders.extend(c.settings.skill_folders.iter().flatten().cloned());
        });
        folders
    }

    fn home_dir(&self) -> PathBuf {
        match self.state() {
            Some(state) => state.home_dir().to_path_buf(),
            None => dirs::home_dir().unwrap_or_default(),
        }
    }

    fn managed_file(&self) -> Option<PathBuf> {
        Some(managed_claude_md())
    }

    fn memory_dir(&self, profile_id: &str, cwd: &Path) -> Option<PathBuf> {
        let state = self.state()?;
        let config_dir = state.config_dir_of(profile_id)?;
        own_memory_dir(&config_dir, cwd)
    }

    fn trash(&self, file: &Path) -> io::Result<()> {
        system_trash(file)
    }

    fn account_changed(&self) {
        if let Some(state) = self.state() {
            state.profiles.emit_changed();
        }
    }
}
