//! Sharing one account's project memory with the others (`src/main/memory/share.ts`). Nothing is
//! linked or moved on disk: a session of a sharing account is started with `autoMemoryDirectory`
//! in its settings file, which Claude Code takes in place of the account's own
//! `projects/<project>/memory`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mc_core::error::{AppError, AppResult};
use mc_core::memory::{
    add_index_line, adopt_slug, entry_named, memory_key, memory_meta, parse_index, plan_adopt, same_name, sanitize_path, stem_of, KEY_MAX,
};
use mc_core::text_file::{content_hash, create_text, decode, encode, write_file_atomic, Eol};
use mc_core::types::{AdoptFile, AdoptResult, MEMORY_INDEX, SKILL_LIMITS};

use super::folders::{conversations_newest_first, cwd_said_by, memory_files};
use crate::state::lock;

/// How many project folders are looked at to see whether Claude Code still names them as this app does.
const SAMPLES: usize = 6;

/// How long a look at the names is good for.
pub const CHECK_MS: i64 = 5 * 60 * 1000;

/// Epoch ms; replaced in tests.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The names in a folder, sorted; none when it cannot be read.
pub(crate) fn names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// Whether the names under an account's `projects` still follow the rule this app knows: the
/// folder a conversation ran in must give the name it is filed under. A few are looked at (the
/// newest conversation of each), and one that agrees is enough: a single folder can hold a
/// conversation that was begun elsewhere, but a changed rule leaves none that agree. None when the
/// rule holds or there is nothing to compare; else what was found.
pub fn naming_problem(config_dir: &Path) -> Option<String> {
    let projects = config_dir.join("projects");
    let mut checked = 0usize;
    let mut odd: Option<String> = None;
    // long names carry a hash that is not reproduced here; they say nothing about the rule
    for name in names_in(&projects).into_iter().filter(|n| mc_core::jsstr::len16(n) < KEY_MAX) {
        if checked >= SAMPLES {
            break;
        }
        let Some(cwd) = conversations_newest_first(&projects.join(&name)).first().and_then(|file| cwd_said_by(file)) else { continue };
        checked += 1;
        if same_name(&sanitize_path(&cwd), &name) {
            return None;
        }
        odd.get_or_insert_with(|| {
            format!("Claude Code files the conversations of \"{cwd}\" under \"{name}\", which is not the name this app works out for that folder.")
        });
    }
    odd
}

/// The memory folder of the main account for sessions that run in `cwd`: what a sharing account
/// is pointed to. The folder need not exist yet; Claude Code makes it with the first memory. When
/// the project's folder is there, it is spelled as on disk. `problem` is what [`naming_problem`]
/// found, which rules sharing out. The error is a sentence for the user.
pub fn shared_memory_dir(main_config_dir: &Path, cwd: &Path, problem: Option<&str>) -> Result<PathBuf, String> {
    if let Some(problem) = problem {
        return Err(problem.to_owned());
    }
    let named = memory_key(cwd)?;
    let projects = main_config_dir.join("projects");
    let names = names_in(&projects);
    let folder = entry_named(&names, &named.key).unwrap_or(&named.key);
    Ok(projects.join(folder).join("memory"))
}

struct Checked {
    dir: PathBuf,
    at: i64,
    problem: Option<String>,
}

/// Answers where a sharing account's session keeps its memory. A session is started in the
/// middle of other work, so the look at the names is kept for [`CHECK_MS`].
pub struct MemorySharing {
    clock: Clock,
    checked: Mutex<Option<Checked>>,
}

impl MemorySharing {
    pub fn new(clock: Clock) -> Self {
        MemorySharing { clock, checked: Mutex::new(None) }
    }

    /// What rules sharing out altogether, or None. Looked at again when `fresh`, after
    /// [`CHECK_MS`], or for another main config dir.
    pub fn problem(&self, main_config_dir: &Path, fresh: bool) -> Option<String> {
        let now = (self.clock)();
        let mut checked = lock(&self.checked);
        let stale = fresh || checked.as_ref().is_none_or(|c| c.dir != main_config_dir || now - c.at > CHECK_MS);
        if stale {
            *checked = Some(Checked { dir: main_config_dir.to_path_buf(), at: now, problem: naming_problem(main_config_dir) });
        }
        checked.as_ref().and_then(|c| c.problem.clone())
    }

    /// [`shared_memory_dir`] with what [`MemorySharing::problem`] says.
    pub fn dir_for(&self, main_config_dir: &Path, cwd: &Path) -> Result<PathBuf, String> {
        let problem = self.problem(main_config_dir, false);
        shared_memory_dir(main_config_dir, cwd, problem.as_deref())
    }
}

// ---------- bringing an account's own memories into the shared folder ----------

/// The Markdown files of a folder with a hash of their text: the same text with other line ends
/// or a byte order mark is the same memory. A file that is not text is hashed as its bytes.
fn hashed(dir: &Path) -> Vec<AdoptFile> {
    let mut files = Vec::new();
    for file in memory_files(dir) {
        // gone since it was listed
        let Ok(bytes) = fs::read(dir.join(&file.name)) else { continue };
        let hash = match decode(&bytes, SKILL_LIMITS.file_bytes) {
            Ok(decoded) => content_hash(decoded.text.as_bytes()),
            Err(_) => content_hash(&bytes),
        };
        files.push(AdoptFile { name: file.name, hash });
    }
    files
}

/// The text of a file; empty when it is not there or not text this app reads.
fn text_or_empty(bytes: &[u8]) -> String {
    decode(bytes, SKILL_LIMITS.file_bytes).map(|d| d.text).unwrap_or_default()
}

/// Copies the memories of `source` into `target` so that nothing is lost on either side (see
/// [`plan_adopt`]), and gives the target's index a line for each file that arrived, after its last
/// entry and with its line ends and byte order mark. The source is left as it is. Files are
/// made, never written over. Without `apply` nothing is written and the result says what would
/// happen. `slug` and `name` are those of the account the memories come from.
pub fn adopt_memories(source: &Path, target: &Path, slug: &str, name: &str, apply: bool) -> AppResult<AdoptResult> {
    let plan = plan_adopt(&hashed(source), &hashed(target), &adopt_slug(slug));
    let result = |applied: bool, plan: mc_core::types::AdoptPlan| AdoptResult {
        copy: plan.copy,
        same: plan.same,
        target: target.to_string_lossy().into_owned(),
        applied,
    };
    if !apply || plan.copy.is_empty() {
        return Ok(result(apply, plan));
    }

    let source_index = fs::read(source.join(MEMORY_INDEX)).map(|bytes| parse_index(&text_or_empty(&bytes))).unwrap_or_default();
    let mut added: Vec<(String, String, String)> = Vec::new();
    for copy in &plan.copy {
        let bytes = fs::read(source.join(&copy.from))?;
        create_text(&target.join(&copy.to), &bytes)?;
        let line = source_index.iter().find(|entry| same_name(&entry.file, &copy.from));
        let meta = memory_meta(&text_or_empty(&bytes));
        let title = [line.map(|l| l.title.as_str()).unwrap_or(""), meta.name.as_str()]
            .into_iter()
            .find(|t| !t.is_empty())
            .unwrap_or_else(|| stem_of(&copy.from))
            .to_owned();
        // a file that had to take another name says whose it was
        let title = if copy.to == copy.from { title } else { format!("{title} (from {name})") };
        let hook = line.map(|l| l.hook.clone()).filter(|h| !h.is_empty()).unwrap_or(meta.description);
        added.push((title, copy.to.clone(), hook));
    }

    let index_file = target.join(MEMORY_INDEX);
    let existing = match fs::read(&index_file) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let (text, eol, bom) = match &existing {
        None => (String::new(), Eol::Lf, false),
        Some(bytes) => match decode(bytes, SKILL_LIMITS.file_bytes) {
            Ok(d) => (d.text, d.eol, d.bom),
            Err(why) => return Err(AppError::io(format!("The files were copied, but MEMORY.md of the shared folder cannot be changed: {why}"))),
        },
    };
    let next = added.iter().fold(text, |index, (title, file, hook)| add_index_line(&index, title, file, hook));
    match existing {
        None => create_text(&index_file, next)?,
        Some(_) => write_file_atomic(&index_file, &encode(&next, eol, bom))?,
    }
    Ok(result(true, plan))
}
