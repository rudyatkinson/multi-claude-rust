//! The agents of a folder and what the list shows of each (`src/main/agents/scan.ts`).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs::{self, FileType};
use std::path::{Path, PathBuf};

use mc_core::agents::{agent_loads, collapse_spaces, inspect_agent};
use mc_core::frontmatter::{field_text, first_body_line, name_of, parse_doc};
use mc_core::jsstr::cut16;
use mc_core::text_file::decode;
use mc_core::types::{AgentIssue, AgentSummary, IssueLevel, SKILL_LIMITS};

/// What a listing shows of a description.
const DESCRIPTION_CHARS: usize = 400;
const SKIP_DIRS: [&str; 2] = ["node_modules", "__pycache__"];
const MAX_DEPTH: usize = 6;
/// A folder lists at most this many agents.
pub const MAX_FILES: usize = 500;

/// A file within a folder, named with forward slashes, as a path.
pub(super) fn join_rel(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |path, part| path.join(part))
}

/// `a.localeCompare(b)`, near enough for file names: letter case only counts between names that
/// are the same otherwise, and then the lower-case one comes first.
fn locale_compare(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

/// Files first, links last: a folder is listed where it really lies.
fn rank(kind: &FileType) -> u8 {
    if kind.is_symlink() {
        2
    } else if kind.is_dir() {
        1
    } else {
        0
    }
}

/// `/\.md$/i`
fn is_markdown(name: &str) -> bool {
    name.len() >= 3 && name.get(name.len() - 3..).is_some_and(|end| end.eq_ignore_ascii_case(".md"))
}

/// The Markdown files of an agents folder, at any depth, as Claude Code reads them: their paths
/// within the folder, with forward slashes. Folders whose name starts with a dot are left out, and
/// a folder that is reached a second time through a link is not walked again. A name that is not
/// valid Unicode is left out too, since the window could not name it back.
pub fn find_agents(root_dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut walked = HashSet::new();
    visit(root_dir, "", 0, &mut found, &mut walked);
    found
}

fn visit(root: &Path, sub: &str, depth: usize, found: &mut Vec<String>, walked: &mut HashSet<String>) {
    let dir = if sub.is_empty() { root.to_path_buf() } else { join_rel(root, sub) };
    let Ok(real) = fs::canonicalize(&dir) else { return };
    if !walked.insert(real.to_string_lossy().to_lowercase()) {
        return;
    }
    let Ok(read) = fs::read_dir(&dir) else { return };
    let mut entries: Vec<(String, FileType)> = read
        .filter_map(|entry| {
            let entry = entry.ok()?;
            Some((entry.file_name().into_string().ok()?, entry.file_type().ok()?))
        })
        .collect();
    entries.sort_by(|a, b| rank(&a.1).cmp(&rank(&b.1)).then_with(|| locale_compare(&a.0, &b.0)));
    for (name, kind) in entries {
        if found.len() >= MAX_FILES {
            return;
        }
        let rel = if sub.is_empty() { name.clone() } else { format!("{sub}/{name}") };
        let (mut is_dir, mut is_file) = (kind.is_dir(), kind.is_file());
        if kind.is_symlink() {
            match fs::metadata(dir.join(&name)) {
                Ok(target) => {
                    is_dir = target.is_dir();
                    is_file = target.is_file();
                }
                // a link that leads nowhere
                Err(_) => continue,
            }
        }
        if is_dir {
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) && depth < MAX_DEPTH {
                visit(root, &rel, depth + 1, found, walked);
            }
        } else if is_file && is_markdown(&name) {
            found.push(rel);
        }
    }
}

fn unreadable(base: AgentSummary, message: String) -> AgentSummary {
    AgentSummary { loads: false, issues: vec![AgentIssue { level: IssueLevel::Error, message, clash: false }], ..base }
}

/// What the list shows of one agent. A file that cannot be read still gets its line.
pub fn summarize(root_id: &str, root_dir: &Path, file: &str) -> AgentSummary {
    let base = AgentSummary {
        root_id: root_id.to_owned(),
        file: file.to_owned(),
        name: String::new(),
        description: String::new(),
        model: String::new(),
        effort: String::new(),
        loads: false,
        issues: Vec::new(),
    };
    let bytes = match fs::read(join_rel(root_dir, file)) {
        Ok(bytes) => bytes,
        Err(err) => return unreadable(base, format!("The file cannot be read: {err}")),
    };
    let decoded = match decode(&bytes, SKILL_LIMITS.file_bytes) {
        Ok(decoded) => decoded,
        Err(why) => return unreadable(base, why.0),
    };
    let doc = parse_doc(&decoded.text);
    let issues = inspect_agent(&doc);
    let trimmed = |key: &str| field_text(&doc, key, false).trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}').to_owned();
    let description = match trimmed("description") {
        given if !given.is_empty() => given,
        _ => first_body_line(doc.body),
    };
    AgentSummary {
        name: name_of(&doc).unwrap_or_default(),
        description: cut16(&collapse_spaces(&description), DESCRIPTION_CHARS).to_owned(),
        model: trimmed("model"),
        effort: trimmed("effort"),
        loads: agent_loads(&issues),
        issues,
        ..base
    }
}
