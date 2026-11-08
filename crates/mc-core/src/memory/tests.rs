//! `test/memory.test.ts`, and the `sanitizePath`, `gitRootOf` and `memoryKeyOf` tests of
//! `test/memoryFiles.test.ts`.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::types::{AdoptCopy, AdoptFile, AdoptPlan, IndexLine, MemoryIssue, MemoryIssueLevel, MemoryType};

fn lines(l: &[&str]) -> String {
    format!("{}\n", l.join("\n"))
}

// as Claude Code writes an index
fn index() -> String {
    lines(&[
        "# Memory Index",
        "",
        "- [No explanatory comments](no-explanatory-comments.md) — Bilal wants zero Claude-added code comments",
        "- [claude-config repo](claude-config-repo.md) — ~/.claude is a private git repo (bilal-arvis/claude-config)",
        "- [Offline compile check](./offline-compile-check.md)",
        "",
        "See also [the docs](https://example.com/page.md) and [a note elsewhere](../other/note.md).",
    ])
}

fn entry(line: u32, title: &str, file: &str, hook: &str) -> IndexLine {
    IndexLine { line, title: title.into(), file: file.into(), hook: hook.into() }
}

fn idx(text: &str) -> Option<IndexFile<'_>> {
    Some(IndexFile { text, bytes: len16(text) as u64 })
}

// ---------- the index ----------

#[test]
fn reads_the_lines_that_point_to_a_memory() {
    assert_eq!(
        parse_index(&index()),
        vec![
            entry(2, "No explanatory comments", "no-explanatory-comments.md", "Bilal wants zero Claude-added code comments"),
            entry(3, "claude-config repo", "claude-config-repo.md", "~/.claude is a private git repo (bilal-arvis/claude-config)"),
            entry(4, "Offline compile check", "offline-compile-check.md", ""),
        ]
    );
    assert_eq!(parse_index(""), vec![]);
    // other ways of writing the same line
    let other: Vec<(String, String)> = parse_index("* [A](a.md): when it rains\n+ [B](b.md) - hook\n").into_iter().map(|e| (e.file, e.hook)).collect();
    assert_eq!(other, vec![("a.md".into(), "when it rains".into()), ("b.md".into(), "hook".into())]);
}

#[test]
fn adds_a_line_after_the_last_one_and_starts_a_list_where_none_was() {
    let added = add_index_line(&index(), "Unity [CLI] tips", "unity-cli-tips.md", "menu builders,\nscreenshots");
    assert_eq!(
        added.split('\n').skip(4).take(3).collect::<Vec<_>>(),
        vec!["- [Offline compile check](./offline-compile-check.md)", "- [Unity CLI tips](unity-cli-tips.md) — menu builders, screenshots", ""]
    );
    assert_eq!(parse_index(&added).len(), 4);

    assert_eq!(add_index_line("", "First", "first.md", ""), "- [First](first.md)\n");
    assert_eq!(add_index_line("# Memory Index\n", "First", "first.md", "why"), "# Memory Index\n\n- [First](first.md) — why\n");
    // a file without a last line break gets one
    assert_eq!(add_index_line("- [A](a.md)", "", "b.md", ""), "- [A](a.md)\n- [b](b.md)\n");
}

#[test]
fn takes_out_the_lines_of_one_file_and_nothing_else() {
    let without = remove_index_line(&index(), "Claude-Config-Repo.md");
    assert_eq!(parse_index(&without).into_iter().map(|e| e.file).collect::<Vec<_>>(), vec!["no-explanatory-comments.md", "offline-compile-check.md"]);
    assert_eq!(
        without,
        index().replace("- [claude-config repo](claude-config-repo.md) — ~/.claude is a private git repo (bilal-arvis/claude-config)\n", "")
    );
    assert_eq!(remove_index_line(&index(), "nothing.md"), index());
}

#[test]
fn writes_a_line_as_it_reads_it() {
    let read = parse_index(&index_line("A title", "a-title.md", "when to read it"));
    assert_eq!((read[0].title.as_str(), read[0].file.as_str(), read[0].hook.as_str()), ("A title", "a-title.md", "when to read it"));
}

// ---------- inspectMemory ----------

const FILES: [&str; 3] = ["no-explanatory-comments.md", "claude-config-repo.md", "offline-compile-check.md"];

#[test]
fn has_nothing_to_say_when_the_index_and_the_folder_agree() {
    let text = index();
    assert_eq!(inspect_memory(idx(&text), &FILES), vec![]);
    assert_eq!(inspect_memory::<&str>(None, &[]), vec![]);
}

#[test]
fn names_files_the_index_does_not_list_and_lines_whose_file_is_gone() {
    let text = index();
    let issues = inspect_memory(idx(&text), &["no-explanatory-comments.md", "Claude-Config-Repo.md", "stray.md"]);
    assert_eq!(
        issues,
        vec![
            MemoryIssue { level: MemoryIssueLevel::Warning, message: "MEMORY.md points to a file that is not there: offline-compile-check.md.".into() },
            MemoryIssue { level: MemoryIssueLevel::Info, message: "Not in MEMORY.md, so a session does not know of it: stray.md.".into() },
        ]
    );
    assert!(inspect_memory(None, &["a.md"])[0].message.contains("There is no MEMORY.md."));
    // a long list is cut short
    let many = (0..8).map(|i| format!("- [M{i}](m{i}.md)")).collect::<Vec<_>>().join("\n");
    assert_eq!(
        inspect_memory::<&str>(idx(&many), &[])[0].message,
        "MEMORY.md points to files that are not there: m0.md, m1.md, m2.md, m3.md, m4.md and 3 more."
    );
}

#[test]
fn warns_when_a_session_would_not_read_the_index_to_its_end() {
    let long = (0..201).map(|i| format!("- [M{i}](m{i}.md)")).collect::<Vec<_>>().join("\n") + "\n";
    let all: Vec<String> = (0..201).map(|i| format!("m{i}.md")).collect();
    assert_eq!(
        inspect_memory(idx(&long), &all),
        vec![MemoryIssue {
            level: MemoryIssueLevel::Warning,
            message: "MEMORY.md has 201 lines. A session reads the first 200; what comes after is not seen.".into()
        }]
    );
    let heavy = format!("- [A](a.md) — {}\n", "x".repeat(26_000));
    assert_eq!(inspect_memory(idx(&heavy), &["a.md"])[0].message, "MEMORY.md has 26 KB. A session reads the first 25 KB; what comes after is not seen.");
}

// ---------- one memory ----------

// as Claude Code writes a memory file
fn real() -> String {
    lines(&[
        "---",
        "name: claude-config-repo",
        "description: ~/.claude is a private git repo syncing user-level skills/agents/settings",
        "metadata: ",
        "  node_type: memory",
        "  type: reference",
        "  originSessionId: 63c5497c-b5b9-4965-81e7-397beb58ae62",
        "  modified: 2026-08-31T21:51:12.751Z",
        "---",
        "",
        "The folder is a git repo.",
    ])
}

fn meta(name: &str, description: &str, kind: &str) -> MemoryMeta {
    MemoryMeta { name: name.into(), description: description.into(), r#type: kind.into() }
}

#[test]
fn reads_what_the_frontmatter_says() {
    assert_eq!(memory_meta(&real()), meta("claude-config-repo", "~/.claude is a private git repo syncing user-level skills/agents/settings", "reference"));
    assert_eq!(memory_meta("Just text.\n"), MemoryMeta::default());
    assert_eq!(memory_meta("---\nname: [broken\n---\n"), MemoryMeta::default());
    // an older file with the type at the top
    assert_eq!(memory_meta("---\nname: a\ntype: feedback\n---\n").r#type, "feedback");
    // a key written twice is no frontmatter to `yaml` either
    assert_eq!(memory_meta("---\nname: a\nname: b\n---\n"), MemoryMeta::default());
}

#[test]
fn writes_a_new_memory_the_same_way() {
    let text = new_memory_text("release-steps", "How a release\nis cut: tag, then build", MemoryType::Project, "2026-10-08T10:00:00.000Z");
    assert_eq!(memory_meta(&text), meta("release-steps", "How a release is cut: tag, then build", "project"));
    assert!(text.contains("  node_type: memory\n  type: project\n  modified: 2026-10-08T10:00:00.000Z\n---\n\n"), "{text}");
}

#[test]
fn asks_for_a_name_that_can_be_a_file() {
    assert_eq!(memory_name_problem("release-steps"), None);
    assert_eq!(memory_name_problem("").as_deref(), Some("Give the memory a name."));
    assert_eq!(memory_name_problem("memory").as_deref(), Some("MEMORY.md is the index itself."));
    assert_eq!(memory_name_problem("Release Steps").as_deref(), Some("Use lower-case letters, digits and hyphens only."));
    assert_eq!(memory_name_problem("-a").as_deref(), Some("A name does not start or end with a hyphen, and has no two hyphens in a row."));
    assert_eq!(memory_name_problem("com1").as_deref(), Some("\"com1\" is a name Windows keeps for a device."));
    assert_eq!(memory_name_problem(&"a".repeat(65)).as_deref(), Some("A name has at most 64 characters."));
}

#[test]
fn shows_the_text_around_what_a_search_found() {
    let text = "The editor must never be started without asking. Bilal may be in the middle of something else on the machine.";
    assert_eq!(memory_snippet(text, &["bilal"], 20).as_deref(), Some("…ted without asking. Bilal may be in the middl…"));
    assert_eq!(memory_snippet(text, &["editor", "machine"], 10).as_deref(), Some("The editor must neve…"));
    assert_eq!(memory_snippet(text, &["editor", "missing"], SNIPPET_AROUND), None);
    assert_eq!(memory_snippet("Işık açık kalsın", &["isik"], SNIPPET_AROUND).as_deref(), Some("Işık açık kalsın"));
    assert_eq!(memory_snippet::<&str>(text, &[], SNIPPET_AROUND), None);
    assert_eq!(search_words("  Işık   KALSIN "), vec!["isik", "kalsin"]);
}

// ---------- planAdopt ----------

fn file(name: &str, hash: &str) -> AdoptFile {
    AdoptFile { name: name.into(), hash: hash.into() }
}

fn copy(from: &str, to: &str) -> AdoptCopy {
    AdoptCopy { from: from.into(), to: to.into() }
}

#[test]
fn copies_what_the_target_lacks_and_leaves_what_it_has() {
    let plan = plan_adopt(&[file("MEMORY.md", "i1"), file("a.md", "a"), file("b.md", "b")], &[file("MEMORY.md", "i2"), file("B.md", "b")], "elif");
    assert_eq!(plan, AdoptPlan { copy: vec![copy("a.md", "a.md")], same: vec!["b.md".into()] });
}

#[test]
fn brings_a_file_of_the_same_name_with_other_content_under_a_name_that_says_where_it_came_from() {
    let plan = plan_adopt(&[file("a.md", "theirs")], &[file("a.md", "mine")], "elif");
    assert_eq!(plan, AdoptPlan { copy: vec![copy("a.md", "a.from-elif.md")], same: vec![] });
    // brought in before: nothing to do
    assert_eq!(
        plan_adopt(&[file("a.md", "theirs")], &[file("a.md", "mine"), file("a.from-elif.md", "theirs")], "elif"),
        AdoptPlan { copy: vec![], same: vec!["a.md".into()] }
    );
    // brought in before, and changed since on one side
    assert_eq!(
        plan_adopt(&[file("a.md", "newer")], &[file("a.md", "mine"), file("a.from-elif.md", "theirs")], "elif").copy,
        vec![copy("a.md", "a.from-elif-2.md")]
    );
}

#[test]
fn never_touches_the_index_and_never_gives_two_files_one_name() {
    let plan = plan_adopt(&[file("MEMORY.md", "x"), file("a.md", "1"), file("A.md", "2")], &[], "solo");
    assert_eq!(plan.copy, vec![copy("a.md", "a.md"), copy("A.md", "A.from-solo.md")]);
    assert_eq!(adopt_slug("Elif Hesabı"), "elif-hesabi");
    assert_eq!(adopt_slug("—"), "-");
    assert_eq!(adopt_slug(""), "other");
}

// ---------- the name of a project (test/memoryFiles.test.ts) ----------

fn write(file: &Path, content: &str) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn temp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    (dir, path)
}

#[test]
fn is_the_path_with_a_hyphen_for_everything_that_is_not_a_plain_letter_or_digit() {
    // as they stand in a real config dir
    assert_eq!(sanitize_path(r"C:\Users\aydnb\multi-claude"), "C--Users-aydnb-multi-claude");
    assert_eq!(sanitize_path(r"W:\Unity Projects\project-lunarshop"), "W--Unity-Projects-project-lunarshop");
    assert_eq!(
        sanitize_path(r"C:\Users\aydnb\Downloads\Metaphor Refantazio Türkçe Yama Orjinal Sürüm (Story Master)"),
        "C--Users-aydnb-Downloads-Metaphor-Refantazio-T-rk-e-Yama-Orjinal-S-r-m--Story-Master-"
    );
    assert_eq!(sanitize_path(r"C:\Users\aydnb\.multi-claude\probe"), "C--Users-aydnb--multi-claude-probe");
    assert_eq!(sanitize_path("/home/me/my_app.v2"), "-home-me-my-app-v2");
    assert_eq!(sanitize_path(r"C:\"), "C--");
    // a character outside the BMP is two UTF-16 units, as JavaScript replaces them
    assert_eq!(sanitize_path("a😀b"), "a--b");
}

#[test]
fn goes_by_the_top_of_the_repository_for_a_folder_inside_it_as_well() {
    let (_guard, tmp) = temp();
    let repo = tmp.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join("src").join("deep")).unwrap();
    assert_eq!(git_root_of(&repo.join("src").join("deep")), Some(repo.clone()));
    assert_eq!(memory_key(&repo.join("src").join("deep")), Ok(MemoryKey { key: sanitize_path(&repo.to_string_lossy()), root: repo.clone() }));
    assert_eq!(memory_key_of(&repo.join("src")), Some(sanitize_path(&repo.to_string_lossy())));
    // `.` and `..` do not count
    assert_eq!(git_root_of(&repo.join("src").join("..").join(".").join("src")), Some(repo.clone()));

    let plain = tmp.join("plain").join("sub");
    fs::create_dir_all(&plain).unwrap();
    assert_eq!(git_root_of(&plain), None);
    assert_eq!(memory_key(&plain), Ok(MemoryKey { key: sanitize_path(&plain.to_string_lossy()), root: plain.clone() }));
}

#[test]
fn goes_by_the_repository_a_worktree_belongs_to_and_by_a_submodule_itself() {
    let (_guard, tmp) = temp();
    let repo = tmp.join("repo");
    let worktree = tmp.join("wt").join("feature");
    let linked = repo.join(".git").join("worktrees").join("feature");
    write(&linked.join("commondir"), "../..\n");
    write(&worktree.join(".git"), &format!("gitdir: {}\n", linked.to_string_lossy().replace('\\', "/")));
    fs::create_dir_all(worktree.join("src")).unwrap();
    assert_eq!(git_root_of(&worktree.join("src")), Some(repo.clone()));
    // with Windows line ends
    write(&worktree.join(".git"), &format!("gitdir: {}\r\n", linked.to_string_lossy()));
    assert_eq!(git_root_of(&worktree), Some(repo.clone()));

    let sub = repo.join("vendor").join("lib");
    write(&repo.join(".git").join("modules").join("lib").join("HEAD"), "ref: refs/heads/main\n");
    write(&sub.join(".git"), "gitdir: ../../.git/modules/lib\n");
    assert_eq!(git_root_of(&sub), Some(sub.clone()));
}

#[test]
fn does_not_serve_a_project_whose_name_claude_code_would_cut() {
    let (_guard, tmp) = temp();
    let long = tmp.join("a".repeat(120)).join("b".repeat(120));
    assert!(memory_key(&long).unwrap_err().contains("longer than 200 characters"));
    assert_eq!(memory_key(Path::new("relative/path")), Err("The folder is not an absolute path.".to_owned()));
    assert_eq!(memory_key_of(Path::new("relative/path")), None);
}

// ---------- small helpers ----------

#[test]
fn names_files_as_the_typescript_does() {
    assert_eq!(stem_of("a.MD"), "a");
    assert_eq!(stem_of("a.txt"), "a.txt");
    assert!(is_index("memory.md") && !is_index("memory.txt"));
    assert_eq!(entry_named(&["C--App", "b"], "c--app"), Some("C--App"));
    assert_eq!(line_count(""), 0);
    assert_eq!(line_count("a\n"), 1);
    assert_eq!(line_count("a\n\n"), 2);
}
