//! The `SkillService` tests of `test/skillFiles.test.ts` and `test/skillArchive.test.ts` (pack,
//! readArchive, unpack, entryProblem, listArchive), plus importing and Ask Claude through the
//! service.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mc_core::text_file::content_hash;
use mc_core::types::{IssueLevel, IssueScope, Place, SaveOutcome, SessionInfo, SessionRequest, SkillFile, SkillIssue, SkillRef, SkillRootKind, TransferMode};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::archive::{entry_problem, list_archive, pack_skill, read_archive, unpack_skill};
use super::store::{AssistRequest, FileAction, SkillDeps, SkillService};
use crate::library::roots::RootSpec;
use crate::library::test_support::link;
use crate::library::trash::TrashFn;

fn write(file: &Path, content: impl AsRef<[u8]>) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn skill_text(name: &str) -> String {
    skill_text_with(name, &format!("Does {name}."))
}

fn skill_text_with(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n")
}

fn add_skill(root: &Path, dir_name: &str, text: &str) -> PathBuf {
    write(&root.join(dir_name).join("SKILL.md"), text);
    root.join(dir_name)
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

fn sref(root_id: &str, dir_name: &str) -> SkillRef {
    SkillRef { root_id: root_id.into(), dir_name: dir_name.into() }
}

fn read(file: &Path) -> String {
    fs::read_to_string(file).unwrap()
}

// ---------- the world of test/skillFiles.test.ts ----------

struct World {
    _tmp: tempfile::TempDir,
    tmp: PathBuf,
    service: SkillService,
    personal: RootSpec,
    project: RootSpec,
    empty: RootSpec,
    trashed: Arc<Mutex<Vec<PathBuf>>>,
    fail_trash: Arc<AtomicBool>,
}

fn recording_trash(trashed: Arc<Mutex<Vec<PathBuf>>>, fail: Arc<AtomicBool>) -> TrashFn {
    Arc::new(move |dir: &Path| {
        if fail.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("no bin on this drive"));
        }
        trashed.lock().unwrap().push(dir.to_path_buf());
        fs::remove_dir_all(dir)
    })
}

fn spec(id: &str, kind: SkillRootKind, dir: PathBuf, profile_ids: &[&str], folder: Option<PathBuf>, exists: bool, pinned: bool) -> RootSpec {
    RootSpec { id: id.into(), kind, real: dir.clone(), dir, profile_ids: profile_ids.iter().map(|s| (*s).to_owned()).collect(), folder, exists, pinned }
}

fn world() -> World {
    let guard = tempfile::tempdir().unwrap();
    let tmp = crate::library::roots::real_of(guard.path());
    let personal_dir = tmp.join("home").join(".claude").join("skills");
    let project_dir = tmp.join("app").join(".claude").join("skills");
    fs::create_dir_all(&personal_dir).unwrap();
    fs::create_dir_all(&project_dir).unwrap();
    let personal = spec("p-1", SkillRootKind::Personal, personal_dir, &["main", "elif"], None, true, false);
    let project = spec("j-1", SkillRootKind::Project, project_dir, &[], Some(tmp.join("app")), true, false);
    let empty_dir = tmp.join("other").join(".claude").join("skills");
    let empty = spec("j-2", SkillRootKind::Project, empty_dir, &[], Some(tmp.join("other")), false, true);
    let trashed = Arc::new(Mutex::new(Vec::new()));
    let fail_trash = Arc::new(AtomicBool::new(false));
    let roots = vec![personal.clone(), project.clone(), empty.clone()];
    let service = SkillService::with_deps(SkillDeps {
        roots: Box::new(move || roots.clone()),
        trash: recording_trash(trashed.clone(), fail_trash.clone()),
        account_names: Box::new(|ids| ids.join(", ")),
        requests_dir: tmp.join("skill-requests"),
    });
    World { _tmp: guard, tmp, service, personal, project, empty, trashed, fail_trash }
}

fn err_of<T: std::fmt::Debug>(result: mc_core::error::AppResult<T>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn lists_the_skills_of_every_root_with_what_the_list_shows() {
    let w = world();
    add_skill(&w.personal.dir, "review", "---\nname: review\ndescription: Reviews code.\ndisable-model-invocation: true\n---\nbody\n");
    add_skill(&w.personal.dir, "broken", "---\nname: broken\ndescription: Use when: it rains\n---\n");
    add_skill(&w.project.dir, "review", &skill_text("review"));
    add_skill(&w.project.dir, "no-frontmatter", "# First line says it\n\nmore\n");
    let listing = w.service.list();
    assert_eq!(listing.roots.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["p-1", "j-1", "j-2"]);
    assert!(serde_json::to_value(&listing.roots[0]).unwrap().get("real").is_none());
    let keys: Vec<String> = listing.skills.iter().map(|s| format!("{}/{}", s.root_id, s.dir_name)).collect();
    assert_eq!(keys, ["p-1/broken", "p-1/review", "j-1/no-frontmatter", "j-1/review"]);
    let by = |key: &str| listing.skills.iter().find(|s| format!("{}/{}", s.root_id, s.dir_name) == key).unwrap();
    let review = by("p-1/review");
    assert_eq!((review.name.as_str(), review.description.as_str()), ("review", "Reviews code."));
    assert_eq!((review.model_invocable, review.user_invocable, review.linked), (false, true, false));
    assert!(review.issues.is_empty());
    assert_eq!(by("p-1/broken").issues.iter().map(|i| i.level).collect::<Vec<_>>(), [IssueLevel::Error]);
    let plain = by("j-1/no-frontmatter");
    assert_eq!((plain.name.as_str(), plain.description.as_str()), ("no-frontmatter", "First line says it"));
    // the personal skill of the same name is the one Claude Code uses
    assert_eq!(
        by("j-1/review").issues,
        [SkillIssue {
            level: IssueLevel::Warning,
            scope: IssueScope::Claude,
            clash: true,
            message: "The personal skill \"review\" (main, elif) has the same name and is used instead of this one.".into()
        }]
    );
}

#[test]
fn lists_a_skill_whose_file_cannot_be_read() {
    let w = world();
    write(&w.personal.dir.join("utf16").join("SKILL.md"), [0xff, 0xfe, 0x61, 0x00]);
    let listing = w.service.list();
    let skill = &listing.skills[0];
    assert_eq!((skill.name.as_str(), skill.description.as_str()), ("utf16", ""));
    assert_eq!(skill.issues[0].level, IssueLevel::Warning);
    assert_eq!(skill.issues[0].message, "The file is stored as UTF-16; this editor reads UTF-8.");
}

#[test]
fn reads_a_skill_with_its_other_files() {
    let w = world();
    let dir = add_skill(&w.personal.dir, "review", &skill_text("review"));
    write(&dir.join("references").join("rules.md"), "rules");
    let detail = w.service.read(&sref("p-1", "review")).unwrap();
    assert_eq!((detail.root_id.as_str(), detail.dir_name.as_str(), detail.text.as_str()), ("p-1", "review", skill_text("review").as_str()));
    assert_eq!((PathBuf::from(&detail.path), detail.linked, detail.more_files), (dir, false, false));
    assert_eq!(detail.files, [SkillFile { path: "references/rules.md".into(), size: 5 }]);
    assert_eq!(detail.hash, content_hash(skill_text("review").as_bytes()));
    assert_eq!(detail.read_only, None);
}

#[test]
fn accepts_only_folders_the_root_really_lists() {
    let w = world();
    add_skill(&w.personal.dir, "review", &skill_text("review"));
    add_skill(&w.personal.dir.join("synced").join("bucket"), "x", &skill_text("x"));
    write(&w.tmp.join("home").join(".claude").join("SKILL.md"), "outside");
    for dir_name in ["..", ".", "synced", "synced/bucket/x", "review/..", "REVIEW ", "missing", ""] {
        assert!(err_of(w.service.read(&sref("p-1", dir_name))).contains("no longer there"), "{dir_name}");
    }
    assert!(err_of(w.service.read(&sref("nope", "review"))).contains("no longer in the list"));
}

#[test]
fn saves_over_the_state_it_started_from_and_keeps_how_the_file_was_stored() {
    let w = world();
    let dir = w.project.dir.join("review");
    write(&dir.join("SKILL.md"), "\u{feff}---\r\nname: review\r\n---\r\nold\r\n");
    let r = sref("j-1", "review");
    let before = w.service.read(&r).unwrap();
    assert_eq!(before.text, "---\nname: review\n---\nold\n");
    let SaveOutcome::Saved(saved) = w.service.save(&r, "---\nname: review\n---\nnew\n", Some(&before.hash)).unwrap() else { panic!("conflict") };
    assert_eq!(saved.text, "---\nname: review\n---\nnew\n");
    assert_eq!(fs::read(dir.join("SKILL.md")).unwrap(), "\u{feff}---\r\nname: review\r\n---\r\nnew\r\n".as_bytes());
    assert_eq!(names(&dir), ["SKILL.md"]);
}

#[test]
fn does_not_overwrite_a_file_that_changed_since_it_was_read_unless_told_to() {
    let w = world();
    let dir = add_skill(&w.personal.dir, "review", &skill_text("review"));
    let r = sref("p-1", "review");
    let before = w.service.read(&r).unwrap();
    // a Claude session edits the file in the meantime
    let theirs = skill_text_with("review", "Changed by Claude.");
    fs::write(dir.join("SKILL.md"), &theirs).unwrap();
    let SaveOutcome::Conflict(on_disk) = w.service.save(&r, "mine\n", Some(&before.hash)).unwrap() else { panic!("saved") };
    assert_eq!(on_disk.text, theirs);
    assert_eq!(read(&dir.join("SKILL.md")), theirs);
    assert!(matches!(w.service.save(&r, "mine\n", None).unwrap(), SaveOutcome::Saved(_)));
    assert_eq!(read(&dir.join("SKILL.md")), "mine\n");
    assert!(err_of(w.service.save(&r, &"x".repeat(1_000_001), None)).contains("too large"));
}

#[test]
fn creates_a_skill_also_the_first_one_of_a_project() {
    let w = world();
    let r = w.service.create("j-2", "release-notes", "Writes release notes: use before a release.").unwrap();
    assert_eq!(r, sref("j-2", "release-notes"));
    assert!(read(&w.empty.dir.join("release-notes").join("SKILL.md")).contains("name: release-notes\ndescription: \"Writes release notes: use before a release.\""));
    assert_eq!(names(&w.empty.dir), ["release-notes"]);
    assert!(err_of(w.service.create("j-2", "Release-Notes", "")).contains("lower-case"));
    assert!(err_of(w.service.create("j-2", "release-notes", "")).contains("already a skill named \"release-notes\""));
    for bad in ["synced", "nul", "../x", "a b", ""] {
        assert!(w.service.create("p-1", bad, "").is_err(), "{bad}");
    }
    assert!(names(&w.personal.dir).is_empty());
}

#[test]
fn duplicates_a_skill_with_its_files_under_a_name_of_its_own() {
    let w = world();
    let dir = add_skill(&w.personal.dir, "review", &skill_text("review"));
    write(&dir.join("scripts").join("check.py"), "pass");
    let copy = w.service.duplicate(&sref("p-1", "review"), "review-fast").unwrap();
    assert_eq!(copy, sref("p-1", "review-fast"));
    let copied = w.personal.dir.join("review-fast");
    assert_eq!(read(&copied.join("SKILL.md")), skill_text("review").replace("name: review", "name: review-fast"));
    assert_eq!(read(&copied.join("scripts").join("check.py")), "pass");
    assert_eq!(read(&dir.join("SKILL.md")), skill_text("review"));
    assert!(err_of(w.service.duplicate(&sref("p-1", "review"), "REVIEW")).contains("lower-case"));
    assert!(err_of(w.service.duplicate(&sref("p-1", "review"), "review")).contains("already a skill"));
}

#[test]
fn renames_the_folder_and_the_name_in_the_file_when_it_was_the_folder_name() {
    let w = world();
    add_skill(&w.personal.dir, "review", &skill_text("review"));
    add_skill(&w.personal.dir, "Odd-Folder", &skill_text("own-name"));
    add_skill(&w.personal.dir, "other", &skill_text("other"));
    let moved = w.service.rename(&sref("p-1", "review"), "code-review").unwrap();
    assert_eq!((moved.r#ref, moved.warning), (sref("p-1", "code-review"), None));
    assert_eq!(read(&w.personal.dir.join("code-review").join("SKILL.md")), skill_text("review").replace("name: review", "name: code-review"));
    // only the case changes: that is the same folder to Windows, and still allowed
    assert_eq!(w.service.rename(&sref("p-1", "Odd-Folder"), "odd-folder").unwrap().r#ref.dir_name, "odd-folder");
    assert_eq!(names(&w.personal.dir), ["code-review", "odd-folder", "other"]);
    assert_eq!(read(&w.personal.dir.join("odd-folder").join("SKILL.md")), skill_text("own-name"));
    assert!(err_of(w.service.rename(&sref("p-1", "other"), "code-review")).contains("already a skill"));
    assert!(err_of(w.service.rename(&sref("p-1", "other"), "Bad Name")).contains("lower-case"));
    // the same name changes nothing
    assert_eq!(w.service.rename(&sref("p-1", "other"), "other").unwrap().r#ref, sref("p-1", "other"));
}

#[test]
fn puts_a_deleted_skill_into_the_bin_and_never_deletes_it_for_good() {
    let w = world();
    let dir = add_skill(&w.personal.dir, "review", &skill_text("review"));
    w.service.remove(&sref("p-1", "review")).unwrap();
    assert_eq!(*w.trashed.lock().unwrap(), [dir]);

    let kept = add_skill(&w.personal.dir, "keep", &skill_text("keep"));
    w.fail_trash.store(true, Ordering::SeqCst);
    let err = err_of(w.service.remove(&sref("p-1", "keep")));
    assert_eq!(err, "The skill could not be moved to the bin (no bin on this drive). It was not deleted.");
    assert!(kept.join("SKILL.md").exists());
}

#[test]
fn removes_only_the_link_of_a_skill_that_lives_elsewhere() {
    let w = world();
    let elsewhere = add_skill(&w.tmp.join("repo"), "shared", &skill_text("shared"));
    link(&elsewhere, &w.personal.dir.join("shared"));
    assert!(w.service.read(&sref("p-1", "shared")).unwrap().linked);
    assert!(err_of(w.service.transfer(&sref("p-1", "shared"), "j-1", TransferMode::Move, "shared")).contains("is a link"));
    w.service.remove(&sref("p-1", "shared")).unwrap();
    assert!(w.trashed.lock().unwrap().is_empty());
    assert!(names(&w.personal.dir).is_empty());
    assert!(elsewhere.join("SKILL.md").exists());
}

#[test]
fn copies_a_skill_to_a_project_and_moves_one_back() {
    let w = world();
    let dir = add_skill(&w.personal.dir, "review", &skill_text("review"));
    write(&dir.join("references").join("rules.md"), "rules");
    let copied = w.service.transfer(&sref("p-1", "review"), "j-2", TransferMode::Copy, "review").unwrap();
    assert_eq!((copied.r#ref, copied.warning), (sref("j-2", "review"), None));
    assert_eq!(read(&w.empty.dir.join("review").join("references").join("rules.md")), "rules");
    assert!(dir.exists());
    assert!(w.trashed.lock().unwrap().is_empty());

    let moved = w.service.transfer(&sref("j-2", "review"), "p-1", TransferMode::Move, "review-from-project").unwrap();
    assert_eq!((moved.r#ref, moved.warning), (sref("p-1", "review-from-project"), None));
    assert_eq!(*w.trashed.lock().unwrap(), [w.empty.dir.join("review")]);
    assert!(read(&w.personal.dir.join("review-from-project").join("SKILL.md")).contains("name: review-from-project"));

    assert!(w.service.transfer(&sref("p-1", "review"), "j-2", TransferMode::Copy, "review").is_ok());
    assert!(err_of(w.service.transfer(&sref("p-1", "review"), "j-2", TransferMode::Copy, "review")).contains("already a skill"));
    assert!(err_of(w.service.transfer(&sref("p-1", "review"), "p-1", TransferMode::Copy, "review")).contains("in that folder already"));
    assert!(err_of(w.service.transfer(&sref("p-1", "review"), "j-1", TransferMode::Copy, "synced")).contains("Claude Code keeps"));
}

#[test]
fn says_so_when_a_move_could_only_copy() {
    let w = world();
    add_skill(&w.personal.dir, "review", &skill_text("review"));
    w.fail_trash.store(true, Ordering::SeqCst);
    let moved = w.service.transfer(&sref("p-1", "review"), "j-1", TransferMode::Move, "review").unwrap();
    assert_eq!(moved.r#ref, sref("j-1", "review"));
    assert!(moved.warning.unwrap().contains("the original is still there"));
    assert!(w.personal.dir.join("review").join("SKILL.md").exists());
}

#[test]
fn opens_text_files_of_a_skill_and_only_shows_the_others() {
    let w = world();
    let dir = add_skill(&w.personal.dir, "review", &skill_text("review"));
    write(&dir.join("refs").join("rules.MD"), "rules");
    write(&dir.join("scripts").join("run.py"), "print(1)");
    let r = sref("p-1", "review");
    assert_eq!(w.service.file_action(&r, "refs/rules.MD").unwrap(), FileAction::Open(dir.join("refs").join("rules.MD")));
    assert_eq!(w.service.file_action(&r, "scripts/run.py").unwrap(), FileAction::Reveal(dir.join("scripts").join("run.py")));
    assert_eq!(w.service.file_of(&r, "SKILL.md").unwrap(), dir.join("SKILL.md"));
    // only what the skill's listing has
    for file in ["../review/SKILL.md", "refs/../SKILL.md", "missing.md", "refs"] {
        assert_eq!(err_of(w.service.file_of(&r, file)), "That file is no longer there.", "{file}");
    }
}

// ---------- archives (test/skillArchive.test.ts) ----------

const SKILL: &str = "---\nname: review\ndescription: Reviews code.\n---\n\n# Review\n";

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (name, content) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let bytes: Vec<(&str, &[u8])> = entries.iter().map(|(n, c)| (*n, c.as_bytes())).collect();
    zip_of(&bytes)
}

fn unzipped(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        out.push((file.name().to_owned(), data));
    }
    out.sort();
    out
}

#[test]
fn packs_the_skill_under_one_top_folder_and_leaves_out_what_tools_fill_by_themselves() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("review");
    write(&dir.join("SKILL.md"), SKILL);
    write(&dir.join("references").join("rules.md"), "rules");
    write(&dir.join("scripts").join("check.py"), "pass");
    write(&dir.join("scripts").join("__pycache__").join("check.cpython-312.pyc"), "x");
    write(&dir.join("scripts").join("old.pyc"), "x");
    write(&dir.join("node_modules").join("dep").join("index.js"), "x");
    write(&dir.join(".DS_Store"), "x");
    write(&dir.join("evals").join("run-1.json"), "{}");
    // only the folder at the top is the skill's own test runs
    write(&dir.join("references").join("evals").join("notes.md"), "kept");
    let packed = unzipped(&pack_skill(&dir, "review").unwrap());
    let keys: Vec<&str> = packed.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(keys, ["review/SKILL.md", "review/references/evals/notes.md", "review/references/rules.md", "review/scripts/check.py"]);
    assert_eq!(packed[0].1, SKILL.as_bytes());
}

#[test]
fn comes_back_as_the_same_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("review");
    write(&dir.join("SKILL.md"), SKILL);
    write(&dir.join("references").join("rules.md"), "rules");
    let archive = read_archive(&pack_skill(&dir, "review").unwrap(), "whatever.zip").unwrap();
    assert_eq!(archive.name, "review");
    assert_eq!(archive.text.as_deref(), Some(SKILL));
    assert_eq!(list_archive(&archive, 300), (vec![SkillFile { path: "references/rules.md".into(), size: 5 }], false));
    assert_eq!(list_archive(&archive, 0), (vec![], true));
}

#[test]
fn reads_a_skill_that_sits_at_the_top_of_its_archive_and_names_it() {
    assert_eq!(read_archive(&zip(&[("SKILL.md", SKILL), ("scripts/a.py", "x")]), "My Download.zip").unwrap().name, "review");
    assert_eq!(read_archive(&zip(&[("SKILL.md", "# no frontmatter\n")]), "Release Notes (2).skill").unwrap().name, "release-notes-2");
    assert_eq!(read_archive(&zip(&[("SKILL.md", "# no frontmatter\n")]), "!!!.zip").unwrap().name, "imported-skill");
    // a top folder that is no valid name gives way to the name in the file
    assert_eq!(read_archive(&zip(&[("My Skill/SKILL.md", SKILL)]), "x.zip").unwrap().name, "review");
    // and when nothing is a valid name, the folder is made into one
    assert_eq!(read_archive(&zip(&[("My Skill/SKILL.md", "# x\n")]), "!!!.ZIP").unwrap().name, "my-skill");
}

#[test]
fn ignores_what_macos_adds_to_its_archives() {
    let archive = read_archive(&zip(&[("review/SKILL.md", SKILL), ("__MACOSX/review/._SKILL.md", "x"), ("review/.DS_Store", "x"), (".DS_Store", "x")]), "r.zip").unwrap();
    assert_eq!(archive.files.iter().map(|(rel, _)| rel.as_str()).collect::<Vec<_>>(), ["SKILL.md"]);
}

#[test]
fn refuses_paths_that_would_leave_the_skill_folder_or_that_windows_cannot_hold() {
    let refuse = |extra: &[(&str, &str)]| -> String {
        let mut entries = vec![("review/SKILL.md", SKILL)];
        entries.extend_from_slice(extra);
        match read_archive(&zip(&entries), "r.zip") {
            Ok(_) => "accepted".to_owned(),
            Err(err) => err.to_string(),
        }
    };
    assert!(refuse(&[("review/../../evil.txt", "x")]).contains("leaves the skill folder"));
    assert!(refuse(&[("../evil.txt", "x")]).contains("leaves the skill folder"));
    assert!(refuse(&[("/etc/passwd", "x")]).contains("absolute"));
    assert!(refuse(&[("C:/Windows/x.txt", "x")]).contains("absolute"));
    assert!(refuse(&[("review/nul.txt", "x")]).contains("Windows keeps"));
    assert!(refuse(&[("review/name.", "x")]).contains("ends with a dot"));
    assert!(refuse(&[("review/a?b.txt", "x")]).contains("not allowed"));
    assert!(refuse(&[("review/readme.md", "x"), ("review/README.md", "y")]).contains("twice"));
    let deep = format!("review/{}x.txt", "d/".repeat(12));
    assert!(refuse(&[(deep.as_str(), "x")]).contains("nested too deep"));
    let long = format!("review/{}.txt", "n".repeat(200));
    assert!(refuse(&[(long.as_str(), "x")]).contains("too long"));
    // a backslash is a folder too, and has to pass the same rules
    assert!(refuse(&[("review\\..\\evil.txt", "x")]).contains("leaves the skill folder"));
    assert_eq!(refuse(&[("review/fine.txt", "x")]), "accepted");
}

#[test]
fn wants_exactly_one_skill() {
    let err = |bytes: Vec<u8>| read_archive(&bytes, "r.zip").unwrap_err().to_string();
    assert!(err(zip(&[("readme.md", "x")])).contains("no SKILL.md"));
    assert!(err(zip(&[("a/SKILL.md", SKILL), ("b/SKILL.md", SKILL)])).contains("several skills (a, b)"));
    assert!(err(zip(&[("SKILL.md", SKILL), ("b/SKILL.md", SKILL)])).contains("several skills (one at the top, b)"));
    assert!(err(zip(&[("a/SKILL.md", SKILL), ("other/readme.md", "x")])).contains("more than the one skill folder"));
    assert!(err(b"this is not a zip".to_vec()).contains("not a zip archive"));
    // a skill at the top takes the folders next to it along
    let archive = read_archive(&zip(&[("SKILL.md", SKILL), ("refs/a.md", "a")]), "r.zip").unwrap();
    assert_eq!(archive.files.iter().map(|(rel, _)| rel.as_str()).collect::<Vec<_>>(), ["SKILL.md", "refs/a.md"]);
}

#[test]
fn stops_at_its_limits_before_unpacking() {
    let mut many: Vec<(String, String)> = vec![("review/SKILL.md".into(), SKILL.into())];
    many.extend((0..2000).map(|i| (format!("review/f{i}.txt"), String::new())));
    let entries: Vec<(&str, &str)> = many.iter().map(|(n, c)| (n.as_str(), c.as_str())).collect();
    assert!(read_archive(&zip(&entries), "r.zip").unwrap_err().to_string().contains("more than 2000 files"));
    let big = vec![0u8; 21 * 1024 * 1024];
    let bytes = zip_of(&[("review/SKILL.md", SKILL.as_bytes()), ("review/big.bin", &big)]);
    assert!(read_archive(&bytes, "r.zip").unwrap_err().to_string().contains("larger than 20 MB"));
    assert_eq!(read_archive(&vec![0u8; 50 * 1024 * 1024 + 1], "r.zip").unwrap_err().to_string(), "The archive is larger than 50 MB.");
}

#[test]
fn names_what_a_path_is_refused_for() {
    assert_eq!(entry_problem("references/rules.md"), None);
    let problem = |entry: &str| entry_problem(entry).unwrap_or_default();
    assert!(problem("a/./b").contains("leaves the skill folder"));
    assert!(problem("a//b").contains("leaves the skill folder"));
    assert!(problem("COM1").contains("Windows keeps"));
    assert!(problem("trailing /x").contains("ends with a dot or a space"));
    assert_eq!(problem("a\u{1}b"), "a name has characters that are not allowed: \"a\\u0001b\"");
    assert_eq!(problem("x/aux.tar.gz"), "\"aux.tar.gz\" is a name Windows keeps for a device");
}

#[test]
fn writes_the_skill_into_a_skills_folder_also_one_that_is_not_there_yet() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("app").join(".claude").join("skills");
    let archive = read_archive(&zip(&[("review/SKILL.md", SKILL), ("review/scripts/check.py", "pass")]), "r.zip").unwrap();
    unpack_skill(&archive, &root, "review").unwrap();
    assert_eq!(names(&root), ["review"]);
    assert_eq!(read(&root.join("review").join("SKILL.md")), SKILL);
    assert_eq!(read(&root.join("review").join("scripts").join("check.py")), "pass");
    assert!(unpack_skill(&archive, &root, "review").unwrap_err().to_string().contains("already a skill named \"review\""));
}

#[test]
fn gives_the_name_in_the_file_the_name_it_is_brought_in_under() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("skills");
    let crlf = SKILL.replace('\n', "\r\n");
    let archive = read_archive(&zip(&[("review/SKILL.md", crlf.as_str())]), "r.zip").unwrap();
    unpack_skill(&archive, &root, "review-imported").unwrap();
    assert_eq!(read(&root.join("review-imported").join("SKILL.md")), SKILL.replace("name: review", "name: review-imported").replace('\n', "\r\n"));
}

#[test]
fn imports_an_archive_it_has_shown_once() {
    let w = world();
    let bytes = zip(&[("review/SKILL.md", SKILL), ("review/refs/a.md", "a")]);
    let preview = w.service.preview_import(&bytes, "review.zip").unwrap();
    assert_eq!((preview.name.as_str(), preview.description.as_str()), ("review", "Reviews code."));
    assert_eq!(preview.files, [SkillFile { path: "refs/a.md".into(), size: 1 }]);
    assert!(!preview.more_files && preview.issues.is_empty());
    assert!(err_of(w.service.import(&preview.token, "j-1", "Bad Name")).contains("lower-case"));
    assert_eq!(w.service.import(&preview.token, "j-1", "review-two").unwrap(), sref("j-1", "review-two"));
    assert!(read(&w.project.dir.join("review-two").join("SKILL.md")).contains("name: review-two"));
    // a token is good once
    assert_eq!(err_of(w.service.import(&preview.token, "j-1", "review-three")), "Choose the archive again.");
    assert_eq!(err_of(w.service.import("unknown", "j-1", "x")), "Choose the archive again.");
}

#[test]
fn shows_an_archive_whose_skill_md_is_not_text() {
    let w = world();
    let bytes = zip_of(&[("odd/SKILL.md", &[0xff, 0xfe, 0x61, 0x00])]);
    let preview = w.service.preview_import(&bytes, "odd.zip").unwrap();
    assert_eq!((preview.name.as_str(), preview.description.as_str()), ("odd", ""));
    assert_eq!(preview.issues[0].message, "Its SKILL.md is not text this app can read.");
}

// ---------- Ask Claude through the service ----------

fn ask<'a>(r: &'a SkillRef, profile_id: &'a str, profile_name: Option<&'a str>, request: &'a str, claude_path: Option<&'a str>) -> AssistRequest<'a> {
    AssistRequest { r, profile_id, profile_name, request, place: None, claude_path }
}

fn session(request: &SessionRequest) -> SessionInfo {
    serde_json::from_value(serde_json::json!({
        "id": "tab1", "profileId": request.profile_id, "kind": "claude", "cwd": request.cwd, "title": "t", "status": "running", "startedAt": 0
    }))
    .unwrap()
}

#[test]
fn asks_claude_about_a_skill_once_everything_is_in_place() {
    let w = world();
    add_skill(&w.personal.dir, "review", &skill_text("review"));
    add_skill(&w.project.dir, "deploy", &skill_text("deploy"));
    let personal = sref("p-1", "review");
    let project = sref("j-1", "deploy");
    let never = |_: &SessionRequest, _: Option<Place>| -> mc_core::error::AppResult<SessionInfo> { panic!("no session") };
    let refused = |a: AssistRequest<'_>| w.service.assist(a, never).err().map(|e| e.to_string()).unwrap_or_default();
    assert_eq!(refused(ask(&personal, "main", Some("Main"), "  ", Some("claude"))), "Say what Claude should do with the skill.");
    assert_eq!(refused(ask(&personal, "main", Some("Main"), "x", None)), "claude was not found. Set its path in Settings.");
    if cfg!(windows) {
        assert!(refused(ask(&personal, "main", Some("Main"), "x", Some(r"C:\npm\claude.cmd"))).contains("native claude executable"));
    }
    assert_eq!(refused(ask(&personal, "gone", None, "x", Some("claude"))), "Profile not found");
    assert_eq!(refused(ask(&personal, "solo", Some("Solo"), "x", Some("claude"))), "\"Solo\" has its own skills folder and does not see this skill.");

    let asked = w.service.assist(ask(&personal, "elif", Some("Elif"), "Shorten it.", Some("claude")), |request, _| Ok(session(request))).unwrap();
    assert_eq!((asked.info.cwd, asked.project), (w.personal.dir.to_string_lossy().into_owned(), None));
    let asked = w.service.assist(ask(&project, "solo", Some("Solo"), "Shorten it.", Some("claude")), |request, _| Ok(session(request))).unwrap();
    let app = w.tmp.join("app").to_string_lossy().into_owned();
    assert_eq!((asked.info.cwd, asked.project), (app.clone(), Some(app)));
    assert_eq!(names(&w.tmp.join("skill-requests")).len(), 2);
}
