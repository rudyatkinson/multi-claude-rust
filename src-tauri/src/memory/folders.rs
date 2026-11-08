//! Where the memories of the accounts lie on disk (`src/main/memory/folders.ts`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use mc_core::conversations::read_head;
use mc_core::memory::{is_markdown, memory_key, sanitize_path};
use mc_core::skills::locale_compare;
use mc_core::types::MEMORY_INDEX;
use sha1::{Digest, Sha1};

use crate::state::lock;

/// A memory folder of one or more accounts, as found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSpec {
    /// Stands for the folder's real path.
    pub id: String,
    /// The accounts whose folder this is: several when their config dirs lead to the same place.
    pub profile_ids: Vec<String>,
    /// The project's name under `projects`.
    pub key: String,
    /// `<config>/projects/<key>/memory`, of the first account that has it.
    pub dir: PathBuf,
    /// The Markdown files in it, MEMORY.md among them.
    pub files: Vec<String>,
    /// When a file in it last changed, epoch ms.
    pub updated_at: i64,
}

/// A Markdown file of a memory folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFile {
    pub name: String,
    pub size: u64,
    /// epoch ms
    pub mtime_ms: i64,
}

/// What `fs.realpathSync` gives on Windows: the path without the `\\?\` that `canonicalize` puts
/// in front.
fn without_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

/// The real path of a folder (links and junctions followed), or the path itself made absolute.
fn real_of(p: &Path) -> PathBuf {
    match fs::canonicalize(p) {
        Ok(real) => without_verbatim(real),
        Err(_) => mc_core::memory::resolve_path(p),
    }
}

/// `m-` and 12 hex digits of the sha1 of the folder's real path (in lower case on Windows, where
/// the case of a path does not matter).
pub fn folder_id(real: &Path) -> String {
    let text = real.to_string_lossy();
    let key = if cfg!(windows) { text.to_lowercase() } else { text.into_owned() };
    let digest = hex::encode(Sha1::digest(key.as_bytes()));
    format!("m-{}", &digest[..12])
}

/// Epoch ms of a time, 0 for one before 1970 or unknown.
pub(crate) fn epoch_ms(time: std::io::Result<SystemTime>) -> i64 {
    time.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The Markdown files of a memory folder: the index first, then by name. Folders inside it (logs,
/// an agent's own notes) and links are left alone. Nothing for a folder that is not there.
pub fn memory_files(dir: &Path) -> Vec<MemoryFile> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type().is_ok_and(|t| t.is_file()) || !is_markdown(&name) {
            continue;
        }
        // gone since it was listed
        let Ok(meta) = fs::metadata(entry.path()) else { continue };
        files.push(MemoryFile { name, size: meta.len(), mtime_ms: epoch_ms(meta.modified()) });
    }
    files.sort_by(|a, b| (b.name == MEMORY_INDEX).cmp(&(a.name == MEMORY_INDEX)).then_with(|| locale_compare(&a.name, &b.name)));
    files
}

/// The memory folders of the accounts (`(profile id, config dir)`, in order) that hold at least one
/// Markdown file. A folder two accounts lead to (a link, a shared config dir) is listed once, with
/// both accounts.
pub fn find_memory_folders<'a>(accounts: impl IntoIterator<Item = (&'a str, &'a Path)>) -> Vec<FolderSpec> {
    let mut found: Vec<FolderSpec> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (profile_id, config_dir) in accounts {
        let projects = config_dir.join("projects");
        let Ok(entries) = fs::read_dir(&projects) else { continue };
        let mut keys: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        keys.sort();
        for key in keys {
            let dir = projects.join(&key).join("memory");
            let files = memory_files(&dir);
            if files.is_empty() {
                continue;
            }
            let id = folder_id(&real_of(&dir));
            if let Some(&known) = at.get(&id) {
                found[known].profile_ids.push(profile_id.to_owned());
                continue;
            }
            at.insert(id.clone(), found.len());
            let updated_at = files.iter().map(|f| f.mtime_ms).max().unwrap_or(0);
            found.push(FolderSpec { id, profile_ids: vec![profile_id.to_owned()], key, dir, files: files.into_iter().map(|f| f.name).collect(), updated_at });
        }
    }
    found
}

/// `^[0-9a-f-]{36}\.jsonl$` in any case: a conversation file of Claude Code.
pub(crate) fn is_conversation(name: &str) -> bool {
    name.len() == 42
        && name.is_char_boundary(36)
        && name[..36].bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        && name[36..].eq_ignore_ascii_case(".jsonl")
}

/// The conversation files of a project folder with their times, newest first.
pub(crate) fn conversations_newest_first(project_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(project_dir) else { return Vec::new() };
    let mut stamped: Vec<(PathBuf, i64)> = Vec::new();
    for entry in entries.flatten() {
        if !is_conversation(&entry.file_name().to_string_lossy()) {
            continue;
        }
        // gone since it was listed
        let Ok(meta) = fs::metadata(entry.path()) else { continue };
        stamped.push((entry.path(), epoch_ms(meta.modified())));
    }
    stamped.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));
    stamped.into_iter().map(|(file, _)| file).collect()
}

/// The folder a conversation names near its start; None for one that is locked, gone or says none.
pub(crate) fn cwd_said_by(file: &Path) -> Option<String> {
    read_head(file).ok()?.cwd.filter(|cwd| !cwd.is_empty())
}

/// The folder one of the newest conversations under a project's name was held in: a few of the
/// newest are enough, one of them names its folder near its start.
fn cwd_of_conversations(project_dir: &Path) -> Option<String> {
    conversations_newest_first(project_dir).iter().take(3).find_map(|file| cwd_said_by(file))
}

/// Which folders the names under `projects` stand for. A name does not tell (every other
/// character became a hyphen), so the folder is taken from a conversation that is filed under the
/// same name, or from a folder the app knows whose memory goes by that name. What was found is
/// kept; what was not is looked for again next time, as a folder may be known by then.
#[derive(Debug, Default)]
pub struct ProjectFolders {
    known: Mutex<HashMap<String, String>>,
}

impl ProjectFolders {
    pub fn new() -> Self {
        Self::default()
    }

    /// `project_dirs`: every account's `projects/<key>` for that key. `candidates`: folders worth trying.
    pub fn cwd_of<S: AsRef<str>>(&self, key: &str, project_dirs: &[PathBuf], candidates: &[S]) -> Option<String> {
        if let Some(cwd) = lock(&self.known).get(key) {
            return Some(cwd.clone());
        }
        // a conversation is filed under the folder it ran in; that is the project when the names agree
        let mut cwd = project_dirs.iter().filter_map(|dir| cwd_of_conversations(dir)).find(|said| sanitize_path(said) == key);
        if cwd.is_none() {
            cwd = candidates
                .iter()
                .filter_map(|candidate| memory_key(Path::new(candidate.as_ref())).ok())
                .find(|named| named.key == key)
                .map(|named| named.root.to_string_lossy().into_owned());
        }
        if let Some(found) = &cwd {
            lock(&self.known).insert(key.to_owned(), found.clone());
        }
        cwd
    }
}
