//! The instruction files Claude Code reads when a session starts, as the Library sees them
//! (`src/shared/instructions.ts`): an account's `CLAUDE.md` and `rules`, and a project's
//! `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md` and `.claude/rules`.
//!
//! Also the parts of `src/main/instructions/scan.ts` that need no file system: the ids of the
//! scopes, the folders a session looks into, and the notes of the load order. Paths here are
//! worked on as text (`path.normalize` of Node); the backend makes them absolute first.

use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use sha1::{Digest, Sha1};

use crate::frontmatter::{field_items, parse_doc};
use crate::jsstr::longer16;
use crate::types::{InstructionKind, InstructionRef, InstructionScopeKind, INSTRUCTION_LIMITS, MEMORY_LIMITS};

/// A file Claude Code reads from a scope's folder when it is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedFile {
    /// Within the scope's folder, with forward slashes.
    pub file: &'static str,
    pub kind: InstructionKind,
    pub about: &'static str,
}

const ACCOUNT_FILES: [FixedFile; 1] =
    [FixedFile { file: "CLAUDE.md", kind: InstructionKind::Claude, about: "Read in every project this account works in" }];

const PROJECT_FILES: [FixedFile; 3] = [
    FixedFile { file: "CLAUDE.md", kind: InstructionKind::Claude, about: "The project's instructions, shared through git" },
    FixedFile { file: ".claude/CLAUDE.md", kind: InstructionKind::Claude, about: "The same, kept in the .claude folder" },
    FixedFile { file: "CLAUDE.local.md", kind: InstructionKind::Local, about: "Yours alone for this project; keep it out of git" },
];

/// `instructionKey`: one text per file, for the window's maps.
pub fn instruction_key(r: &InstructionRef) -> String {
    format!("{}/{}", r.scope_id, r.file)
}

/// The fixed files of a scope, in the order they are listed; rules come after them.
pub fn fixed_files(kind: InstructionScopeKind) -> &'static [FixedFile] {
    match kind {
        InstructionScopeKind::Account => &ACCOUNT_FILES,
        InstructionScopeKind::Project => &PROJECT_FILES,
    }
}

/// The folder of a scope's rules, within the scope's folder.
pub fn rules_dir(kind: InstructionScopeKind) -> &'static str {
    match kind {
        InstructionScopeKind::Account => "rules",
        InstructionScopeKind::Project => ".claude/rules",
    }
}

static KEBAB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9-]+$").expect("valid regex"));
/// Names Windows keeps for devices, whatever the extension.
static DEVICE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(con|prn|aux|nul|com[0-9]|lpt[0-9])$").expect("valid regex"));

/// `kebabProblem` of `src/shared/skills.ts`, kept here so that this module stands on its own.
fn kebab_problem(name: &str, max: usize) -> Option<String> {
    if longer16(name, max) {
        return Some(format!("A name has at most {max} characters."));
    }
    if !KEBAB.is_match(name) {
        return Some("Use lower-case letters, digits and hyphens only.".to_owned());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Some("A name does not start or end with a hyphen, and has no two hyphens in a row.".to_owned());
    }
    if DEVICE_NAME.is_match(name) {
        return Some(format!("\"{name}\" is a name Windows keeps for a device."));
    }
    None
}

/// Why a name cannot be given to a new rule, or None when it can.
pub fn rule_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Give the rule a name.".to_owned());
    }
    kebab_problem(name, INSTRUCTION_LIMITS.rule_name)
}

/// The heading of a new rule: its name in words, `code-style` as `Code Style`; the name itself
/// when that leaves nothing.
pub fn rule_title(name: &str) -> String {
    let heading = name
        .split('-')
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    if heading.is_empty() {
        name.to_owned()
    } else {
        heading
    }
}

/// The text a file starts with when it is made here.
pub fn starter_text(kind: InstructionKind, scope: InstructionScopeKind, title: &str) -> String {
    match (kind, scope) {
        (InstructionKind::Rule, _) => format!(
            "# {title}\n\nOne topic per rule. To have it read only for some files, put a frontmatter with a \"paths\" list of globs above this line.\n"
        ),
        (InstructionKind::Local, _) => "# Local notes\n\nInstructions for this project that stay on this machine. Keep this file out of git.\n".to_owned(),
        (_, InstructionScopeKind::Account) => {
            "# My instructions\n\nWhat Claude should know in every project: how you like to work, what to always or never do.\n".to_owned()
        }
        _ => format!("# {title}\n\nWhat Claude should know about this project: how to build and test it, its conventions, what to stay away from.\n"),
    }
}

// ---------- reading a file ----------

/// `\s` of JavaScript: WhiteSpace and LineTerminator of ECMAScript, for a character class.
const JS_SPACE: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^[{JS_SPACE}]*(```|~~~)")).expect("valid regex"));
static CODE_SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").expect("valid regex"));
static IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?:^|[{JS_SPACE}(])@((?:[^{JS_SPACE}\\)]|\\ )+)")).expect("valid regex"));
/// An address or a link, which an `@` in a sentence can be part of. `\w` and `/i` of JavaScript
/// are ASCII only here, so the letters are spelled out.
static NOT_A_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.+-]+@|^[Hh][Tt][Tt][Pp][Ss]?:").expect("valid regex"));

/// The text without what stands in code: fenced blocks and `spans`, where an @ is just an @.
fn without_code(text: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut fence: Option<&str> = None;
    for line in text.split('\n') {
        let mark = FENCE.captures(line).and_then(|c| c.get(1)).map(|m| m.as_str());
        if let Some(open) = fence {
            if mark == Some(open) {
                fence = None;
            }
            continue;
        }
        if mark.is_some() {
            fence = mark;
            continue;
        }
        kept.push(CODE_SPAN.replace_all(line, " ").into_owned());
    }
    kept.join("\n")
}

/// The files a text pulls in with `@path`. Claude Code takes any `@word` outside code for one;
/// listed here are those that look like a path (a slash, a dot or a leading `~`), so that a
/// person's handle in a sentence is not shown as a missing file.
pub fn find_imports(text: &str) -> Vec<String> {
    let body = without_code(text);
    let mut found: Vec<String> = Vec::new();
    for caps in IMPORT.captures_iter(&body) {
        let raw = caps.get(1).map_or("", |m| m.as_str());
        let target = raw.replace("\\ ", " ");
        let target = target.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if !target.contains(['.', '/']) && !target.starts_with('~') {
            continue;
        }
        if NOT_A_FILE.is_match(target) {
            continue;
        }
        if !found.iter().any(|f| f == target) {
            found.push(target.to_owned());
        }
    }
    found
}

/// The globs of a rule's `paths`; empty for a rule that is always read.
pub fn rule_paths(text: &str) -> Vec<String> {
    let doc = parse_doc(text);
    if doc.error.is_some() {
        return Vec::new();
    }
    field_items(&doc, "paths")
}

/// `lineCount`: the lines of a text, a last line break not counting as one more.
pub fn line_count(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let lines = text.strip_suffix('\n').unwrap_or(text).split('\n').count();
    u32::try_from(lines).unwrap_or(u32::MAX)
}

// ---------- what a session reads ----------

/// The note of a file longer than Anthropic advises.
pub fn long_note() -> String {
    format!("Longer than the {} lines Anthropic advises; long files are followed less closely.", INSTRUCTION_LIMITS.lines)
}

/// The note of a rule with `paths`.
pub fn demand_note(paths: &[String]) -> String {
    format!("Read when Claude works on {}", paths.join(", "))
}

/// The note of an import that leads nowhere; `target` as it was written after the `@`.
pub fn missing_note(target: &str) -> String {
    format!("\"@{target}\" leads to no file.")
}

/// The lines of MEMORY.md a session reads, and the note when that is not all of them.
pub fn memory_lines(lines: u32) -> (u32, Option<String>) {
    let max = u32::try_from(MEMORY_LIMITS.index_lines).unwrap_or(u32::MAX);
    let note = (lines > max).then(|| format!("Only the first {max} of its {lines} lines are read."));
    (lines.min(max), note)
}

/// What stands for a folder in the ids and in the sets of what was read: letter case does not
/// count on Windows.
pub fn path_key(real: &str, windows: bool) -> String {
    if windows {
        real.to_lowercase()
    } else {
        real.to_owned()
    }
}

/// The id of a scope: `a-` (an account) or `f-` (a project folder) and the first 12 hex digits of
/// the sha1 of its resolved folder's key.
pub fn scope_id(prefix: char, real: &str, windows: bool) -> String {
    let digest = hex::encode(Sha1::digest(path_key(real, windows).as_bytes()));
    format!("{prefix}-{}", &digest[..12])
}

/// `path.normalize` for a path: `.` dropped, `..` taking the folder before it away (never above
/// the root), the separators of the system, no separator at the end.
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    // folders that a `..` can take away again
    let mut depth = 0usize;
    for part in p.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => out.push(part.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            Component::Normal(name) => {
                out.push(name);
                depth += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// `path.join(base, rest)`: `rest` is taken as lying within `base` even when it starts with a
/// separator, as `~/notes.md` does once the `~` is taken off.
pub fn join_within(base: &Path, rest: &str) -> PathBuf {
    normalize_path(&base.join(rest.trim_start_matches(['/', '\\'])))
}

/// The folders Claude Code looks into for a session in `cwd` (an absolute path): from just below
/// the top of the drive (the top itself is not read), or from `top`, down to `cwd`.
pub fn chain_to(cwd: &Path, top: Option<&Path>, windows: bool) -> Vec<PathBuf> {
    let stop = top.map(|t| path_key(&normalize_path(t).to_string_lossy(), windows));
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut at = normalize_path(cwd);
    while let Some(up) = at.parent().map(Path::to_path_buf) {
        let reached = stop.as_deref() == Some(path_key(&at.to_string_lossy(), windows).as_str());
        chain.push(at);
        if reached {
            break;
        }
        at = up;
    }
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    //! The pure parts of `test/instructions.test.ts`.

    use super::*;

    fn lines(l: &[&str]) -> String {
        format!("{}\n", l.join("\n"))
    }

    #[test]
    fn finds_the_files_a_text_imports_and_leaves_code_and_addresses_alone() {
        let text = lines(&[
            "# Project",
            "See @README.md and @docs/git-flow.md, also (@~/.claude/my-notes.md).",
            "Ask @bilal or mail someone@example.com; the package is @anthropic-ai/sdk.",
            "Run `npm i @types/node` to install, or:",
            "```",
            "@not/an-import.md",
            "```",
            "A path with a space: @docs/My\\ Notes.md and twice @README.md.",
        ]);
        assert_eq!(find_imports(&text), ["README.md", "docs/git-flow.md", "~/.claude/my-notes.md", "anthropic-ai/sdk", "docs/My Notes.md"]);
        assert!(find_imports("No imports here.\n").is_empty());
    }

    #[test]
    fn imports_skip_links_addresses_and_tilde_fences() {
        let text = lines(&["~~~", "@inside/tilde.md", "~~~", "@https://example.com/x.md and @me@example.com and @~", "@a.md, @b.md!"]);
        assert_eq!(find_imports(&text), ["~", "a.md", "b.md"]);
        // a fence that is never closed hides the rest of the file
        assert!(find_imports("```\n@a.md\n").is_empty());
    }

    #[test]
    fn reads_the_paths_of_a_rule_however_they_are_written() {
        assert_eq!(rule_paths(&lines(&["---", "paths:", "  - \"src/**/*.ts\"", "  - tests/**", "---", "", "# Rule"])), ["src/**/*.ts", "tests/**"]);
        assert_eq!(rule_paths(&lines(&["---", "paths: src/**, docs/**", "---"])), ["src/**", "docs/**"]);
        assert!(rule_paths("# A rule for everything\n").is_empty());
        assert!(rule_paths(&lines(&["---", "paths: [broken", "---"])).is_empty());
    }

    #[test]
    fn counts_lines_names_rules_and_starts_new_files() {
        assert_eq!([line_count(""), line_count("a"), line_count("a\n"), line_count("a\nb\n")], [0, 1, 1, 2]);
        assert_eq!(rule_name_problem("code-style"), None);
        assert_eq!(rule_name_problem("Code Style").as_deref(), Some("Use lower-case letters, digits and hyphens only."));
        assert_eq!(rule_name_problem("").as_deref(), Some("Give the rule a name."));
        assert_eq!(rule_name_problem("-x").as_deref(), Some("A name does not start or end with a hyphen, and has no two hyphens in a row."));
        assert_eq!(rule_name_problem("nul").as_deref(), Some("\"nul\" is a name Windows keeps for a device."));
        assert_eq!(rule_name_problem(&"a".repeat(65)).as_deref(), Some("A name has at most 64 characters."));
        let files = |kind| fixed_files(kind).iter().map(|f| f.file).collect::<Vec<_>>();
        assert_eq!(files(InstructionScopeKind::Account), ["CLAUDE.md"]);
        assert_eq!(files(InstructionScopeKind::Project), ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"]);
        assert!(starter_text(InstructionKind::Claude, InstructionScopeKind::Project, "my-app").starts_with("# my-app\n\n"));
        assert!(starter_text(InstructionKind::Rule, InstructionScopeKind::Project, "Code Style").starts_with("# Code Style\n\n"));
        assert!(starter_text(InstructionKind::Claude, InstructionScopeKind::Account, "").starts_with("# My instructions\n\n"));
        assert!(starter_text(InstructionKind::Local, InstructionScopeKind::Project, "x").starts_with("# Local notes\n\n"));
        assert_eq!(rule_title("code-style"), "Code Style");
        assert_eq!(rule_title("tone"), "Tone");
        assert_eq!(rule_title("-"), "-");
        assert_eq!(rules_dir(InstructionScopeKind::Account), "rules");
        assert_eq!(rules_dir(InstructionScopeKind::Project), ".claude/rules");
        assert_eq!(instruction_key(&InstructionRef { scope_id: "f-0123456789ab".into(), file: "CLAUDE.md".into() }), "f-0123456789ab/CLAUDE.md");
    }

    #[test]
    fn notes_of_the_load_order() {
        assert!(long_note().starts_with("Longer than the 200 lines"));
        assert_eq!(demand_note(&["**/*.ts".into(), "src/**".into()]), "Read when Claude works on **/*.ts, src/**");
        assert_eq!(missing_note("missing.md"), "\"@missing.md\" leads to no file.");
        assert_eq!(memory_lines(230), (200, Some("Only the first 200 of its 230 lines are read.".into())));
        assert_eq!(memory_lines(12), (12, None));
    }

    #[test]
    fn scope_ids_follow_the_folder_and_ignore_case_on_windows() {
        let id = scope_id('a', "C:\\Users\\Me\\.claude", true);
        assert!(id.starts_with("a-") && id.len() == 14);
        assert_eq!(id, scope_id('a', "c:\\users\\me\\.claude", true));
        assert_ne!(scope_id('f', "/Code/App", false), scope_id('f', "/code/app", false));
        assert_eq!(scope_id('f', "/code/app", false), format!("f-{}", &hex::encode(Sha1::digest(b"/code/app"))[..12]));
    }

    #[test]
    fn normalizes_and_joins_like_node() {
        let base = Path::new("/home/me");
        assert_eq!(normalize_path(Path::new("/a/./b/../c/")), Path::new("/a/c"));
        assert_eq!(normalize_path(Path::new("/../a")), Path::new("/a"));
        assert_eq!(normalize_path(Path::new("a/../..")), Path::new(".."));
        assert_eq!(join_within(base, "/notes/style.md"), base.join("notes").join("style.md"));
        assert_eq!(join_within(base, "../x.md"), Path::new("/home").join("x.md"));
    }

    #[test]
    #[cfg(not(windows))]
    fn the_chain_goes_from_below_the_root_or_from_top_down_to_the_folder() {
        let chain = chain_to(Path::new("/a/b/c"), None, false);
        assert_eq!(chain, [PathBuf::from("/a"), PathBuf::from("/a/b"), PathBuf::from("/a/b/c")]);
        assert_eq!(chain_to(Path::new("/a/b/c"), Some(Path::new("/a/b")), false), [PathBuf::from("/a/b"), PathBuf::from("/a/b/c")]);
        assert!(chain_to(Path::new("/"), None, false).is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn the_chain_goes_from_below_the_drive_or_from_top_down_to_the_folder() {
        let chain = chain_to(Path::new("C:\\a\\b\\c"), None, true);
        assert_eq!(chain, [PathBuf::from("C:\\a"), PathBuf::from("C:\\a\\b"), PathBuf::from("C:\\a\\b\\c")]);
        assert_eq!(chain_to(Path::new("C:\\a\\b\\c"), Some(Path::new("c:\\A\\b")), true), [PathBuf::from("C:\\a\\b"), PathBuf::from("C:\\a\\b\\c")]);
        assert!(chain_to(Path::new("C:\\"), None, true).is_empty());
    }
}
