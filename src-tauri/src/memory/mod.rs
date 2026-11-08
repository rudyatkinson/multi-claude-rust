//! Auto memory per account and project, sharing and adopting (`src/main/memory/*`).
//!
//! - `folders.rs`: where the memory folders are, and which project folder a name stands for;
//! - `share.rs`: the main account's folder for a sharing account's session, and bringing an
//!   account's own memories into it;
//! - [`MemoryService`]: everything the Library's memory browser does on disk.
//!
//! The service is handed what it needs to know of the app on every call ([`MemoryInputs`]: the
//! accounts and the folders the app knows), so that it never holds on to an account list that
//! has changed since.

mod folders;
mod share;
#[cfg(test)]
mod tests;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use mc_core::error::{AppError, AppResult};
use mc_core::hooks::iso_string;
use mc_core::memory::{
    add_index_line, entry_named, inspect_memory, is_index, line_count, memory_meta, memory_name_problem, memory_snippet,
    new_memory_text, one_line, parse_index, remove_index_line, same_name, search_words, stem_of, IndexFile, SNIPPET_AROUND,
};
use mc_core::skills::locale_compare;
use mc_core::text_file::{create_text, decode, encode, read_text, save_text, write_file_atomic, TextRead};
use mc_core::types::{
    AdoptResult, IndexLine, MemoryCreateInput, MemoryDetail, MemoryEntry, MemoryFolder, MemoryHit, MemoryListing, MemoryProject, MemoryRef,
    MemorySaveResult, MemoryView, Profile, SaveOutcome, SessionKind, MEMORY_INDEX, MEMORY_LIMITS, MEMORY_TYPES, SKILL_LIMITS,
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::library::trash::{system_trash_fn, trash_with};

pub use folders::{find_memory_folders, folder_id, memory_files, FolderSpec, MemoryFile, ProjectFolders};
pub use share::{adopt_memories, naming_problem, shared_memory_dir, Clock, MemorySharing, CHECK_MS};

use crate::config_store::now_ms;
use crate::paths::Paths;
use crate::state::AppState;

const GONE: &str = "That memory is no longer there. The list may be out of date.";
const FOLDER_GONE: &str = "That memory folder is no longer there. The list may be out of date.";
/// A file larger than this is not searched.
const SEARCH_BYTES: u64 = 400_000;

/// An account as the memory service sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryAccount {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// `effectiveConfigDir`: the account's CLAUDE_CONFIG_DIR, `~/.claude` for Main.
    pub config_dir: PathBuf,
    pub is_default: bool,
    /// The account's sessions use the main account's memory folders.
    pub share_memory: bool,
}

impl MemoryAccount {
    pub fn of(profile: &Profile, paths: &Paths) -> Self {
        MemoryAccount {
            id: profile.id.clone(),
            name: profile.name.clone(),
            slug: profile.slug.clone(),
            config_dir: paths.config_dir_of(profile),
            is_default: profile.is_default,
            share_memory: profile.share_memory == Some(true),
        }
    }
}

/// What the service needs to know of the app for one call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryInputs {
    pub accounts: Vec<MemoryAccount>,
    /// Folders the app knows, for telling which project a memory folder belongs to.
    pub folders: Vec<String>,
}

impl MemoryInputs {
    /// The accounts, and the folders the app has to do with: where Claude sessions run, what was
    /// opened lately, what was added to the Library (`knownFolders` of `src/main/index.ts`).
    pub fn of(state: &AppState) -> Self {
        let paths = state.profiles.paths();
        let accounts = state.profiles.list().iter().map(|p| MemoryAccount::of(p, paths)).collect();
        let mut folders: Vec<String> = state.sessions.list().into_iter().filter(|s| s.kind == SessionKind::Claude).map(|s| s.cwd).collect();
        state.profiles.config().with(|c| {
            folders.extend(c.recent_folders.iter().cloned());
            folders.extend(c.settings.skill_folders.iter().flatten().cloned());
        });
        MemoryInputs { accounts, folders }
    }

    fn main(&self) -> Option<&MemoryAccount> {
        self.accounts.iter().find(|a| a.is_default)
    }
}

pub use crate::library::trash::TrashFn;

/// `sameKey`: two project names are one project, in any case on Windows.
fn same_key(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        same_name(a, b)
    } else {
        a == b
    }
}

/// The text of a file and its size, or None when it is not there or not text this app reads.
fn text_of(file: &Path) -> Option<(String, u64)> {
    let bytes = fs::read(file).ok()?;
    let decoded = decode(&bytes, SKILL_LIMITS.file_bytes).ok()?;
    Some((decoded.text, bytes.len() as u64))
}

/// Changes a text file in place, keeping its line ends and byte order mark; makes it when it is
/// missing. Nothing is written when the text stays the same.
fn rewrite(file: &Path, change: impl FnOnce(&str) -> String) -> AppResult<()> {
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return create_text(file, change("")),
        Err(e) => return Err(e.into()),
    };
    let decoded = decode(&bytes, SKILL_LIMITS.file_bytes).map_err(|why| {
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        AppError::io(format!("{name} cannot be changed: {why}"))
    })?;
    let next = change(&decoded.text);
    if next != decoded.text {
        write_file_atomic(file, &encode(&next, decoded.eol, decoded.bom))?;
    }
    Ok(())
}

/// `line?.title || meta.name || stem`.
fn title_of(line: Option<&IndexLine>, name: &str, file: &str) -> String {
    [line.map_or("", |l| l.title.as_str()), name].into_iter().find(|t| !t.is_empty()).unwrap_or_else(|| stem_of(file)).to_owned()
}

fn detail(r: &MemoryRef, abs: &Path, on_disk: TextRead) -> MemoryDetail {
    MemoryDetail {
        folder_id: r.folder_id.clone(),
        file: r.file.clone(),
        text: on_disk.text,
        hash: on_disk.hash,
        path: abs.to_string_lossy().into_owned(),
        read_only: on_disk.read_only,
    }
}

/// Everything the memory browser does on disk. A memory is always named by its folder's id and
/// its file's name; the file has to be one the folder really lists (in a listing made for the
/// call), so nothing that comes from the window is ever used as a path as it stands.
pub struct MemoryService {
    app: Option<AppHandle>,
    project_folders: ProjectFolders,
    sharing: MemorySharing,
    clock: Clock,
    trash: RwLock<TrashFn>,
}

impl MemoryService {
    /// The service of the app. The memories live in the accounts' config dirs, not under `root`.
    pub fn new(app: AppHandle, _root: PathBuf) -> Self {
        Self::with_parts(Some(app), Arc::new(now_ms))
    }

    /// A service from its parts. Without an app handle nothing can be shown in the file manager
    /// (tests).
    pub fn with_parts(app: Option<AppHandle>, clock: Clock) -> Self {
        MemoryService { app, project_folders: ProjectFolders::new(), sharing: MemorySharing::new(clock.clone()), clock, trash: RwLock::new(system_trash_fn()) }
    }

    /// Replaces moving a deleted memory to the system's bin (tests).
    pub fn set_trash(&self, f: impl Fn(&Path) -> io::Result<()> + Send + Sync + 'static) {
        *self.trash.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(f);
    }

    /// Where a sharing account's session keeps its memory; the look at the names is kept a while.
    pub fn sharing(&self) -> &MemorySharing {
        &self.sharing
    }

    /// What `prepare_settings` puts into a session's settings as `autoMemoryDirectory`: the main
    /// account's memory folder for the project `cwd` lies in, for an account that shares it.
    /// None for Main and for an account that keeps its own memory. The error is the warning to
    /// show when sharing is refused for this folder; the session then keeps its own memory.
    pub fn auto_memory_directory(&self, main_config_dir: &Path, profile: &Profile, cwd: &Path) -> Result<Option<PathBuf>, String> {
        if profile.is_default || profile.share_memory != Some(true) {
            return Ok(None);
        }
        // the session keeps its memory where the main account keeps it for this project
        match self.sharing.dir_for(main_config_dir, cwd) {
            Ok(dir) => Ok(Some(dir)),
            Err(why) => Err(format!("{} uses its own memory in this folder. {why}", profile.name)),
        }
    }

    /// The memory folder a session of the account reads MEMORY.md from in `cwd` (`memoryIndex` of
    /// the instruction service's dependencies, as a folder; it fits
    /// `InstructionService::set_memory_dir`): the main account's when the account shares and
    /// sharing is not refused, else its own. None when the project cannot be named.
    /// `own_config_dir` is the account's `effectiveConfigDir`.
    pub fn session_memory_dir(&self, main_config_dir: &Path, profile: &Profile, own_config_dir: &Path, cwd: &Path) -> Option<PathBuf> {
        let shares = profile.share_memory == Some(true) && !profile.is_default;
        let found = if shares { self.sharing.dir_for(main_config_dir, cwd).ok() } else { None };
        found.or_else(|| shared_memory_dir(own_config_dir, cwd, None).ok())
    }

    fn specs(&self, inputs: &MemoryInputs) -> Vec<FolderSpec> {
        find_memory_folders(inputs.accounts.iter().map(|a| (a.id.as_str(), a.config_dir.as_path())))
    }

    fn folder(&self, inputs: &MemoryInputs, folder_id: &str) -> AppResult<FolderSpec> {
        self.specs(inputs).into_iter().find(|s| s.id == folder_id).ok_or_else(|| AppError::refused(FOLDER_GONE))
    }

    fn locate(&self, inputs: &MemoryInputs, r: &MemoryRef) -> AppResult<(FolderSpec, PathBuf)> {
        let spec = self.folder(inputs, &r.folder_id)?;
        if !spec.files.contains(&r.file) {
            return Err(AppError::refused(GONE));
        }
        let abs = spec.dir.join(&r.file);
        Ok((spec, abs))
    }

    /// The folder, for showing it in the file manager.
    pub fn dir_of(&self, inputs: &MemoryInputs, folder_id: &str) -> AppResult<PathBuf> {
        Ok(self.folder(inputs, folder_id)?.dir)
    }

    /// A memory's file, once its folder lists it.
    pub fn file_of(&self, inputs: &MemoryInputs, r: &MemoryRef) -> AppResult<PathBuf> {
        Ok(self.locate(inputs, r)?.1)
    }

    /// The projects that have a memory in at least one account, the latest change first; with
    /// each folder the accounts whose own folder it is and those that use it by sharing.
    pub fn list(&self, inputs: &MemoryInputs) -> MemoryListing {
        let accounts = &inputs.accounts;
        let main = inputs.main();
        let sharing: Vec<&str> = accounts.iter().filter(|a| !a.is_default && a.share_memory).map(|a| a.id.as_str()).collect();
        let mut projects: Vec<MemoryProject> = Vec::new();
        for spec in self.specs(inputs) {
            let at = match projects.iter().position(|p| same_key(&p.key, &spec.key)) {
                Some(at) => at,
                None => {
                    let dirs: Vec<PathBuf> = accounts.iter().map(|a| a.config_dir.join("projects").join(&spec.key)).collect();
                    let cwd = self.project_folders.cwd_of(&spec.key, &dirs, &inputs.folders);
                    projects.push(MemoryProject { key: spec.key.clone(), cwd, folders: Vec::new() });
                    projects.len() - 1
                }
            };
            let main_owns = main.is_some_and(|m| spec.profile_ids.contains(&m.id));
            let shared_with = if main_owns {
                sharing.iter().filter(|id| !spec.profile_ids.iter().any(|p| p == *id)).map(|id| (*id).to_owned()).collect()
            } else {
                Vec::new()
            };
            projects[at].folders.push(MemoryFolder {
                id: spec.id,
                count: u32::try_from(spec.files.iter().filter(|f| !is_index(f)).count()).unwrap_or(u32::MAX),
                profile_ids: spec.profile_ids,
                shared_with,
                dir: spec.dir.to_string_lossy().into_owned(),
                updated_at: spec.updated_at,
            });
        }
        let latest = |p: &MemoryProject| p.folders.iter().map(|f| f.updated_at).max().unwrap_or(0);
        projects.sort_by_key(|p| std::cmp::Reverse(latest(p)));
        let share_problem = main.and_then(|m| self.sharing.problem(&m.config_dir, false));
        MemoryListing { projects, share_problem }
    }

    /// One folder: its memories as the index lists them, then those the index does not know; and
    /// what a session would miss.
    pub fn entries(&self, inputs: &MemoryInputs, folder_id: &str) -> AppResult<MemoryView> {
        let spec = self.folder(inputs, folder_id)?;
        let files = memory_files(&spec.dir);
        let index = text_of(&spec.dir.join(MEMORY_INDEX));
        let has_index = files.iter().any(|f| is_index(&f.name));
        let listed = index.as_ref().map(|(text, _)| parse_index(text)).unwrap_or_default();
        let line_of = |file: &str| listed.iter().position(|e| same_name(&e.file, file));
        let index_lines = index.as_ref().map_or(0, |(text, _)| line_count(text));

        let mut entries: Vec<MemoryEntry> = Vec::new();
        for file in &files {
            if is_index(&file.name) {
                let unit = if index_lines == 1 { "line" } else { "lines" };
                entries.push(MemoryEntry {
                    file: file.name.clone(),
                    title: MEMORY_INDEX.to_owned(),
                    description: format!("The index every session reads: {index_lines} {unit}"),
                    r#type: String::new(),
                    updated_at: file.mtime_ms,
                    size: file.size,
                    indexed: true,
                });
                continue;
            }
            let meta = memory_meta(&text_of(&spec.dir.join(&file.name)).map(|(text, _)| text).unwrap_or_default());
            let line = line_of(&file.name).map(|at| &listed[at]);
            entries.push(MemoryEntry {
                file: file.name.clone(),
                title: title_of(line, &meta.name, &file.name),
                description: line.map(|l| l.hook.clone()).filter(|h| !h.is_empty()).unwrap_or(meta.description),
                r#type: meta.r#type,
                updated_at: file.mtime_ms,
                size: file.size,
                indexed: line.is_some(),
            });
        }
        let order = |e: &MemoryEntry| -> i64 {
            if is_index(&e.file) {
                -1
            } else {
                line_of(&e.file).map_or(i64::MAX, |at| at as i64)
            }
        };
        entries.sort_by(|a, b| order(a).cmp(&order(b)).then_with(|| locale_compare(&a.file, &b.file)));

        let topics: Vec<&str> = files.iter().filter(|f| !is_index(&f.name)).map(|f| f.name.as_str()).collect();
        let index_file =
            has_index.then(|| index.as_ref().map_or(IndexFile { text: "", bytes: 0 }, |(text, bytes)| IndexFile { text, bytes: *bytes }));
        let issues = inspect_memory(index_file, &topics);
        Ok(MemoryView {
            folder_id: folder_id.to_owned(),
            dir: spec.dir.to_string_lossy().into_owned(),
            has_index,
            index_lines: u32::try_from(index_lines).unwrap_or(u32::MAX),
            index_bytes: index.as_ref().map_or(0, |(_, bytes)| *bytes),
            entries,
            issues,
        })
    }

    pub fn read(&self, inputs: &MemoryInputs, r: &MemoryRef) -> AppResult<MemoryDetail> {
        let (_, abs) = self.locate(inputs, r)?;
        Ok(detail(r, &abs, read_text(&abs, SKILL_LIMITS.file_bytes)?))
    }

    /// Writes a memory file against the state the editor started from; see [`save_text`].
    pub fn save(&self, inputs: &MemoryInputs, r: &MemoryRef, text: &str, base_hash: Option<&str>) -> AppResult<MemorySaveResult> {
        let (_, abs) = self.locate(inputs, r)?;
        Ok(match save_text(&abs, text, base_hash)? {
            SaveOutcome::Saved(on_disk) => SaveOutcome::Saved(detail(r, &abs, on_disk)),
            SaveOutcome::Conflict(on_disk) => SaveOutcome::Conflict(detail(r, &abs, on_disk)),
        })
    }

    /// Makes a memory file and gives it a line in the index, as Claude Code does when it
    /// remembers something.
    pub fn create(&self, inputs: &MemoryInputs, folder_id: &str, input: &MemoryCreateInput) -> AppResult<MemoryRef> {
        let spec = self.folder(inputs, folder_id)?;
        if let Some(problem) = memory_name_problem(&input.name) {
            return Err(AppError::invalid(problem));
        }
        let kind = MEMORY_TYPES.into_iter().find(|t| t.as_str() == input.r#type).ok_or_else(|| AppError::invalid("Invalid type"))?;
        let file = format!("{}.md", input.name);
        if let Some(taken) = entry_named(&spec.files, &file) {
            return Err(AppError::refused(format!("There is already a memory named \"{taken}\" there.")));
        }
        create_text(&spec.dir.join(&file), new_memory_text(&input.name, &input.description, kind, &iso_string((self.clock)())))?;
        let title = if one_line(&input.title).is_empty() { &input.name } else { &input.title };
        rewrite(&spec.dir.join(MEMORY_INDEX), |text| add_index_line(text, title, &file, &input.description))?;
        Ok(MemoryRef { folder_id: folder_id.to_owned(), file })
    }

    /// Puts a memory into the bin and takes its line out of the index. A memory that cannot go
    /// to the bin is never deleted for good instead.
    pub fn delete(&self, inputs: &MemoryInputs, r: &MemoryRef) -> AppResult<()> {
        let (spec, abs) = self.locate(inputs, r)?;
        if is_index(&r.file) {
            return Err(AppError::refused("MEMORY.md is the index of this folder. Delete the memories it lists instead."));
        }
        let trash = self.trash.read().unwrap_or_else(PoisonError::into_inner).clone();
        trash_with(&trash, &abs, "memory")?;
        if spec.files.iter().any(|f| is_index(f)) {
            rewrite(&spec.dir.join(MEMORY_INDEX), |text| remove_index_line(text, &r.file))?;
        }
        Ok(())
    }

    /// Brings the memories of an account's own folder into the main account's folder of the same
    /// project, where a sharing account's sessions read and write. Without `apply` it only says
    /// what would happen. The account's own folder is left as it is.
    pub fn adopt(&self, inputs: &MemoryInputs, folder_id: &str, apply: bool) -> AppResult<AdoptResult> {
        let spec = self.folder(inputs, folder_id)?;
        let main = inputs.main().ok_or_else(|| AppError::refused("There is no main account to share with."))?;
        if spec.profile_ids.contains(&main.id) {
            return Err(AppError::refused("This is the main account's memory already."));
        }
        let from = inputs
            .accounts
            .iter()
            .find(|a| spec.profile_ids.contains(&a.id))
            .ok_or_else(|| AppError::refused("The account of this memory is no longer there."))?;
        let projects = main.config_dir.join("projects");
        let names = share::names_in(&projects);
        let folder = entry_named(&names, &spec.key).unwrap_or(&spec.key);
        adopt_memories(&spec.dir, &projects.join(folder).join("memory"), &from.slug, &from.name, apply)
    }

    /// The memories of every project and account in which every word of the query occurs, at
    /// most [`MEMORY_LIMITS`]`.hits`. The line of the index counts as part of a memory.
    pub fn search(&self, inputs: &MemoryInputs, query: &str) -> Vec<MemoryHit> {
        let words = search_words(query);
        if words.is_empty() {
            return Vec::new();
        }
        let listing = self.list(inputs);
        let specs = self.specs(inputs);
        let mut hits: Vec<MemoryHit> = Vec::new();
        for project in &listing.projects {
            for folder in &project.folders {
                let Some(spec) = specs.iter().find(|s| s.id == folder.id) else { continue };
                let listed = text_of(&spec.dir.join(MEMORY_INDEX)).map(|(text, _)| parse_index(&text)).unwrap_or_default();
                for file in &spec.files {
                    if hits.len() >= MEMORY_LIMITS.hits {
                        return hits;
                    }
                    if is_index(file) {
                        continue;
                    }
                    let abs = spec.dir.join(file);
                    if !fs::metadata(&abs).is_ok_and(|m| m.len() <= SEARCH_BYTES) {
                        continue;
                    }
                    let Some((text, _)) = text_of(&abs) else { continue };
                    let line = listed.iter().find(|e| same_name(&e.file, file));
                    // the line of the index is what a session sees first
                    let searched = match line {
                        Some(l) => format!("{} — {}\n{text}", l.title, l.hook),
                        None => text.clone(),
                    };
                    let Some(snippet) = memory_snippet(&searched, &words, SNIPPET_AROUND) else { continue };
                    let title = match line.map(|l| l.title.as_str()).filter(|t| !t.is_empty()) {
                        Some(title) => title.to_owned(),
                        None => title_of(None, &memory_meta(&text).name, file),
                    };
                    hits.push(MemoryHit {
                        folder_id: spec.id.clone(),
                        file: file.clone(),
                        key: project.key.clone(),
                        cwd: project.cwd.clone(),
                        profile_ids: spec.profile_ids.clone(),
                        title,
                        snippet,
                    });
                }
            }
        }
        hits
    }

    /// Shows a memory in the file manager, or opens its folder when no file is named.
    pub fn reveal(&self, inputs: &MemoryInputs, folder_id: &str, file: Option<&str>) -> AppResult<()> {
        let app = self.app.as_ref().ok_or_else(|| AppError::refused("The file manager is not available."))?;
        match file {
            Some(file) => {
                let path = self.file_of(inputs, &MemoryRef { folder_id: folder_id.to_owned(), file: file.to_owned() })?;
                app.opener().reveal_item_in_dir(path).map_err(|e| AppError::io(e.to_string()))
            }
            None => {
                let dir = self.dir_of(inputs, folder_id)?;
                app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| AppError::io(e.to_string()))
            }
        }
    }
}
