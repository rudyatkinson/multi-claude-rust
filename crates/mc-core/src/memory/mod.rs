//! Auto-memory rules: index lines, inspection, meta, adopt plan (`src/shared/memory.ts`) and
//! project keys (`src/main/memory/projectKey.ts`, in `project_key.rs`).
//!
//! Claude Code's auto memory as the Library sees it: per account and project a folder
//! `<config>/projects/<project>/memory` with a `MEMORY.md` (the index every session reads) and one
//! Markdown file per thing that was written down.
//!
//! Lengths and offsets that the TypeScript counted with `string.length` are UTF-16 units here too
//! (see [`crate::jsstr`]).

mod project_key;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use yaml_rust2::{Yaml, YamlLoader};

use crate::conversations::fold_text;
use crate::frontmatter::{frontmatter_block, split_doc, YamlValue};
use crate::jsstr::len16;
use crate::skills::kebab_problem;
use crate::types::{AdoptCopy, AdoptFile, AdoptPlan, IndexLine, MemoryIssue, MemoryIssueLevel, MemoryType, MEMORY_INDEX, MEMORY_LIMITS};
use crate::usage::{is_js_space, js_trim};

pub use project_key::{git_root_of, memory_key, memory_key_of, resolve_path, sanitize_path, MemoryKey, KEY_MAX};

/// How many UTF-16 units [`memory_snippet`] shows on either side of what was found, by default.
pub const SNIPPET_AROUND: usize = 70;

/// The file is the index (`MEMORY.md`, in any case).
pub fn is_index(file: &str) -> bool {
    same_name(file, MEMORY_INDEX)
}

/// `a.toLowerCase() === b.toLowerCase()`.
pub fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// `entryNamed`: the name among `names` that is `name` in any case, as it is spelled there.
pub fn entry_named<'a, S: AsRef<str>>(names: &'a [S], name: &str) -> Option<&'a str> {
    names.iter().map(AsRef::as_ref).find(|n| same_name(n, name))
}

/// `text.replace(/\s+/g, ' ').trim()`: one line, with single spaces.
pub fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_js_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    js_trim(&out).to_owned()
}

/// `name.replace(/\.md$/i, '')`.
pub fn stem_of(file: &str) -> &str {
    let cut = file.len().checked_sub(3).filter(|&at| file.is_char_boundary(at) && file[at..].eq_ignore_ascii_case(".md"));
    cut.map_or(file, |at| &file[..at])
}

/// `/\.md$/i`.
pub fn is_markdown(file: &str) -> bool {
    stem_of(file).len() < file.len()
}

/// The lines of a text as MEMORY.md counts them: none for an empty text, and a last line break
/// ends the last line.
pub fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.strip_suffix('\n').unwrap_or(text).split('\n').count()
    }
}

/// The words of a search: folded as [`fold_text`] folds, split at white space.
pub fn search_words(query: &str) -> Vec<String> {
    fold_text(query).split(is_js_space).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

// ---------- the index ----------

/// `\s` of JavaScript, as the body of a character class.
const JS_SPACE: &str = r"\t\n\x0B\x0C\r \xA0\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// `- [Title](file.md) — hook`:
/// `^\s*[-*+]\s+\[([^\]]*)\]\(([^)\s]+)\)\s*(?:[—–:-]+\s*)?(.*)$`, with JavaScript's `\s` and `.`.
static INDEX_LINE: LazyLock<Regex> = LazyLock::new(|| {
    let s = format!("[{JS_SPACE}]");
    let pattern =
        format!(r"^{s}*[-*+]{s}+\[([^\]]*)\]\(([^){JS_SPACE}]+)\){s}*(?:[\x{{2014}}\x{{2013}}:-]+{s}*)?([^\n\r\x{{2028}}\x{{2029}}]*)$");
    Regex::new(&pattern).expect("valid regex")
});

/// The file a link of the index points to, when it is a file of the memory folder itself:
/// `^[^/\\:?#]+\.md$` (any case) once a leading `./` is taken off.
fn target_of(link: &str) -> Option<&str> {
    let file = link.strip_prefix("./").unwrap_or(link);
    let ok = is_markdown(file) && !stem_of(file).is_empty() && !file.contains(['/', '\\', ':', '?', '#']);
    ok.then_some(file)
}

/// The lines of MEMORY.md that point to a memory.
pub fn parse_index(text: &str) -> Vec<IndexLine> {
    let mut found = Vec::new();
    for (line, raw) in text.split('\n').enumerate() {
        let Some(m) = INDEX_LINE.captures(raw) else { continue };
        let Some(file) = target_of(&m[2]) else { continue };
        found.push(IndexLine {
            line: u32::try_from(line).unwrap_or(u32::MAX),
            title: js_trim(&m[1]).to_owned(),
            file: file.to_owned(),
            hook: js_trim(&m[3]).to_owned(),
        });
    }
    found
}

/// A line of the index as Claude Code writes them: `- [Title](file.md) — hook`. The title is one
/// line without brackets, the file's name when nothing is left; the hook is left out when empty.
pub fn index_line(title: &str, file: &str, hook: &str) -> String {
    let title = one_line(title).replace(['[', ']'], "");
    let title = if title.is_empty() { stem_of(file).to_owned() } else { title };
    let hook = one_line(hook);
    if hook.is_empty() {
        format!("- [{title}]({file})")
    } else {
        format!("- [{title}]({file}) — {hook}")
    }
}

/// MEMORY.md with a line for another memory: after the last such line, or at the end (after an
/// empty line when the text so far does not end with one). The result ends with a line break.
pub fn add_index_line(text: &str, title: &str, file: &str, hook: &str) -> String {
    let added = index_line(title, file, hook);
    let mut lines: Vec<&str> = if text.is_empty() { Vec::new() } else { text.strip_suffix('\n').unwrap_or(text).split('\n').collect() };
    match parse_index(text).last() {
        Some(last) => {
            let at = (last.line as usize + 1).min(lines.len());
            lines.insert(at, &added);
        }
        None => {
            // a first entry under whatever the file says so far
            if lines.last().is_some_and(|l| !js_trim(l).is_empty()) {
                lines.push("");
            }
            lines.push(&added);
        }
    }
    format!("{}\n", lines.join("\n"))
}

/// MEMORY.md without the lines that point to a file (in any case); the text as it was when none does.
pub fn remove_index_line(text: &str, file: &str) -> String {
    let gone: HashSet<usize> = parse_index(text).into_iter().filter(|e| same_name(&e.file, file)).map(|e| e.line as usize).collect();
    if gone.is_empty() {
        return text.to_owned();
    }
    text.split('\n').enumerate().filter(|(line, _)| !gone.contains(line)).map(|(_, l)| l).collect::<Vec<_>>().join("\n")
}

/// A few names, and how many more there are.
fn some(names: &[&str], shown: usize) -> String {
    if names.len() <= shown {
        names.join(", ")
    } else {
        format!("{} and {} more", names[..shown].join(", "), names.len() - shown)
    }
}

/// MEMORY.md as [`inspect_memory`] looks at it: its text and its size on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexFile<'a> {
    pub text: &'a str,
    pub bytes: u64,
}

fn issue(level: MemoryIssueLevel, message: String) -> MemoryIssue {
    MemoryIssue { level, message }
}

/// What a session does not get to read of MEMORY.md, and what the index and the folder disagree
/// on. `index` is None when the folder has no MEMORY.md; `files` are the memories, MEMORY.md not
/// among them.
pub fn inspect_memory<S: AsRef<str>>(index: Option<IndexFile<'_>>, files: &[S]) -> Vec<MemoryIssue> {
    let mut issues = Vec::new();
    let Some(index) = index else {
        if !files.is_empty() {
            issues.push(issue(MemoryIssueLevel::Warning, "There is no MEMORY.md. A session only knows of memories that are listed there.".to_owned()));
        }
        return issues;
    };
    let lines = line_count(index.text);
    if lines > MEMORY_LIMITS.index_lines {
        issues.push(issue(
            MemoryIssueLevel::Warning,
            format!("MEMORY.md has {lines} lines. A session reads the first {}; what comes after is not seen.", MEMORY_LIMITS.index_lines),
        ));
    } else if index.bytes > MEMORY_LIMITS.index_bytes as u64 {
        // Math.round(bytes / 1000)
        let kb = (index.bytes + 500) / 1000;
        issues.push(issue(MemoryIssueLevel::Warning, format!("MEMORY.md has {kb} KB. A session reads the first 25 KB; what comes after is not seen.")));
    }
    let listed = parse_index(index.text);
    let has = |file: &str| files.iter().any(|f| same_name(f.as_ref(), file));
    let mut missing: Vec<&str> = Vec::new();
    for entry in &listed {
        if !has(&entry.file) && !missing.contains(&entry.file.as_str()) {
            missing.push(&entry.file);
        }
    }
    if !missing.is_empty() {
        let what = if missing.len() == 1 { "a file that is" } else { "files that are" };
        issues.push(issue(MemoryIssueLevel::Warning, format!("MEMORY.md points to {what} not there: {}.", some(&missing, 5))));
    }
    let unlisted: Vec<&str> = files.iter().map(AsRef::as_ref).filter(|file| !listed.iter().any(|e| same_name(&e.file, file))).collect();
    if !unlisted.is_empty() {
        let what = if unlisted.len() == 1 { "it" } else { "them" };
        issues.push(issue(MemoryIssueLevel::Info, format!("Not in MEMORY.md, so a session does not know of {what}: {}.", some(&unlisted, 5))));
    }
    issues
}

// ---------- one memory ----------

/// What the frontmatter of a memory says about it; empty texts for what it does not say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryMeta {
    pub name: String,
    pub description: String,
    /// `metadata.type`, else a `type` at the top (older files).
    pub r#type: String,
}

/// Reads the frontmatter of a memory. A frontmatter that is not YAML, not a map, or holds more
/// than one document says nothing; neither does a value that is not a text.
pub fn memory_meta(text: &str) -> MemoryMeta {
    let Some(frontmatter) = split_doc(text).frontmatter else { return MemoryMeta::default() };
    let Ok(docs) = YamlLoader::load_from_str(frontmatter) else { return MemoryMeta::default() };
    let [Yaml::Hash(record)] = docs.as_slice() else { return MemoryMeta::default() };
    let key = |k: &str| Yaml::String(k.to_owned());
    let text_of = |value: Option<&Yaml>| match value {
        Some(Yaml::String(s)) => one_line(s),
        _ => String::new(),
    };
    let nested = match record.get(&key("metadata")) {
        Some(Yaml::Hash(metadata)) => text_of(metadata.get(&key("type"))),
        _ => String::new(),
    };
    MemoryMeta {
        name: text_of(record.get(&key("name"))),
        description: text_of(record.get(&key("description"))),
        r#type: if nested.is_empty() { text_of(record.get(&key("type"))) } else { nested },
    }
}

/// Why a name cannot be given to a new memory, or None when it can.
pub fn memory_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Give the memory a name.".to_owned());
    }
    if name.to_lowercase() == "memory" {
        return Some("MEMORY.md is the index itself.".to_owned());
    }
    kebab_problem(name, MEMORY_LIMITS.name)
}

/// The text of a memory that was just made, as Claude Code writes them. `modified` is an ISO time
/// as `Date.prototype.toISOString` writes it.
pub fn new_memory_text(name: &str, description: &str, kind: MemoryType, modified: &str) -> String {
    let metadata = YamlValue::Map(vec![
        ("node_type".to_owned(), "memory".into()),
        ("type".to_owned(), kind.as_str().into()),
        ("modified".to_owned(), modified.into()),
    ]);
    let head = frontmatter_block(&[("name", name.into()), ("description", one_line(description).into()), ("metadata", metadata)]);
    format!("{head}\nWrite here what Claude should remember.\n")
}

// ---------- searching ----------

/// The byte offset of the character boundary `units` UTF-16 units into `s`. A cut inside a
/// surrogate pair moves past the character with `up`, before it without.
fn byte_at16(s: &str, units: usize, up: bool) -> usize {
    let mut count = 0usize;
    for (idx, ch) in s.char_indices() {
        if count >= units {
            return idx;
        }
        let next = count + ch.len_utf16();
        if next > units {
            return if up { idx + ch.len_utf8() } else { idx };
        }
        count = next;
    }
    s.len()
}

/// The text around the first of the words, for a list of hits; None when a word is missing or
/// there are none. The words are folded as [`fold_text`] folds (see [`search_words`]).
pub fn memory_snippet<S: AsRef<str>>(text: &str, words: &[S], around: usize) -> Option<String> {
    let first = words.first()?.as_ref();
    let folded = fold_text(text);
    if !words.iter().all(|w| folded.contains(w.as_ref())) {
        return None;
    }
    // folding keeps the length of ordinary text; a stray difference only moves the cut a little
    let length = len16(text);
    let found = folded.find(first).map_or(0, |at| len16(&folded[..at]));
    let at = found.min(length);
    let from = at.saturating_sub(around);
    let to = length.min(at + len16(first) + around);
    let start = byte_at16(text, from, true);
    let end = byte_at16(text, to, false).max(start);
    Some(format!("{}{}{}", if from > 0 { "…" } else { "" }, one_line(&text[start..end]), if to < length { "…" } else { "" }))
}

// ---------- bringing one account's memories into another's folder ----------

/// The slug a file brought from another account is named with: `foldText(slug)` with every run of
/// characters other than `a-z0-9` as one hyphen; `other` when that leaves nothing.
pub fn adopt_slug(slug: &str) -> String {
    let mut out = String::new();
    let mut run = false;
    for c in fold_text(slug).chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            run = false;
        } else if !run {
            out.push('-');
            run = true;
        }
    }
    if out.is_empty() {
        "other".to_owned()
    } else {
        out
    }
}

/// What it takes to bring the memories of one folder into another without losing anything on
/// either side: a file the target does not have goes over as it is; one it has with the same
/// content is left; one it has with other content goes over under a name that says where it came
/// from (`<stem>.from-<slug>.md`, then `-2`, `-3`, ...). MEMORY.md is never among them: the index
/// of the target gets lines for what arrived.
pub fn plan_adopt(source: &[AdoptFile], target: &[AdoptFile], slug: &str) -> AdoptPlan {
    let mut plan = AdoptPlan::default();
    let mut taken: HashMap<String, String> = target.iter().map(|f| (f.name.to_lowercase(), f.hash.clone())).collect();
    for file in source {
        if is_index(&file.name) {
            continue;
        }
        let stem = stem_of(&file.name);
        for n in 0usize.. {
            let name = match n {
                0 => file.name.clone(),
                1 => format!("{stem}.from-{slug}.md"),
                _ => format!("{stem}.from-{slug}-{n}.md"),
            };
            match taken.get(&name.to_lowercase()) {
                None => {
                    taken.insert(name.to_lowercase(), file.hash.clone());
                    plan.copy.push(AdoptCopy { from: file.name.clone(), to: name });
                    break;
                }
                Some(there) if *there == file.hash => {
                    plan.same.push(file.name.clone());
                    break;
                }
                Some(_) => {}
            }
        }
    }
    plan
}
