//! Where the Library looks for subagents (`src/main/skills/roots.ts` with `sub: 'agents'`, and the
//! folder list of `libraryRoots`, `pickLibraryFolder`, `addLibraryFolder` and `removeLibraryFolder`
//! of `src/main/index.ts`).
//!
//! Private to the agents package for now: the skills package finds its roots the same way, and the
//! two are to be made one shared helper.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use mc_core::error::AppResult;
use mc_core::folders::folder_key;
use mc_core::types::{AgentRoot, SkillRootKind};
use sha1::{Digest, Sha1};

use crate::config_store::ConfigStore;

/// The folder of an account or a project that holds its agents.
const SUB: &str = "agents";
/// How far above a folder its project's agents are looked for.
const MAX_UP: usize = 6;
/// How many folders the user can add to the list by hand (`slice(-30)`).
pub const PINNED_MAX: usize = 30;

/// An account and the Claude Code folder it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFolder {
    pub id: String,
    /// For telling whose personal agents a folder holds.
    pub name: String,
    /// `effectiveConfigDir`.
    pub config_dir: PathBuf,
}

/// `RootInputs`: what the roots are found from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootInputs {
    /// The accounts, with the Claude Code folder each one uses.
    pub profiles: Vec<ProfileFolder>,
    /// Folders worth looking at: where sessions run, what was opened lately.
    pub folders: Vec<String>,
    /// Folders the user added to the list by hand (`settings.skillFolders`); they stay in it
    /// without agents too.
    pub pinned: Vec<String>,
}

impl RootInputs {
    /// `accountNames`: the names of these accounts, joined by commas.
    pub fn account_names(&self, ids: &[String]) -> String {
        let names: Vec<&str> = ids
            .iter()
            .filter_map(|id| self.profiles.iter().find(|p| p.id == *id))
            .map(|p| p.name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        names.join(", ")
    }
}

/// `RootSpec`: a folder that holds agents, as the window sees it and as it is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSpec {
    pub root: AgentRoot,
    /// `root.dir` as a path.
    pub dir: PathBuf,
    /// The folder with links resolved; what the id stands for.
    pub real: PathBuf,
}

fn is_dir(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_dir())
}

/// `path.isAbsolute`: on Windows a path that starts at the root of the current drive is one too.
fn is_absolute_js(p: &Path) -> bool {
    p.is_absolute() || p.has_root()
}

/// `path.resolve`.
fn resolve(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// A canonical path as Node's `realpath` gives it: without the `\\?\` that Windows puts in front.
pub(super) fn plain(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = p.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            // only a drive path; other verbatim paths have no plain spelling
            if rest.as_bytes().get(1) == Some(&b':') {
                return PathBuf::from(rest);
            }
        }
    }
    p
}

/// The folder a path really is, also when it is reached through a link or does not exist yet.
fn real_of(p: &Path) -> PathBuf {
    fs::canonicalize(p).map(plain).unwrap_or_else(|_| resolve(p))
}

fn key_of(real: &Path) -> String {
    let text = real.to_string_lossy();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

fn id_of(prefix: &str, real: &Path) -> String {
    let digest = hex::encode(Sha1::digest(key_of(real).as_bytes()));
    format!("{prefix}-{}", &digest[..12])
}

fn shown(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The agents folders of a project as Claude Code finds them from `folder`: the folder's own and
/// those of the folders above it, up to the top of the repository.
fn project_dirs(folder: &Path) -> Vec<(PathBuf, PathBuf)> {
    let start = resolve(folder);
    // the folders from this one up to the top of its repository; outside a repository only itself,
    // or the walk would end in the home folder and call the personal agents a project's
    let mut chain = vec![start.clone()];
    let mut in_repo = start.join(".git").exists();
    let mut at = start.clone();
    for _ in 0..MAX_UP {
        if in_repo {
            break;
        }
        let Some(up) = at.parent().map(Path::to_path_buf) else { break };
        in_repo = up.join(".git").exists();
        chain.push(up.clone());
        at = up;
    }
    let chain = if in_repo { chain } else { vec![start] };
    chain
        .into_iter()
        .filter_map(|at| {
            let dir = at.join(".claude").join(SUB);
            is_dir(&dir).then_some((at, dir))
        })
        .collect()
}

/// `findRoots(inputs, 'agents')`: the folders that hold agents. Accounts whose folder is one and
/// the same folder (the others link to the main account's) make one personal root; a project root
/// is a `.claude/agents` folder. A root's id stands for its real path, so the window never names
/// a path itself.
pub fn find_roots(inputs: &RootInputs) -> Vec<RootSpec> {
    let mut roots: Vec<RootSpec> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();

    for profile in &inputs.profiles {
        let dir = profile.config_dir.join(SUB);
        let real = real_of(&dir);
        if let Some(&known) = seen.get(&key_of(&real)) {
            roots[known].root.profile_ids.push(profile.id.clone());
            continue;
        }
        let root = AgentRoot {
            id: id_of("p", &real),
            kind: SkillRootKind::Personal,
            dir: shown(&dir),
            profile_ids: vec![profile.id.clone()],
            folder: None,
            exists: is_dir(&dir),
            pinned: None,
        };
        seen.insert(key_of(&real), roots.len());
        roots.push(RootSpec { root, dir, real });
    }

    let mut add_project = |folder: &Path, dir: PathBuf, pinned: bool| {
        let real = real_of(&dir);
        if let Some(&known) = seen.get(&key_of(&real)) {
            // the home folder's agents are the personal ones; a folder that is pinned stays marked
            let known = &mut roots[known].root;
            if known.kind == SkillRootKind::Project && pinned {
                known.pinned = Some(true);
            }
            return;
        }
        let root = AgentRoot {
            id: id_of("j", &real),
            kind: SkillRootKind::Project,
            dir: shown(&dir),
            profile_ids: Vec::new(),
            folder: Some(shown(folder)),
            exists: is_dir(&dir),
            pinned: pinned.then_some(true),
        };
        seen.insert(key_of(&real), roots.len());
        roots.push(RootSpec { root, dir, real });
    };

    for folder in &inputs.pinned {
        let folder = Path::new(folder);
        if !is_absolute_js(folder) {
            continue;
        }
        // shown without agents as well, so that the first one can be made there
        let at = resolve(folder);
        add_project(&at, at.join(".claude").join(SUB), true);
        for (found, dir) in project_dirs(folder) {
            add_project(&found, dir, false);
        }
    }
    for folder in &inputs.folders {
        let folder = Path::new(folder);
        if !is_absolute_js(folder) {
            continue;
        }
        for (found, dir) in project_dirs(folder) {
            add_project(&found, dir, false);
        }
    }
    roots
}

/// `pickLibraryFolder`, after the dialog: keeps a folder in the Library's list (the last 30), once
/// whatever its spelling.
pub fn pin_folder(config: &ConfigStore, folder: &str) -> AppResult<()> {
    let windows = cfg!(windows);
    let key = folder_key(folder, windows);
    if config.with(|c| c.settings.skill_folders.as_deref().unwrap_or_default().iter().any(|f| folder_key(f, windows) == key)) {
        return Ok(());
    }
    config.update(|c| {
        let mut pinned = c.settings.skill_folders.take().unwrap_or_default();
        pinned.push(folder.to_owned());
        let skip = pinned.len().saturating_sub(PINNED_MAX);
        c.settings.skill_folders = Some(pinned.split_off(skip));
    })
}

/// `removeLibraryFolder`: takes the folder of a project root out of the Library's list. A root
/// that is not in the list, or not a project's, changes nothing.
pub fn unpin_root(config: &ConfigStore, roots: &[RootSpec], root_id: &str) -> AppResult<()> {
    let Some(folder) = roots.iter().find(|r| r.root.id == root_id).and_then(|r| r.root.folder.clone()) else { return Ok(()) };
    let windows = cfg!(windows);
    let key = folder_key(&folder, windows);
    config.update(|c| {
        let pinned = c.settings.skill_folders.take().unwrap_or_default();
        c.settings.skill_folders = Some(pinned.into_iter().filter(|f| folder_key(f, windows) != key).collect());
    })
}

/// `addLibraryFolder`, after the folder was pinned: the id of its agents root; None when that
/// folder's agents are an account's own, as with the home folder.
pub fn project_root_of(roots: &[RootSpec], folder: &str) -> Option<String> {
    let windows = cfg!(windows);
    let dir = folder_key(&shown(&Path::new(folder).join(".claude").join(SUB)), windows);
    roots.iter().find(|r| r.root.kind == SkillRootKind::Project && folder_key(&r.root.dir, windows) == dir).map(|r| r.root.id.clone())
}
