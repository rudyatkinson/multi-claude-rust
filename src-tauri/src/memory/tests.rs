//! `test/memoryFiles.test.ts`: memory folders, `ProjectFolders`, `MemoryService`,
//! `sharedMemoryDir`, `namingProblem`, the `MemorySharing` cache and `adoptMemories`. (The
//! `sanitizePath`, `gitRootOf` and `memoryKeyOf` tests are in `mc_core::memory`.)

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use mc_core::memory::sanitize_path;
use mc_core::types::{AdoptCopy, AdoptResult, MemoryCreateInput, MemoryRef, Profile, SaveOutcome, ShareConfig};
use serde_json::json;
use tempfile::TempDir;

use super::*;

const UUID: &str = "0f8e8e78-4877-4f7c-960e-dea62b43de77";
/// 2026-10-08T10:00:00.000Z
const OCT_8: i64 = 1_791_453_600_000;

/// A folder for one test, by its real path (as `fs.realpathSync(fs.mkdtempSync(...))`).
fn temp() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let real = fs::canonicalize(dir.path()).unwrap();
    let text = real.to_string_lossy().into_owned();
    let real = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
    (dir, real)
}

fn write(file: &Path, content: impl AsRef<[u8]>) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn read(file: &Path) -> String {
    fs::read_to_string(file).unwrap()
}

fn memory_text(name: &str, kind: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: What {name} is\nmetadata:\n  node_type: memory\n  type: {kind}\n---\n\n{body}\n")
}

/// A conversation file as Claude Code starts one: its first records name the folder.
fn conversation(cwd: &Path) -> String {
    format!("{}\n", json!({ "type": "user", "cwd": cwd.to_string_lossy(), "message": { "content": "hello" } }))
}

fn key_of(p: &Path) -> String {
    sanitize_path(&p.to_string_lossy())
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn account(id: &str, name: &str, slug: &str, config_dir: &Path, is_default: bool, share_memory: bool) -> MemoryAccount {
    MemoryAccount { id: id.into(), name: name.into(), slug: slug.into(), config_dir: config_dir.to_path_buf(), is_default, share_memory }
}

fn link_dir(target: &Path, link: &Path) {
    #[cfg(windows)]
    junction::create(target, link).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

// ---------- memory folders ----------

#[test]
fn lists_the_folders_that_hold_a_file_once_when_two_accounts_lead_to_the_same_place() {
    let (_guard, tmp) = temp();
    let main = tmp.join("main");
    let other = tmp.join("other");
    let mem = |root: &Path, key: &str| root.join("projects").join(key).join("memory");
    write(&mem(&main, "C--proj-a").join("MEMORY.md"), "- [One](one.md)\n");
    write(&mem(&main, "C--proj-a").join("one.md"), memory_text("one", "project", "About one."));
    write(&mem(&main, "C--proj-a").join("logs").join("today.md"), "a log");
    write(&mem(&main, "C--proj-a").join("notes.txt"), "no memory");
    fs::create_dir_all(mem(&main, "C--proj-empty")).unwrap();
    write(&mem(&other, "C--proj-a").join("own.md"), memory_text("own", "project", "About own."));
    write(&mem(&main, "C--proj-b").join("b.md"), memory_text("b", "project", "About b."));
    fs::create_dir_all(other.join("projects").join("C--proj-b")).unwrap();
    link_dir(&mem(&main, "C--proj-b"), &mem(&other, "C--proj-b"));

    let found = find_memory_folders([("main", main.as_path()), ("p2", other.as_path())]);
    let seen: Vec<(String, Vec<String>, Vec<String>)> = found.iter().map(|f| (f.key.clone(), f.profile_ids.clone(), f.files.clone())).collect();
    let row = |key: &str, ids: &[&str], files: &[&str]| (key.to_owned(), ids.iter().map(|s| (*s).to_owned()).collect(), files.iter().map(|s| (*s).to_owned()).collect());
    assert_eq!(
        seen,
        vec![row("C--proj-a", &["main"], &["MEMORY.md", "one.md"]), row("C--proj-b", &["main", "p2"], &["b.md"]), row("C--proj-a", &["p2"], &["own.md"])]
    );
    let ids: std::collections::HashSet<&str> = found.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids.len(), 3);
    assert!(mc_core::validate::memory_folder(&found[0].id).is_ok(), "{}", found[0].id);
    assert_eq!(found[0].dir, mem(&main, "C--proj-a"));
}

#[test]
fn tells_the_folder_of_a_project_from_a_conversation_or_from_a_folder_the_app_knows() {
    let (_guard, tmp) = temp();
    let folders = ProjectFolders::new();
    let project = tmp.join("My Project");
    let key = key_of(&project);
    let project_dir = tmp.join("config").join("projects").join(&key);
    write(&project_dir.join(format!("{UUID}.jsonl")), conversation(&project));
    let none: [&str; 0] = [];
    assert_eq!(folders.cwd_of(&key, &[tmp.join("nowhere"), project_dir.clone()], &none), Some(s(&project)));
    // once found, it is kept
    fs::remove_dir_all(&project_dir).unwrap();
    assert_eq!(folders.cwd_of(&key, std::slice::from_ref(&project_dir), &none), Some(s(&project)));

    // a conversation held in another folder that happens to be filed there says nothing
    let odd = tmp.join("config").join("projects").join("C--other");
    write(&odd.join(format!("{UUID}.jsonl")), conversation(&project));
    assert_eq!(ProjectFolders::new().cwd_of("C--other", &[odd], &none), None);

    // sessions only ran inside the repository: no conversation is filed under its top
    let repo = tmp.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join("app")).unwrap();
    let candidates = [s(&tmp.join("elsewhere")), s(&repo.join("app"))];
    assert_eq!(ProjectFolders::new().cwd_of(&key_of(&repo), &[], &candidates), Some(s(&repo)));
}

// ---------- MemoryService ----------

struct World {
    _guard: TempDir,
    service: MemoryService,
    inputs: MemoryInputs,
    other: PathBuf,
    project: PathBuf,
    key: String,
    dir: PathBuf,
    trashed: Arc<Mutex<Vec<PathBuf>>>,
    fail_trash: Arc<AtomicBool>,
}

fn world(share_memory: bool) -> World {
    let (guard, tmp) = temp();
    let main = tmp.join("main");
    let other = tmp.join("other");
    let project = tmp.join("app");
    fs::create_dir_all(&project).unwrap();
    let key = key_of(&project);
    let dir = main.join("projects").join(&key).join("memory");
    write(&main.join("projects").join(&key).join(format!("{UUID}.jsonl")), conversation(&project));
    write(&dir.join("MEMORY.md"), "# Memory Index\n\n- [Release steps](release-steps.md) — how a release is cut\n- [Gone](gone.md) — its file was deleted by hand\n");
    write(&dir.join("release-steps.md"), memory_text("release-steps", "project", "Tag first, then build the installer."));
    write(&dir.join("stray.md"), memory_text("stray", "feedback", "Nobody listed this one."));
    write(&other.join("projects").join(&key).join("memory").join("own.md"), memory_text("own", "user", "The other account wrote this about the installer."));
    let trashed: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let fail_trash = Arc::new(AtomicBool::new(false));
    let service = MemoryService::with_parts(None, Arc::new(|| OCT_8));
    let (t, f) = (trashed.clone(), fail_trash.clone());
    service.set_trash(move |file| {
        if f.load(Ordering::SeqCst) {
            return Err(io::Error::other("no bin on this drive"));
        }
        t.lock().unwrap().push(file.to_path_buf());
        fs::remove_file(file)
    });
    let inputs = MemoryInputs {
        accounts: vec![account("main", "Main", "main", &main, true, false), account("p2", "Second", "second", &other, false, share_memory)],
        folders: vec![],
    };
    World { _guard: guard, service, inputs, other, project, key, dir, trashed, fail_trash }
}

fn folder_of(service: &MemoryService, inputs: &MemoryInputs, profile_id: &str) -> String {
    service.list(inputs).projects.iter().flat_map(|p| &p.folders).find(|f| f.profile_ids.iter().any(|id| id == profile_id)).unwrap().id.clone()
}

fn main_folder(w: &World) -> String {
    folder_of(&w.service, &w.inputs, "main")
}

fn mref(folder_id: &str, file: &str) -> MemoryRef {
    MemoryRef { folder_id: folder_id.into(), file: file.into() }
}

fn input(name: &str, title: &str, description: &str, kind: &str) -> MemoryCreateInput {
    MemoryCreateInput { name: name.into(), title: title.into(), description: description.into(), r#type: kind.into() }
}

#[test]
fn lists_the_projects_with_the_accounts_that_remember_something_of_them() {
    let w = world(true);
    let listing = w.service.list(&w.inputs);
    assert_eq!(listing.projects.len(), 1);
    assert_eq!((listing.projects[0].key.as_str(), listing.projects[0].cwd.clone()), (w.key.as_str(), Some(s(&w.project))));
    let folders: Vec<(Vec<String>, Vec<String>, u32)> = listing.projects[0].folders.iter().map(|f| (f.profile_ids.clone(), f.shared_with.clone(), f.count)).collect();
    assert_eq!(folders, vec![(vec!["main".to_owned()], vec!["p2".to_owned()], 2), (vec!["p2".to_owned()], vec![], 1)]);
    assert_eq!(listing.share_problem, None);
}

#[test]
fn opens_a_folder_the_index_first_then_its_memories_as_the_index_lists_them() {
    let w = world(false);
    let view = w.service.entries(&w.inputs, &main_folder(&w)).unwrap();
    assert_eq!((view.has_index, view.index_lines, view.dir.as_str()), (true, 4, s(&w.dir).as_str()));
    let rows: Vec<(String, String, String, String, bool)> =
        view.entries.iter().map(|e| (e.file.clone(), e.title.clone(), e.description.clone(), e.r#type.clone(), e.indexed)).collect();
    let row = |a: &str, b: &str, c: &str, d: &str, e: bool| (a.to_owned(), b.to_owned(), c.to_owned(), d.to_owned(), e);
    assert_eq!(
        rows,
        vec![
            row("MEMORY.md", "MEMORY.md", "The index every session reads: 4 lines", "", true),
            row("release-steps.md", "Release steps", "how a release is cut", "project", true),
            row("stray.md", "stray", "What stray is", "feedback", false),
        ]
    );
    let messages: Vec<&str> = view.issues.iter().map(|i| i.message.as_str()).collect();
    assert_eq!(messages, vec!["MEMORY.md points to a file that is not there: gone.md.", "Not in MEMORY.md, so a session does not know of it: stray.md."]);
}

#[test]
fn reads_and_saves_a_memory_and_only_files_the_folder_lists() {
    let w = world(false);
    let folder_id = main_folder(&w);
    let read = w.service.read(&w.inputs, &mref(&folder_id, "stray.md")).unwrap();
    assert!(read.text.contains("Nobody listed this one."));
    assert_eq!(read.path, s(&w.dir.join("stray.md")));
    let saved = w.service.save(&w.inputs, &mref(&folder_id, "stray.md"), &read.text.replace("Nobody", "Somebody"), Some(&read.hash)).unwrap();
    assert!(matches!(saved, SaveOutcome::Saved(_)));
    assert!(read_to(&w.dir.join("stray.md")).contains("Somebody listed"));
    // the state it was read in is gone by now
    assert!(matches!(w.service.save(&w.inputs, &mref(&folder_id, "stray.md"), "x", Some(&read.hash)).unwrap(), SaveOutcome::Conflict(_)));

    write(&w.dir.join("logs").join("day.md"), "log");
    for file in ["../../settings.json", "logs/day.md", "nope.md", r"..\x.md"] {
        assert!(w.service.read(&w.inputs, &mref(&folder_id, file)).unwrap_err().to_string().contains("no longer there"));
        assert!(w.service.save(&w.inputs, &mref(&folder_id, file), "x", None).unwrap_err().to_string().contains("no longer there"));
    }
    assert!(w.service.read(&w.inputs, &mref("m-000000000000", "stray.md")).unwrap_err().to_string().contains("memory folder is no longer there"));
}

fn read_to(file: &Path) -> String {
    read(file)
}

#[test]
fn makes_a_memory_with_its_line_in_the_index() {
    let w = world(false);
    let folder_id = main_folder(&w);
    let made = w.service.create(&w.inputs, &folder_id, &input("unity-tips", "Unity tips", "what to try when the editor hangs", "reference")).unwrap();
    assert_eq!(made, mref(&folder_id, "unity-tips.md"));
    assert_eq!(
        read(&w.dir.join("unity-tips.md")),
        "---\nname: unity-tips\ndescription: what to try when the editor hangs\nmetadata:\n  node_type: memory\n  type: reference\n  modified: 2026-10-08T10:00:00.000Z\n---\n\nWrite here what Claude should remember.\n"
    );
    assert_eq!(
        read(&w.dir.join("MEMORY.md")),
        "# Memory Index\n\n- [Release steps](release-steps.md) — how a release is cut\n- [Gone](gone.md) — its file was deleted by hand\n- [Unity tips](unity-tips.md) — what to try when the editor hangs\n"
    );
    let err = |i: MemoryCreateInput| w.service.create(&w.inputs, &folder_id, &i).unwrap_err().to_string();
    assert!(err(input("unity-tips", "", "", "reference")).contains("already a memory named \"unity-tips.md\""));
    assert!(err(input("Unity Tips", "", "", "reference")).contains("lower-case"));
    assert!(err(input("memory", "", "", "reference")).contains("index itself"));
    assert_eq!(err(input("ok", "", "", "secret")), "Invalid type");
}

#[test]
fn starts_an_index_for_a_folder_that_had_none_and_keeps_the_line_ends_of_one_that_is_there() {
    let w = world(false);
    let other_folder = folder_of(&w.service, &w.inputs, "p2");
    w.service.create(&w.inputs, &other_folder, &input("first", "First", "why", "user")).unwrap();
    assert_eq!(read(&w.other.join("projects").join(&w.key).join("memory").join("MEMORY.md")), "- [First](first.md) — why\n");

    fs::write(w.dir.join("MEMORY.md"), "- [Release steps](release-steps.md)\r\n").unwrap();
    w.service.create(&w.inputs, &main_folder(&w), &input("second", "Second", "", "user")).unwrap();
    assert_eq!(read(&w.dir.join("MEMORY.md")), "- [Release steps](release-steps.md)\r\n- [Second](second.md)\r\n");
}

#[test]
fn puts_a_deleted_memory_into_the_bin_and_takes_its_line_out_of_the_index() {
    let w = world(false);
    let folder_id = main_folder(&w);
    w.service.delete(&w.inputs, &mref(&folder_id, "release-steps.md")).unwrap();
    assert_eq!(*w.trashed.lock().unwrap(), vec![w.dir.join("release-steps.md")]);
    assert_eq!(read(&w.dir.join("MEMORY.md")), "# Memory Index\n\n- [Gone](gone.md) — its file was deleted by hand\n");

    assert!(w.service.delete(&w.inputs, &mref(&folder_id, "MEMORY.md")).unwrap_err().to_string().contains("is the index of this folder"));
    w.fail_trash.store(true, Ordering::SeqCst);
    let err = w.service.delete(&w.inputs, &mref(&folder_id, "stray.md")).unwrap_err().to_string();
    assert_eq!(err, "The memory could not be moved to the bin (no bin on this drive). It was not deleted.");
    assert!(w.dir.join("stray.md").exists());
}

#[test]
fn searches_the_memories_of_every_project_and_account() {
    let w = world(false);
    let hits = w.service.search(&w.inputs, "INSTALLER");
    let rows: Vec<(String, Vec<String>, String, Option<String>)> = hits.iter().map(|h| (h.file.clone(), h.profile_ids.clone(), h.title.clone(), h.cwd.clone())).collect();
    assert_eq!(
        rows,
        vec![
            ("release-steps.md".to_owned(), vec!["main".to_owned()], "Release steps".to_owned(), Some(s(&w.project))),
            ("own.md".to_owned(), vec!["p2".to_owned()], "own".to_owned(), Some(s(&w.project))),
        ]
    );
    assert!(hits[0].snippet.contains("then build the installer."));
    // the line of the index is part of what is searched
    let files: Vec<String> = w.service.search(&w.inputs, "release cut").into_iter().map(|h| h.file).collect();
    assert_eq!(files, vec!["release-steps.md"]);
    assert_eq!(w.service.search(&w.inputs, "  "), vec![]);
    assert_eq!(w.service.search(&w.inputs, "nothing-like-this"), vec![]);
}

// ---------- sharing the main account's memory ----------

#[test]
fn points_a_session_to_the_main_accounts_folder_for_the_project_it_runs_in() {
    let (_guard, tmp) = temp();
    let main = tmp.join("main");
    let repo = tmp.join("Repo One");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join("src")).unwrap();
    let key = key_of(&repo);
    // the folder need not be there yet
    assert_eq!(shared_memory_dir(&main, &repo.join("src"), None), Ok(main.join("projects").join(&key).join("memory")));
    // when it is, its spelling on disk is kept
    fs::create_dir_all(main.join("projects").join(key.to_uppercase())).unwrap();
    assert_eq!(shared_memory_dir(&main, &repo, None), Ok(main.join("projects").join(key.to_uppercase()).join("memory")));
}

#[test]
fn refuses_a_project_it_cannot_name_and_everything_while_the_names_do_not_follow_the_rule() {
    let (_guard, tmp) = temp();
    let main = tmp.join("main");
    assert!(shared_memory_dir(&main, &tmp.join("a".repeat(120)).join("b".repeat(120)), None).is_err());
    assert_eq!(shared_memory_dir(&main, &tmp.join("app"), Some("The names changed.")), Err("The names changed.".to_owned()));
}

#[test]
fn notices_when_claude_code_names_the_folders_in_another_way() {
    let (_guard, tmp) = temp();
    let main = tmp.join("main");
    let project = tmp.join("My App");
    assert_eq!(naming_problem(&main), None);
    write(&main.join("projects").join(key_of(&project)).join(format!("{UUID}.jsonl")), conversation(&project));
    fs::create_dir_all(main.join("projects").join("no-conversations-here").join("memory")).unwrap();
    assert_eq!(naming_problem(&main), None);

    // one folder that holds a conversation begun elsewhere says nothing while another agrees
    write(&main.join("projects").join("a-name-by-another-rule").join(format!("{UUID}.jsonl")), conversation(&tmp.join("Other App")));
    assert_eq!(naming_problem(&main), None);
    // with no folder left that agrees, the rule has changed
    fs::remove_dir_all(main.join("projects").join(key_of(&project))).unwrap();
    assert!(naming_problem(&main).unwrap().contains("under \"a-name-by-another-rule\""));
}

#[test]
fn keeps_its_look_at_the_names_for_a_while() {
    let (_guard, tmp) = temp();
    let main = tmp.join("main");
    let now = Arc::new(AtomicI64::new(1_000_000));
    let clock = now.clone();
    let sharing = MemorySharing::new(Arc::new(move || clock.load(Ordering::SeqCst)));
    let project = tmp.join("app");
    fs::create_dir_all(&project).unwrap();
    assert!(sharing.dir_for(&main, &project).is_ok());
    write(&main.join("projects").join("odd-name").join(format!("{UUID}.jsonl")), conversation(&project));
    assert!(sharing.dir_for(&main, &project).is_ok());
    now.fetch_add(6 * 60 * 1000, Ordering::SeqCst);
    assert!(sharing.dir_for(&main, &project).is_err());
    fs::remove_dir_all(main.join("projects").join("odd-name")).unwrap();
    assert_eq!(sharing.problem(&main, true), None);
}

fn profile(name: &str, is_default: bool, share_memory: Option<bool>) -> Profile {
    Profile {
        id: name.to_lowercase(),
        name: name.into(),
        slug: name.to_lowercase(),
        color: "#5b9bd5".into(),
        is_default,
        config_dir: None,
        share: ShareConfig::default(),
        email_hint: None,
        auth: None,
        token_enc: None,
        last_sync: None,
        account: None,
        share_memory,
        extra: Default::default(),
    }
}

#[test]
fn gives_a_sharing_accounts_session_the_main_accounts_folder_or_a_warning() {
    let (_guard, tmp) = temp();
    let main = tmp.join("main");
    let own = tmp.join("own");
    let project = tmp.join("app");
    fs::create_dir_all(&project).unwrap();
    let service = MemoryService::with_parts(None, Arc::new(|| OCT_8));
    let expected = main.join("projects").join(key_of(&project)).join("memory");
    assert_eq!(service.auto_memory_directory(&main, &profile("Main", true, Some(true)), &project), Ok(None));
    assert_eq!(service.auto_memory_directory(&main, &profile("Work", false, None), &project), Ok(None));
    assert_eq!(service.auto_memory_directory(&main, &profile("Work", false, Some(true)), &project), Ok(Some(expected.clone())));
    assert_eq!(
        service.auto_memory_directory(&main, &profile("Work", false, Some(true)), Path::new("relative")),
        Err("Work uses its own memory in this folder. The folder is not an absolute path.".to_owned())
    );
    // the folder a session reads MEMORY.md from: the shared one, else the account's own
    assert_eq!(service.session_memory_dir(&main, &profile("Work", false, Some(true)), &own, &project), Some(expected));
    let own_dir = own.join("projects").join(key_of(&project)).join("memory");
    assert_eq!(service.session_memory_dir(&main, &profile("Work", false, None), &own, &project), Some(own_dir.clone()));
    assert_eq!(service.session_memory_dir(&main, &profile("Main", true, None), &own, &project), Some(own_dir));
    assert_eq!(service.session_memory_dir(&main, &profile("Work", false, None), &own, Path::new("relative")), None);
}

// ---------- bringing memories into the shared folder ----------

fn body(name: &str, text: &str) -> String {
    format!("---\nname: {name}\ndescription: About {name}\nmetadata:\n  type: project\n---\n\n{text}\n")
}

fn adopt_folders(tmp: &Path) -> (PathBuf, PathBuf) {
    let source = tmp.join("elif").join("memory");
    let target = tmp.join("main").join("memory");
    write(&source.join("MEMORY.md"), "- [Own note](own.md) — only Elif has it\n- [Shared topic](topic.md) — both wrote about it\n");
    write(&source.join("own.md"), body("own", "Elif wrote this."));
    write(&source.join("topic.md"), body("topic", "What Elif knows."));
    write(&source.join("same.md"), body("same", "Word for word.").replace('\n', "\r\n"));
    write(&source.join("unlisted.md"), body("unlisted", "Not in her index."));
    write(&target.join("MEMORY.md"), "# Memory Index\n\n- [Shared topic](topic.md) — the main account on it\n");
    write(&target.join("topic.md"), body("topic", "What the main account knows."));
    write(&target.join("same.md"), body("same", "Word for word."));
    (source, target)
}

fn snapshot(dir: &Path) -> BTreeMap<String, String> {
    fs::read_dir(dir).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), read(&e.path()))).collect()
}

fn copy(from: &str, to: &str) -> AdoptCopy {
    AdoptCopy { from: from.into(), to: to.into() }
}

#[test]
fn says_what_would_happen_without_touching_anything() {
    let (_guard, tmp) = temp();
    let (source, target) = adopt_folders(&tmp);
    let before = (snapshot(&source), snapshot(&target));
    let plan = adopt_memories(&source, &target, "elif", "Elif", false).unwrap();
    assert_eq!(
        plan,
        AdoptResult {
            copy: vec![copy("own.md", "own.md"), copy("topic.md", "topic.from-elif.md"), copy("unlisted.md", "unlisted.md")],
            same: vec!["same.md".into()],
            target: s(&target),
            applied: false,
        }
    );
    assert_eq!((snapshot(&source), snapshot(&target)), before);
}

#[test]
fn copies_the_files_lists_them_in_the_index_and_leaves_the_source_and_the_files_that_were_there_alone() {
    let (_guard, tmp) = temp();
    let (source, target) = adopt_folders(&tmp);
    let source_before = snapshot(&source);
    let target_before = snapshot(&target);
    let result = adopt_memories(&source, &target, "elif", "Elif", true).unwrap();
    assert!(result.applied);
    assert_eq!(snapshot(&source), source_before);
    let after = snapshot(&target);
    assert_eq!(after.keys().cloned().collect::<Vec<_>>(), vec!["MEMORY.md", "own.md", "same.md", "topic.from-elif.md", "topic.md", "unlisted.md"]);
    assert_eq!(after["topic.md"], target_before["topic.md"]);
    assert_eq!(after["same.md"], target_before["same.md"]);
    assert_eq!(after["topic.from-elif.md"], source_before["topic.md"]);
    assert_eq!(
        after["MEMORY.md"],
        "# Memory Index\n\n- [Shared topic](topic.md) — the main account on it\n- [Own note](own.md) — only Elif has it\n- [Shared topic (from Elif)](topic.from-elif.md) — both wrote about it\n- [unlisted](unlisted.md) — About unlisted\n"
    );
    // a second time there is nothing left to bring
    let again = adopt_memories(&source, &target, "elif", "Elif", true).unwrap();
    assert_eq!((again.copy, again.same), (vec![], vec!["own.md".to_owned(), "same.md".into(), "topic.md".into(), "unlisted.md".into()]));
    assert_eq!(snapshot(&target), after);
}

#[test]
fn keeps_the_line_ends_and_byte_order_mark_of_the_targets_index() {
    let (_guard, tmp) = temp();
    let (source, target) = adopt_folders(&tmp);
    fs::write(target.join("MEMORY.md"), "\u{feff}# Memory Index\r\n").unwrap();
    adopt_memories(&source, &target, "elif", "Elif", true).unwrap();
    let index = read(&target.join("MEMORY.md"));
    assert!(index.starts_with("\u{feff}# Memory Index\r\n\r\n- [Own note](own.md) — only Elif has it\r\n"), "{index:?}");
}

#[test]
fn makes_the_folder_and_its_index_when_the_main_account_has_none_for_the_project() {
    let (_guard, tmp) = temp();
    let (source, _) = adopt_folders(&tmp);
    let target = tmp.join("main").join("projects").join("C--new").join("memory");
    adopt_memories(&source, &target, "Elif Hesabı", "Elif", true).unwrap();
    assert_eq!(snapshot(&target).keys().cloned().collect::<Vec<_>>(), vec!["MEMORY.md", "own.md", "same.md", "topic.md", "unlisted.md"]);
    assert_eq!(read(&target.join("MEMORY.md")).split('\n').next(), Some("- [Own note](own.md) — only Elif has it"));
}

#[test]
fn goes_through_the_service_for_the_folder_of_an_account_that_is_not_the_main_one() {
    let (_guard, tmp) = temp();
    let main_dir = tmp.join("main");
    let other_dir = tmp.join("other");
    write(&main_dir.join("projects").join("C--App").join("memory").join("a.md"), body("a", "Main."));
    write(&other_dir.join("projects").join("c--app").join("memory").join("b.md"), body("b", "Other."));
    let service = MemoryService::with_parts(None, Arc::new(|| OCT_8));
    let inputs = MemoryInputs {
        accounts: vec![account("main", "Main", "main", &main_dir, true, false), account("p2", "Second", "second", &other_dir, false, true)],
        folders: vec![],
    };
    let main_folder = folder_of(&service, &inputs, "main");
    assert!(service.adopt(&inputs, &main_folder, true).unwrap_err().to_string().contains("main account's memory already"));
    let result = service.adopt(&inputs, &folder_of(&service, &inputs, "p2"), true).unwrap();
    // the main account's folder for that project, as it is spelled on disk
    assert_eq!(result.copy, vec![copy("b.md", "b.md")]);
    assert_eq!(result.target, s(&main_dir.join("projects").join("C--App").join("memory")));
    assert!(result.applied);
    assert!(main_dir.join("projects").join("C--App").join("memory").join("b.md").exists());
    assert!(other_dir.join("projects").join("c--app").join("memory").join("b.md").exists());
}
