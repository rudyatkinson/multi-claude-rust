//! `src/main/memory/projectKey.ts`: how Claude Code names a project's folder under
//! `<config>/projects`, and which folder a session's auto memory belongs to. Both follow what
//! Claude Code 2.1 does itself:
//!
//! - the name is the path with a hyphen for every character that is not a letter or a digit of
//!   plain ASCII (for every UTF-16 unit, as JavaScript replaces); a name longer than 200
//!   characters is cut and gets a hash, which is not reproduced here (such a project is simply
//!   not served);
//! - conversations are filed under the folder the session runs in, but memory under the top of
//!   its git repository, and for a worktree under the top of the repository it belongs to, so
//!   that every folder and worktree of one repository shares one memory.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::usage::js_trim;

/// The longest project name Claude Code writes as it is.
pub const KEY_MAX: usize = 200;

/// How far up a repository is looked for.
const MAX_UP: usize = 40;

/// `^gitdir:\s*(.+?)\s*$`, on any line.
static GITDIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^gitdir:\s*(.+?)\s*$").expect("valid regex"));

/// `p.replace(/[^a-zA-Z0-9]/g, '-')`: a hyphen for every UTF-16 unit that is not a plain letter or
/// digit (so two for a character outside the BMP).
pub fn sanitize_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                out.push('-');
            }
        }
    }
    out
}

/// `.` and `..` taken out of a path, as `path.resolve` takes them out; `..` at the root stays there.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            Component::Normal(name) => out.push(name),
        }
    }
    out
}

/// `path.isAbsolute`: on Windows a path that starts at the root of the current drive counts too.
fn is_absolute(p: &Path) -> bool {
    p.is_absolute() || p.has_root()
}

/// `path.resolve(base, p)`.
fn resolve_from(base: &Path, p: &Path) -> PathBuf {
    normalize(&base.join(p))
}

/// `path.resolve(p)`: absolute (against the current folder) and without `.` and `..`.
pub fn resolve_path(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return normalize(p);
    }
    match std::env::current_dir() {
        Ok(cwd) => resolve_from(&cwd, p),
        Err(_) => normalize(p),
    }
}

fn read_lossy(file: &Path) -> Option<String> {
    fs::read(file).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The repository a `.git` file of a worktree belongs to, or None when the file is of another
/// kind (a submodule).
fn main_repo_of(dot_git: &Path, holder: &Path) -> Option<PathBuf> {
    let text = read_lossy(dot_git)?;
    let gitdir = GITDIR.captures(&text)?.get(1)?.as_str().to_owned();
    let linked = resolve_from(holder, Path::new(&gitdir));
    let common = read_lossy(&linked.join("commondir"))?;
    let common = js_trim(&common);
    if common.is_empty() {
        return None;
    }
    let common_dir = resolve_from(&linked, Path::new(common));
    let named_git = common_dir.file_name().is_some_and(|name| name.to_string_lossy().to_lowercase() == ".git");
    if named_git {
        common_dir.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// The top of the git repository a folder lies in, as memory goes by it: for a worktree the top
/// of the repository it was added to, for a submodule the submodule itself. None outside a
/// repository (looked for up to 40 folders up).
pub fn git_root_of(folder: &Path) -> Option<PathBuf> {
    let mut at = resolve_path(folder);
    for _ in 0..MAX_UP {
        let dot_git = at.join(".git");
        match fs::metadata(&dot_git) {
            Ok(meta) if meta.is_dir() => return Some(at),
            Ok(meta) if meta.is_file() => return Some(main_repo_of(&dot_git, &at).unwrap_or(at)),
            _ => {}
        }
        let up = at.parent()?.to_path_buf();
        if up == at {
            return None;
        }
        at = up;
    }
    None
}

/// The name of the folder under `projects` that holds the memory of sessions run in a folder,
/// and the folder it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryKey {
    pub key: String,
    /// The top of the repository, or the folder itself outside one.
    pub root: PathBuf,
}

/// `memoryKeyOf`: the memory key of sessions run in `cwd`, or why there is none (a sentence for
/// the user): the folder is not absolute, or its name is one Claude Code would cut.
pub fn memory_key(cwd: &Path) -> Result<MemoryKey, String> {
    if !is_absolute(cwd) {
        return Err("The folder is not an absolute path.".to_owned());
    }
    let root = git_root_of(cwd).unwrap_or_else(|| resolve_path(cwd));
    let key = sanitize_path(&root.to_string_lossy());
    // all ASCII: bytes are UTF-16 units
    if key.len() > KEY_MAX {
        return Err(format!(
            "The path of this project is longer than {KEY_MAX} characters, and Claude Code names such a project in a way this app does not reproduce."
        ));
    }
    Ok(MemoryKey { key, root })
}

/// Just the key of [`memory_key`]; None when it is refused.
pub fn memory_key_of(cwd: &Path) -> Option<String> {
    memory_key(cwd).ok().map(|k| k.key)
}
