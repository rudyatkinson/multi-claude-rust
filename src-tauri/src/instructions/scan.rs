//! Where the instruction files of the accounts and the projects lie, and what a session reads of
//! them (`src/main/instructions/scan.ts`).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use mc_core::instructions::{
    chain_to, demand_note, find_imports, fixed_files, join_within, line_count, long_note, memory_lines, missing_note, normalize_path,
    path_key, rule_paths, rules_dir, scope_id,
};
use mc_core::text_file::decode;
use mc_core::types::{
    InstructionFile, InstructionKind, InstructionScope, InstructionScopeKind, LoadStep, LoadWhen, INSTRUCTION_LIMITS, MEMORY_INDEX,
    SKILL_LIMITS,
};

const WINDOWS: bool = cfg!(windows);

/// A scope as the service keeps it: with its folder resolved, which is what the id stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSpec {
    pub scope: InstructionScope,
    /// The folder with links resolved.
    pub real: PathBuf,
}

/// An account, as far as its instruction files go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionAccount {
    pub id: String,
    /// `effectiveConfigDir`: its CLAUDE_CONFIG_DIR, `~/.claude` for Main.
    pub config_dir: PathBuf,
    /// The account's CLAUDE.md is copied from the main account's when it is synced.
    pub synced_claude_md: bool,
}

// ---------- paths ----------

fn key_of(p: &Path) -> String {
    path_key(&p.to_string_lossy(), WINDOWS)
}

/// `path.resolve(p)`: absolute, with `.` and `..` worked out.
pub(crate) fn resolve(p: &Path) -> PathBuf {
    normalize_path(&std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()))
}

/// `C:\x` for the `\\?\C:\x` that `canonicalize` gives on Windows (and `\\server\share` for
/// `\\?\UNC\server\share`); anything else as it is.
fn without_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = p.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            let b = rest.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                return PathBuf::from(rest);
            }
        }
    }
    p
}

/// `realOf`: the path with links resolved, or as `path.resolve` gives it when it is not there.
pub(crate) fn real_of(p: &Path) -> PathBuf {
    fs::canonicalize(p).map(without_verbatim).unwrap_or_else(|_| resolve(p))
}

fn is_dir(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_dir())
}

/// A file of a folder named the way the window names it, with forward slashes.
pub(crate) fn within(dir: &Path, file: &str) -> PathBuf {
    let mut p = dir.to_path_buf();
    for part in file.split('/') {
        p.push(part);
    }
    normalize_path(&p)
}

/// How far up a repository's top is looked for.
const MAX_UP: usize = 40;

/// The repository a `.git` file of a worktree belongs to, or None when the file is of another
/// kind (a submodule).
fn main_repo_of(dot_git: &Path, holder: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(dot_git).ok()?;
    let gitdir = text.lines().find_map(|line| line.strip_prefix("gitdir:")).map(str::trim).filter(|g| !g.is_empty())?;
    let linked = normalize_path(&holder.join(gitdir));
    let common = fs::read_to_string(linked.join("commondir")).ok()?;
    let common = common.trim();
    if common.is_empty() {
        return None;
    }
    let common_dir = normalize_path(&linked.join(common));
    let is_git = common_dir.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == ".git");
    if is_git {
        common_dir.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// `gitRootOf` of `src/main/memory/projectKey.ts`, kept here so that this package stands on its
/// own: the top of the git repository a folder lies in, for a worktree the top of the repository
/// it was added to. None outside a repository.
pub(crate) fn git_root_of(folder: &Path) -> Option<PathBuf> {
    let mut at = resolve(folder);
    for _ in 0..MAX_UP {
        let dot_git = at.join(".git");
        match fs::metadata(&dot_git) {
            Ok(meta) if meta.is_dir() => return Some(at),
            Ok(meta) if meta.is_file() => return Some(main_repo_of(&dot_git, &at).unwrap_or(at)),
            _ => {}
        }
        at = at.parent()?.to_path_buf();
    }
    None
}

// ---------- the scopes ----------

fn add_project(folder: &Path, scopes: &mut Vec<ScopeSpec>, seen: &mut HashSet<String>) {
    let real = real_of(folder);
    // an account's own folder is no project
    if seen.contains(&key_of(&real)) || !is_dir(folder) {
        return;
    }
    seen.insert(key_of(&real));
    let scope = InstructionScope {
        id: scope_id('f', &real.to_string_lossy(), WINDOWS),
        kind: InstructionScopeKind::Project,
        profile_id: None,
        dir: resolve(folder).to_string_lossy().into_owned(),
        synced_claude_md: None,
    };
    scopes.push(ScopeSpec { scope, real });
}

/// The accounts, and the project folders in sight: each folder the app knows and the top of its
/// repository, where a project's CLAUDE.md usually lies. Folders that are not absolute or not
/// there are left out; a folder is listed once, however it is spelled.
pub fn find_scopes(accounts: &[InstructionAccount], folders: &[String]) -> Vec<ScopeSpec> {
    let mut scopes: Vec<ScopeSpec> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for account in accounts {
        let real = real_of(&account.config_dir);
        if !seen.insert(key_of(&real)) {
            continue;
        }
        let scope = InstructionScope {
            id: scope_id('a', &real.to_string_lossy(), WINDOWS),
            kind: InstructionScopeKind::Account,
            profile_id: Some(account.id.clone()),
            dir: account.config_dir.to_string_lossy().into_owned(),
            synced_claude_md: account.synced_claude_md.then_some(true),
        };
        scopes.push(ScopeSpec { scope, real });
    }
    for folder in folders {
        let folder = Path::new(folder);
        if !folder.is_absolute() {
            continue;
        }
        if let Some(root) = git_root_of(folder) {
            add_project(&root, &mut scopes, &mut seen);
        }
        add_project(folder, &mut scopes, &mut seen);
    }
    scopes
}

// ---------- the files of a scope ----------

const SKIP_DIRS: [&str; 2] = ["node_modules", ".git"];
const MAX_DEPTH: usize = 6;
const MAX_RULES: usize = 300;

/// Where a character stands in the default collation, roughly as `localeCompare` orders file
/// names: spaces, then punctuation and symbols in ICU's order, then digits, then letters without
/// regard to case, then everything else.
fn primary(c: char) -> (u8, u32) {
    const SYMBOLS: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    if c.is_whitespace() {
        return (0, c as u32);
    }
    if let Some(at) = SYMBOLS.find(c) {
        return (1, u32::try_from(at).unwrap_or(u32::MAX));
    }
    if c.is_ascii_digit() {
        return (2, c as u32);
    }
    if c.is_alphabetic() {
        return (3, c.to_lowercase().next().unwrap_or(c) as u32);
    }
    (4, c as u32)
}

/// `a.localeCompare(b)` for file names, close enough for ordering a list: letters without regard
/// to case first, then lower case before upper case.
fn locale_compare(a: &str, b: &str) -> Ordering {
    a.chars()
        .map(primary)
        .cmp(b.chars().map(primary))
        .then_with(|| a.chars().map(char::is_uppercase).cmp(b.chars().map(char::is_uppercase)))
        .then_with(|| a.cmp(b))
}

fn is_markdown(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 3 && b[b.len() - 3..].eq_ignore_ascii_case(b".md")
}

fn visit_rules(dir: &Path, sub: &str, depth: usize, found: &mut Vec<String>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    // links count as neither files nor folders, as a Dirent of Node has it
    let mut entries: Vec<(String, bool, bool)> = read
        .filter_map(Result::ok)
        .map(|entry| {
            let kind = entry.file_type().ok();
            (entry.file_name().to_string_lossy().into_owned(), kind.is_some_and(|k| k.is_dir()), kind.is_some_and(|k| k.is_file()))
        })
        .collect();
    // files before folders
    entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| locale_compare(&a.0, &b.0)));
    for (name, folder, file) in entries {
        if found.len() >= MAX_RULES {
            return;
        }
        let rel = if sub.is_empty() { name.clone() } else { format!("{sub}/{name}") };
        if folder {
            if !SKIP_DIRS.contains(&name.as_str()) && depth < MAX_DEPTH {
                visit_rules(&dir.join(&name), &rel, depth + 1, found);
            }
        } else if file && is_markdown(&name) {
            found.push(rel);
        }
    }
}

/// The Markdown files under a rules folder, at any depth up to six folders down (at most 300),
/// with forward slashes; within a folder its files come before its folders.
pub fn find_rules(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    visit_rules(dir, "", 0, &mut found);
    found
}

/// The text of a file, or None when it is not there. A file this app does not read as text
/// (too large, UTF-16, not UTF-8) is there with no text.
pub(crate) fn text_of(file: &Path) -> Option<String> {
    let bytes = fs::read(file).ok()?;
    Some(decode(&bytes, SKILL_LIMITS.file_bytes).map(|d| d.text).unwrap_or_default())
}

/// The files of a scope: the fixed ones whether they are there or not, then its rules.
pub fn list_files(kind: InstructionScopeKind, dir: &Path) -> Vec<InstructionFile> {
    let mut files: Vec<InstructionFile> = Vec::new();
    for fixed in fixed_files(kind) {
        let text = text_of(&within(dir, fixed.file));
        let body = text.as_deref().unwrap_or("");
        files.push(InstructionFile {
            file: fixed.file.to_owned(),
            kind: fixed.kind,
            exists: text.is_some(),
            lines: line_count(body),
            paths: Vec::new(),
            imports: find_imports(body),
        });
    }
    let rules = rules_dir(kind);
    let rules_at = within(dir, rules);
    for rule in find_rules(&rules_at) {
        let text = text_of(&within(&rules_at, &rule)).unwrap_or_default();
        files.push(InstructionFile {
            file: format!("{rules}/{rule}"),
            kind: InstructionKind::Rule,
            exists: true,
            lines: line_count(&text),
            paths: rule_paths(&text),
            imports: find_imports(&text),
        });
    }
    files
}

// ---------- what a session reads ----------

/// What [`load_order`] needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadInputs {
    /// The account's config folder.
    pub config_dir: PathBuf,
    /// The folder the session runs in.
    pub cwd: PathBuf,
    /// Where `@~/...` imports lead.
    pub home_dir: PathBuf,
    /// The organisation's own CLAUDE.md, when there is a place for one on this system.
    pub managed_file: Option<PathBuf>,
    /// The memory folder the session reads MEMORY.md from, when it has one.
    pub memory_dir: Option<PathBuf>,
    /// The folder above which no project files are looked for; the top of the drive when None.
    pub top: Option<PathBuf>,
}

struct Loader<'a> {
    home: &'a Path,
    steps: Vec<LoadStep>,
    /// The files read so far, by their resolved path: each is read once, and imports in a circle stop.
    read: HashSet<String>,
}

impl Loader<'_> {
    fn add(&mut self, file: &Path, role: &str, depth: u32, when: LoadWhen) {
        let real = key_of(&real_of(file));
        if self.read.contains(&real) {
            return;
        }
        let Some(text) = text_of(file) else { return };
        self.read.insert(real);
        let lines = line_count(&text);
        let long = role != "Import" && role != "Rule" && lines as usize > INSTRUCTION_LIMITS.lines;
        self.steps.push(LoadStep {
            path: file.to_string_lossy().into_owned(),
            role: role.to_owned(),
            when,
            lines,
            depth,
            note: long.then(long_note),
        });
        if depth as usize >= INSTRUCTION_LIMITS.import_hops {
            return;
        }
        for target in find_imports(&text) {
            let resolved = match target.strip_prefix('~') {
                Some(rest) => join_within(self.home, rest),
                None => normalize_path(&file.parent().unwrap_or(file).join(&target)),
            };
            let before = self.steps.len();
            self.add(&resolved, "Import", depth + 1, when);
            if self.steps.len() == before && !self.read.contains(&key_of(&real_of(&resolved))) {
                self.steps.push(LoadStep {
                    path: resolved.to_string_lossy().into_owned(),
                    role: "Import".to_owned(),
                    when: LoadWhen::Missing,
                    lines: 0,
                    depth: depth + 1,
                    note: Some(missing_note(&target)),
                });
            }
        }
    }

    /// A rule with `paths` is read later, when Claude works on a matching file.
    fn add_rules(&mut self, dir: &Path) {
        for rule in find_rules(dir) {
            let file = within(dir, &rule);
            let paths = rule_paths(&text_of(&file).unwrap_or_default());
            self.add(&file, "Rule", 0, if paths.is_empty() { LoadWhen::Start } else { LoadWhen::Demand });
            if !paths.is_empty() {
                let shown = file.to_string_lossy();
                if let Some(step) = self.steps.iter_mut().find(|s| s.path == shown) {
                    step.note = Some(demand_note(&paths));
                }
            }
        }
    }
}

/// The instruction files a session of an account reads in a folder, in the order Claude Code
/// reads them: the organisation's file, the account's CLAUDE.md and rules, then for every folder
/// from the top of the drive down to the session's folder its CLAUDE.md, `.claude/CLAUDE.md`,
/// rules and `CLAUDE.local.md`, each followed by what it imports, and MEMORY.md last. A rule with
/// `paths` is read later, when Claude works on a matching file. This is a model of what Claude
/// Code does; `/context` in a session shows what it really read.
pub fn load_order(inputs: &LoadInputs) -> Vec<LoadStep> {
    let mut loader = Loader { home: &inputs.home_dir, steps: Vec::new(), read: HashSet::new() };
    if let Some(managed) = &inputs.managed_file {
        loader.add(&normalize_path(managed), "Managed policy", 0, LoadWhen::Start);
    }
    loader.add(&within(&inputs.config_dir, "CLAUDE.md"), "Account", 0, LoadWhen::Start);
    loader.add_rules(&within(&inputs.config_dir, "rules"));
    let top = inputs.top.as_deref().map(resolve);
    for folder in chain_to(&resolve(&inputs.cwd), top.as_deref(), WINDOWS) {
        loader.add(&folder.join("CLAUDE.md"), "Project", 0, LoadWhen::Start);
        loader.add(&within(&folder, ".claude/CLAUDE.md"), "Project", 0, LoadWhen::Start);
        loader.add_rules(&within(&folder, ".claude/rules"));
        loader.add(&folder.join("CLAUDE.local.md"), "Local", 0, LoadWhen::Start);
    }
    let mut steps = loader.steps;
    if let Some(dir) = &inputs.memory_dir {
        let index = within(dir, MEMORY_INDEX);
        if let Some(text) = text_of(&index) {
            let (lines, note) = memory_lines(line_count(&text));
            steps.push(LoadStep { path: index.to_string_lossy().into_owned(), role: "Memory".to_owned(), when: LoadWhen::Start, lines, depth: 0, note });
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_names_as_locale_compare_does() {
        let mut names = vec!["b.md", "B2.md", "a_b.md", "a.md", "a-b.md", "A.md", "10.md", "9.md"];
        names.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(names, ["10.md", "9.md", "a_b.md", "a-b.md", "a.md", "A.md", "b.md", "B2.md"]);
    }

    #[test]
    fn markdown_files_by_their_extension() {
        assert!(is_markdown("a.md") && is_markdown("A.MD") && is_markdown(".md"));
        assert!(!is_markdown("md") && !is_markdown("a.txt") && !is_markdown("ü"));
    }

    #[test]
    #[cfg(windows)]
    fn verbatim_paths_lose_their_prefix() {
        assert_eq!(without_verbatim(PathBuf::from(r"\\?\C:\x")), PathBuf::from(r"C:\x"));
        assert_eq!(without_verbatim(PathBuf::from(r"\\?\UNC\srv\share\x")), PathBuf::from(r"\\srv\share\x"));
        assert_eq!(without_verbatim(PathBuf::from(r"\\?\Volume{1}\x")), PathBuf::from(r"\\?\Volume{1}\x"));
    }
}
