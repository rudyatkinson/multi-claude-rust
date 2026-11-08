//! Subagent files per account and project (`src/main/agents/*`).
//!
//! Everything the agent manager does on disk. An agent is always named by its root and its file
//! within it; the file has to be one that a fresh listing of the root really has, so nothing that
//! comes from the window is ever used as a path as it stands. The roots are found anew for every
//! call from [`RootInputs`] ([`root_inputs`] reads them from the app).

mod roots;
mod scan;
#[cfg(test)]
mod tests;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mc_core::agents::{agent_clashes, agent_key, agent_name_problem, agent_stem, new_agent_text, NamedAgent};
use mc_core::error::{AppError, AppResult};
use mc_core::frontmatter::{name_of, parse_doc, set_field};
use mc_core::text_file::{
    busy_message, create_text, decode, encode, is_busy, read_text, retry_busy, save_text, write_file_atomic, TextRead, BUSY_TRIES,
    BUSY_WAIT,
};
use mc_core::types::{
    AgentDetail, AgentListing, AgentMoved, AgentRef, AgentSaveResult, AgentSummary, SaveOutcome, SessionKind, TransferMode, SKILL_LIMITS,
};
use tauri::AppHandle;

use crate::config_store::ConfigStore;
use crate::state::{lock, AppState};

pub use roots::{find_roots, pin_folder, project_root_of, unpin_root, ProfileFolder, RootInputs, RootSpec, PINNED_MAX};
pub use scan::{find_agents, summarize, MAX_FILES};

const GONE: &str = "That agent is no longer there. The list may be out of date.";
const NOT_LISTED: &str = "That folder is no longer in the list.";
const FIX_FIRST: &str = "The frontmatter of this agent cannot be read. Fix it in the Source tab first.";

/// Moves a file to the system's bin. Fails when there is none for that drive.
pub type TrashFn = Arc<dyn Fn(&Path) -> io::Result<()> + Send + Sync>;

/// `shell.trashItem`: the system's bin through the `trash` crate. Windows' reasons come back as OS
/// errors, so that a file in use is told apart (see [`is_busy`]).
///
/// On a thread of its own: on Windows the bin is reached through COM, and the `trash` crate panics
/// on a thread that other code set up for COM another way.
fn trash_file(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    std::thread::spawn(move || trash::delete(&path).map_err(trash_error))
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("The system's bin could not be reached.")))
}

fn trash_error(err: trash::Error) -> io::Error {
    #[cfg(windows)]
    if let trash::Error::Os { code, .. } = &err {
        let code = *code as u32;
        let win32 = match code {
            // COPYENGINE_E_ACCESS_DENIED_SRC, _SHARING_VIOLATION_SRC, _DIR_NOT_EMPTY
            0x8027_0021 => Some(5),
            0x8027_0027 => Some(32),
            0x8027_003D => Some(145),
            // HRESULT_FROM_WIN32
            c if c & 0xFFFF_0000 == 0x8007_0000 => Some(c & 0xFFFF),
            _ => None,
        };
        if let Some(win32) = win32 {
            return io::Error::from_raw_os_error(win32 as i32);
        }
    }
    match err {
        trash::Error::Os { description, .. } | trash::Error::Unknown { description } => io::Error::other(description),
        other => io::Error::other(other.to_string()),
    }
}

/// `libraryRoots('agents')`'s inputs as the app has them now: the accounts, the folders of the
/// Claude sessions and the recent folders, and the folders the user added to the Library.
pub fn root_inputs(state: &AppState) -> RootInputs {
    let paths = state.profiles.paths();
    let profiles =
        state.profiles.list().into_iter().map(|p| ProfileFolder { config_dir: paths.config_dir_of(&p), id: p.id, name: p.name }).collect();
    let mut folders: Vec<String> = state.sessions.list().into_iter().filter(|s| s.kind == SessionKind::Claude).map(|s| s.cwd).collect();
    let (recent, pinned) =
        state.profiles.config().with(|c| (c.recent_folders.clone(), c.settings.skill_folders.clone().unwrap_or_default()));
    folders.extend(recent);
    RootInputs { profiles, folders, pinned }
}

/// The folder part of a file within a root, with its slash: `team/` of `team/qa.md`.
fn folder_of(file: &str) -> &str {
    file.rfind('/').map_or("", |at| &file[..=at])
}

/// The file's own name: `qa.md` of `team/qa.md`.
fn base_of(file: &str) -> &str {
    file.rsplit('/').next().unwrap_or(file)
}

/// `entryNamed`: the entry of a folder that has this name, whatever its letter case.
fn entry_named<'a>(names: &'a [String], name: &str) -> Option<&'a str> {
    let lower = name.to_lowercase();
    names.iter().find(|n| n.to_lowercase() == lower).map(String::as_str)
}

fn detail(r: &AgentRef, abs: &Path, on_disk: TextRead) -> AgentDetail {
    AgentDetail {
        root_id: r.root_id.clone(),
        file: r.file.clone(),
        text: on_disk.text,
        hash: on_disk.hash,
        path: abs.to_string_lossy().into_owned(),
        read_only: on_disk.read_only,
    }
}

/// The subagents of every account and project in sight.
pub struct AgentService {
    app: Option<AppHandle>,
    trash: TrashFn,
    /// One change on disk at a time, so that two of them cannot both find a name free.
    writing: Mutex<()>,
}

impl AgentService {
    /// The service of the app: deleted agents go to the system's bin.
    pub fn new(app: AppHandle, _root: PathBuf) -> Self {
        Self::with_parts(Some(app), Arc::new(trash_file))
    }

    /// A service from its parts. Without an app handle nothing can be shown in the file manager
    /// and no folder can be picked (tests).
    pub fn with_parts(app: Option<AppHandle>, trash: TrashFn) -> Self {
        AgentService { app, trash, writing: Mutex::new(()) }
    }

    // ---------- finding ----------

    fn root<'a>(roots: &'a [RootSpec], root_id: &str) -> AppResult<&'a RootSpec> {
        roots.iter().find(|r| r.root.id == root_id).ok_or_else(|| AppError::refused(NOT_LISTED))
    }

    /// The root and the file of an agent that the root really lists.
    fn locate<'a>(roots: &'a [RootSpec], agent: &AgentRef) -> AppResult<(&'a RootSpec, PathBuf)> {
        let root = Self::root(roots, &agent.root_id)?;
        if !find_agents(&root.dir).contains(&agent.file) {
            return Err(AppError::refused(GONE));
        }
        Ok((root, scan::join_rel(&root.dir, &agent.file)))
    }

    /// The file of an agent, for showing it in the file manager.
    pub fn file_of(&self, inputs: &RootInputs, agent: &AgentRef) -> AppResult<PathBuf> {
        Ok(Self::locate(&find_roots(inputs), agent)?.1)
    }

    fn summaries(root: &RootSpec) -> Vec<AgentSummary> {
        find_agents(&root.dir).iter().map(|file| summarize(&root.root.id, &root.dir, file)).collect()
    }

    // ---------- reading ----------

    /// `agents:list`: the roots and the agents in them, with what each one says about itself and
    /// about the agents of the same name.
    pub fn list(&self, inputs: &RootInputs) -> AgentListing {
        let roots = find_roots(inputs);
        let mut agents: Vec<AgentSummary> = roots.iter().flat_map(Self::summaries).collect();
        let shown: Vec<_> = roots.into_iter().map(|r| r.root).collect();
        let named: Vec<NamedAgent<'_>> = agents.iter().map(|a| NamedAgent { root_id: &a.root_id, file: &a.file, name: &a.name }).collect();
        let mut clashes = agent_clashes(&named, &shown, |root| inputs.account_names(&root.profile_ids));
        for agent in &mut agents {
            if let Some(more) = clashes.remove(&agent_key(&agent.root_id, &agent.file)) {
                agent.issues.extend(more);
            }
        }
        AgentListing { roots: shown, agents }
    }

    /// `agents:read`.
    pub fn read(&self, inputs: &RootInputs, agent: &AgentRef) -> AppResult<AgentDetail> {
        let (_, abs) = Self::locate(&find_roots(inputs), agent)?;
        Ok(detail(agent, &abs, read_text(&abs, SKILL_LIMITS.file_bytes)?))
    }

    /// `agents:save`: writes an agent file against the state the editor started from; see
    /// [`save_text`].
    pub fn save(&self, inputs: &RootInputs, agent: &AgentRef, text: &str, base_hash: Option<&str>) -> AppResult<AgentSaveResult> {
        let _writing = lock(&self.writing);
        let (_, abs) = Self::locate(&find_roots(inputs), agent)?;
        Ok(match save_text(&abs, text, base_hash)? {
            SaveOutcome::Saved(on_disk) => SaveOutcome::Saved(detail(agent, &abs, on_disk)),
            SaveOutcome::Conflict(on_disk) => SaveOutcome::Conflict(detail(agent, &abs, on_disk)),
        })
    }

    // ---------- names ----------

    /// Checks a name for an agent of a root; `except` is the agent that may keep its own.
    fn free_name(root: &RootSpec, name: &str, except: Option<&str>) -> AppResult<()> {
        if let Some(problem) = agent_name_problem(name) {
            return Err(AppError::invalid(problem));
        }
        if let Some(taken) = Self::summaries(root).into_iter().find(|a| a.name == name && Some(a.file.as_str()) != except) {
            return Err(AppError::refused(format!("There is already an agent named \"{name}\" there ({}).", taken.file)));
        }
        Ok(())
    }

    /// Checks that a file can be made in a root; `except` is a file that may be renamed onto its
    /// own name.
    fn free_file(root: &RootSpec, file: &str, except: Option<&str>) -> AppResult<()> {
        let at = scan::join_rel(&root.dir, file);
        // the folder is made with its first agent
        let names: Vec<String> = at
            .parent()
            .and_then(|dir| fs::read_dir(dir).ok())
            .map(|read| read.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        if let Some(taken) = entry_named(&names, base_of(file)) {
            if except.is_none_or(|except| taken != base_of(except)) {
                return Err(AppError::refused(format!("There is already a file named \"{taken}\" there.")));
            }
        }
        Ok(())
    }

    /// The text of an agent under another name, stored as the original was, and the name it had.
    fn renamed(abs: &Path, name: &str) -> AppResult<(Vec<u8>, Option<String>)> {
        let decoded = decode(&fs::read(abs)?, SKILL_LIMITS.file_bytes).map_err(|why| AppError::refused(why.0))?;
        let doc = parse_doc(&decoded.text);
        if doc.error.is_some() {
            return Err(AppError::refused(FIX_FIRST));
        }
        let was = name_of(&doc);
        if was.as_deref() == Some(name) {
            return Ok((encode(&decoded.text, decoded.eol, decoded.bom), was));
        }
        let text = set_field(&decoded.text, "name", name.into()).map_err(|failure| AppError::refused(failure.0))?;
        Ok((encode(&text, decoded.eol, decoded.bom), was))
    }

    // ---------- changes ----------

    /// `agents:create`: `<name>.md` at the top of the root, with a first text that loads. The
    /// folder is made with it when the project has none yet.
    pub fn create(&self, inputs: &RootInputs, root_id: &str, name: &str, description: &str) -> AppResult<AgentRef> {
        let _writing = lock(&self.writing);
        let roots = find_roots(inputs);
        let root = Self::root(&roots, root_id)?;
        Self::free_name(root, name, None)?;
        let file = format!("{name}.md");
        Self::free_file(root, &file, None)?;
        create_text(&scan::join_rel(&root.dir, &file), new_agent_text(name, description))?;
        Ok(AgentRef { root_id: root_id.to_owned(), file })
    }

    /// `agents:duplicate`: a copy of an agent under another name, next to the original.
    pub fn duplicate(&self, inputs: &RootInputs, agent: &AgentRef, name: &str) -> AppResult<AgentRef> {
        let _writing = lock(&self.writing);
        let roots = find_roots(inputs);
        let (root, abs) = Self::locate(&roots, agent)?;
        Self::free_name(root, name, None)?;
        let file = format!("{}{name}.md", folder_of(&agent.file));
        Self::free_file(root, &file, None)?;
        let (bytes, _) = Self::renamed(&abs, name)?;
        create_text(&scan::join_rel(&root.dir, &file), bytes)?;
        Ok(AgentRef { root_id: agent.root_id.clone(), file })
    }

    /// `agents:rename`: gives an agent another name: the `name` in its file, and the file's own
    /// name when the file was named after the agent. The text changes first; when the file cannot
    /// be renamed after it, the agent keeps its new name and the result says so.
    pub fn rename(&self, inputs: &RootInputs, agent: &AgentRef, name: &str) -> AppResult<AgentMoved> {
        let _writing = lock(&self.writing);
        let roots = find_roots(inputs);
        let (root, abs) = Self::locate(&roots, agent)?;
        Self::free_name(root, name, Some(&agent.file))?;
        let (bytes, was) = Self::renamed(&abs, name)?;
        let same = AgentMoved { r#ref: agent.clone(), warning: None };
        if was.as_deref() == Some(name) {
            return Ok(same);
        }
        let follows = was.as_deref() == Some(agent_stem(&agent.file));
        let file = if follows { format!("{}{name}.md", folder_of(&agent.file)) } else { agent.file.clone() };
        if follows {
            Self::free_file(root, &file, Some(&agent.file))?;
        }
        write_file_atomic(&abs, &bytes)?;
        if !follows {
            return Ok(same);
        }
        let target = scan::join_rel(&root.dir, &file);
        if retry_busy(|| fs::rename(&abs, &target), BUSY_TRIES, BUSY_WAIT).is_err() {
            return Ok(AgentMoved {
                r#ref: agent.clone(),
                warning: Some(format!("The agent has its new name; its file is still called \"{}\".", agent.file)),
            });
        }
        Ok(AgentMoved { r#ref: AgentRef { root_id: agent.root_id.clone(), file }, warning: None })
    }

    /// `agents:delete`: puts an agent's file into the bin. A file that is a link loses only the
    /// link. It is never deleted for good instead.
    pub fn remove(&self, inputs: &RootInputs, agent: &AgentRef) -> AppResult<()> {
        let _writing = lock(&self.writing);
        self.remove_located(&Self::locate(&find_roots(inputs), agent)?.1)
    }

    fn remove_located(&self, abs: &Path) -> AppResult<()> {
        if fs::symlink_metadata(abs)?.file_type().is_symlink() {
            fs::remove_file(abs)?;
            return Ok(());
        }
        retry_busy(|| (self.trash)(abs), BUSY_TRIES, BUSY_WAIT).map_err(|err| {
            if is_busy(&err) {
                busy_message(err, "Deleting")
            } else {
                AppError::io(format!("The agent could not be moved to the bin ({err}). It was not deleted."))
            }
        })
    }

    /// `agents:transfer`: copies an agent into another root, or moves it there (a copy, then the
    /// original into the bin). With another `name` the copy answers to that one and its file is
    /// named after it; without, the bytes go over as they are, under the file's own name.
    pub fn transfer(&self, inputs: &RootInputs, agent: &AgentRef, to_root_id: &str, mode: TransferMode, name: &str) -> AppResult<AgentMoved> {
        let _writing = lock(&self.writing);
        let roots = find_roots(inputs);
        let (from, abs) = Self::locate(&roots, agent)?;
        if to_root_id == agent.root_id {
            return Err(AppError::refused("The agent is in that folder already."));
        }
        let target = Self::root(&roots, to_root_id)?;
        let current = summarize(&from.root.id, &from.dir, &agent.file).name;
        let wanted = if name.is_empty() { current.as_str() } else { name };
        let (bytes, file) = if wanted == current {
            // as it is, under the name its file has
            if !current.is_empty() {
                if let Some(taken) = Self::summaries(target).into_iter().find(|a| a.name == current) {
                    return Err(AppError::refused(format!("There is already an agent named \"{current}\" there ({}).", taken.file)));
                }
            }
            (fs::read(&abs)?, base_of(&agent.file).to_owned())
        } else {
            Self::free_name(target, wanted, None)?;
            (Self::renamed(&abs, wanted)?.0, format!("{wanted}.md"))
        };
        Self::free_file(target, &file, None)?;
        create_text(&scan::join_rel(&target.dir, &file), bytes)?;
        let copy = AgentRef { root_id: to_root_id.to_owned(), file };
        if mode == TransferMode::Copy {
            return Ok(AgentMoved { r#ref: copy, warning: None });
        }
        if let Err(err) = self.remove_located(&abs) {
            return Ok(AgentMoved { r#ref: copy, warning: Some(format!("The agent was copied, but the original is still there: {err}")) });
        }
        Ok(AgentMoved { r#ref: copy, warning: None })
    }

    // ---------- the file manager and the folder list ----------

    /// `agents:reveal`: the agent's file in the file manager.
    pub fn reveal(&self, inputs: &RootInputs, agent: &AgentRef) -> AppResult<()> {
        use tauri_plugin_opener::OpenerExt;
        let abs = self.file_of(inputs, agent)?;
        let app = self.app.as_ref().ok_or_else(|| AppError::refused("There is no window to show the file from."))?;
        app.opener().reveal_item_in_dir(abs).map_err(|e| AppError::io(e.to_string()))
    }

    /// Asks for a project folder; None when the dialog was cancelled.
    async fn pick_folder(&self) -> AppResult<Option<String>> {
        use tauri_plugin_dialog::DialogExt;
        let app = self.app.as_ref().ok_or_else(|| AppError::refused("There is no window to ask in."))?;
        let mut dialog = app.dialog().file().set_title("Choose a project folder");
        if let Some(win) = crate::app::window::main_window(app) {
            dialog = dialog.set_parent(&win);
        }
        let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder()).await.map_err(|e| AppError::io(e.to_string()))?;
        Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
    }

    /// `agents:addFolder`: asks for a project folder and keeps it in the Library's list; the id of
    /// its agents root (found from `inputs`, read after the folder was added), or None when the
    /// dialog was cancelled or the folder's agents are an account's own.
    pub async fn add_folder(&self, config: &ConfigStore, inputs: impl FnOnce() -> RootInputs) -> AppResult<Option<String>> {
        let Some(folder) = self.pick_folder().await? else { return Ok(None) };
        pin_folder(config, &folder)?;
        Ok(project_root_of(&find_roots(&inputs()), &folder))
    }

    /// `agents:removeFolder`: takes a project's folder out of the Library's list.
    pub fn remove_folder(&self, config: &ConfigStore, inputs: &RootInputs, root_id: &str) -> AppResult<()> {
        unpin_root(config, &find_roots(inputs), root_id)
    }
}
