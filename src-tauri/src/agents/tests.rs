//! `test/agentFiles.test.ts`, and the roots of `test/skillRoots.test.ts` as agents find them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mc_core::types::{AgentRef, IssueLevel, SkillRootKind, TransferMode};
use tempfile::TempDir;

use super::*;
use crate::config_store::ConfigStore;

fn temp() -> (TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let base = roots::plain(fs::canonicalize(tmp.path()).unwrap());
    (tmp, base)
}

fn write(file: &Path, content: impl AsRef<[u8]>) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn agent_text(name: &str, extra: &str) -> String {
    format!("---\nname: {name}\ndescription: Does {name}.\n{extra}---\n\nYou are {name}.\n")
}

fn add_agent(root: &Path, file: &str, text: Option<&str>) -> PathBuf {
    let abs = scan::join_rel(root, file);
    let stem = file.rsplit('/').next().unwrap().trim_end_matches(".md").to_owned();
    write(&abs, text.map_or_else(|| agent_text(&stem, ""), str::to_owned));
    abs
}

/// A link to a folder: a junction on Windows, which needs no rights.
fn link(target: &Path, at: &Path) {
    #[cfg(windows)]
    junction::create(target, at).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
}

fn tree(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, sub: &str, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir.join(sub)).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().unwrap().is_dir() {
                walk(dir, &format!("{sub}{name}/"), out);
            } else {
                out.push(format!("{sub}{name}"));
            }
        }
    }
    let mut out = Vec::new();
    if dir.exists() {
        walk(dir, "", &mut out);
    }
    out.sort();
    out
}

fn read(file: &Path) -> String {
    fs::read_to_string(file).unwrap()
}

fn err<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    result.unwrap_err().to_string()
}

fn at(root_id: &str, file: &str) -> AgentRef {
    AgentRef { root_id: root_id.to_owned(), file: file.to_owned() }
}

// ---------- findAgents ----------

#[test]
fn lists_the_markdown_files_at_any_depth_and_nothing_else() {
    let (_tmp, base) = temp();
    let root = base.join("agents");
    add_agent(&root, "reviewer.md", None);
    add_agent(&root, "team/qa/tester.md", None);
    add_agent(&root, "team/Lead.MD", None);
    write(&root.join("notes.txt"), "not an agent");
    write(&root.join(".archive").join("old.md"), agent_text("old", ""));
    write(&root.join("node_modules").join("pkg").join("readme.md"), "# readme");
    write(&root.join("__pycache__").join("x.md"), "# x");
    assert_eq!(find_agents(&root), ["reviewer.md", "team/Lead.MD", "team/qa/tester.md"]);
    assert_eq!(find_agents(&base.join("nowhere")), Vec::<String>::new());
}

#[test]
fn walks_a_folder_once_that_is_reached_through_a_link_as_well() {
    let (_tmp, base) = temp();
    let root = base.join("agents");
    add_agent(&root, "team/tester.md", None);
    link(&root.join("team"), &root.join("again"));
    link(&root, &root.join("team").join("loop"));
    assert_eq!(find_agents(&root), ["team/tester.md"]);
}

#[test]
fn goes_six_folders_deep_and_no_further() {
    let (_tmp, base) = temp();
    let root = base.join("agents");
    add_agent(&root, "1/2/3/4/5/6/deep.md", None);
    add_agent(&root, "1/2/3/4/5/6/7/deeper.md", None);
    assert_eq!(find_agents(&root), ["1/2/3/4/5/6/deep.md"]);
}

#[test]
fn lists_at_most_five_hundred_files() {
    let (_tmp, base) = temp();
    let root = base.join("agents");
    for i in 0..(MAX_FILES + 5) {
        write(&root.join(format!("a{i:04}.md")), "x");
    }
    assert_eq!(find_agents(&root).len(), MAX_FILES);
}

// ---------- the roots ----------

fn profile(id: &str, config_dir: &Path) -> ProfileFolder {
    ProfileFolder { id: id.to_owned(), name: id.to_owned(), config_dir: config_dir.to_path_buf() }
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn makes_one_personal_root_of_accounts_that_share_a_folder() {
    let (_tmp, base) = temp();
    let main = base.join("home").join(".claude");
    fs::create_dir_all(main.join("agents")).unwrap();
    let other = base.join("profiles").join("work");
    let inputs = RootInputs {
        profiles: vec![profile("main", &main), profile("elif", &main), profile("work", &other)],
        ..RootInputs::default()
    };
    let roots = find_roots(&inputs);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].root.profile_ids, ["main", "elif"]);
    assert_eq!((roots[0].root.kind, roots[0].root.exists), (SkillRootKind::Personal, true));
    assert_eq!(roots[0].root.dir, text(&main.join("agents")));
    assert_eq!((roots[1].root.profile_ids.as_slice(), roots[1].root.exists), (&["work".to_owned()][..], false));
    for root in &roots {
        let id = &root.root.id;
        assert!(id.len() == 14 && id.starts_with("p-") && id[2..].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{id}");
    }
    assert_ne!(roots[0].root.id, roots[1].root.id);
    // the same folder always has the same id
    assert_eq!(find_roots(&inputs)[0].root.id, roots[0].root.id);
    assert_eq!(inputs.account_names(&roots[0].root.profile_ids), "main, elif");
}

#[test]
fn finds_the_agents_of_a_project_up_to_the_top_of_its_repository() {
    let (_tmp, base) = temp();
    let repo = base.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join(".claude").join("agents")).unwrap();
    fs::create_dir_all(repo.join("app").join(".claude").join("agents")).unwrap();
    fs::create_dir_all(repo.join("app").join("src")).unwrap();
    let loose = base.join("loose");
    fs::create_dir_all(loose.join("inner")).unwrap();
    fs::create_dir_all(loose.join(".claude").join("agents")).unwrap();

    let inputs = RootInputs {
        folders: vec![text(&repo.join("app").join("src")), text(&loose.join("inner")), "relative/folder".to_owned()],
        ..RootInputs::default()
    };
    let roots = find_roots(&inputs);
    let folders: Vec<&str> = roots.iter().map(|r| r.root.folder.as_deref().unwrap()).collect();
    // outside a repository only the folder itself counts, and it has no agents
    assert_eq!(folders, [text(&repo.join("app")), text(&repo)]);
    assert!(roots.iter().all(|r| r.root.kind == SkillRootKind::Project && r.root.exists && r.root.pinned.is_none() && r.root.id.starts_with("j-")));
}

#[test]
fn keeps_a_pinned_folder_in_the_list_without_agents_and_marks_it() {
    let (_tmp, base) = temp();
    let app = base.join("app");
    fs::create_dir_all(app.join(".git")).unwrap();
    fs::create_dir_all(app.join(".claude").join("agents")).unwrap();
    let empty = base.join("empty");
    fs::create_dir_all(empty.join(".git")).unwrap();
    let inputs = RootInputs { folders: vec![text(&app)], pinned: vec![text(&empty), text(&app)], ..RootInputs::default() };
    let roots = find_roots(&inputs);
    assert_eq!(roots.len(), 2);
    assert_eq!((roots[0].root.folder.as_deref(), roots[0].root.exists, roots[0].root.pinned), (Some(text(&empty).as_str()), false, Some(true)));
    assert_eq!((roots[1].root.folder.as_deref(), roots[1].root.exists, roots[1].root.pinned), (Some(text(&app).as_str()), true, Some(true)));
}

#[test]
fn takes_the_home_folder_for_the_personal_root_it_is() {
    let (_tmp, base) = temp();
    let home = base.join("home");
    fs::create_dir_all(home.join(".git")).unwrap();
    fs::create_dir_all(home.join(".claude").join("agents")).unwrap();
    let inputs = RootInputs { profiles: vec![profile("main", &home.join(".claude"))], pinned: vec![text(&home)], ..RootInputs::default() };
    let roots = find_roots(&inputs);
    assert_eq!(roots.len(), 1);
    assert_eq!((roots[0].root.kind, roots[0].root.pinned), (SkillRootKind::Personal, None));
    assert_eq!(project_root_of(&roots, &text(&home)), None);
}

#[test]
fn pins_a_folder_once_keeps_the_last_thirty_and_takes_it_out_again() {
    let (_tmp, base) = temp();
    let config = ConfigStore::open(base.join("config.json"));
    let app = base.join("app");
    fs::create_dir_all(app.join(".git")).unwrap();
    pin_folder(&config, &text(&app)).unwrap();
    pin_folder(&config, &format!("{}{}", text(&app), std::path::MAIN_SEPARATOR)).unwrap();
    assert_eq!(config.settings().skill_folders.unwrap(), [text(&app)]);

    let inputs = RootInputs { pinned: config.settings().skill_folders.unwrap(), ..RootInputs::default() };
    let roots = find_roots(&inputs);
    let id = project_root_of(&roots, &text(&app)).unwrap();
    assert_eq!(id, roots[0].root.id);
    unpin_root(&config, &roots, "j-000000000000").unwrap();
    assert_eq!(config.settings().skill_folders.unwrap().len(), 1);
    unpin_root(&config, &roots, &id).unwrap();
    assert_eq!(config.settings().skill_folders.unwrap(), Vec::<String>::new());

    for i in 0..(PINNED_MAX + 2) {
        pin_folder(&config, &text(&base.join(format!("p{i}")))).unwrap();
    }
    let pinned = config.settings().skill_folders.unwrap();
    assert_eq!(pinned.len(), PINNED_MAX);
    assert_eq!(pinned[0], text(&base.join("p2")));
}

// ---------- AgentService ----------

struct World {
    _tmp: TempDir,
    base: PathBuf,
    service: AgentService,
    inputs: RootInputs,
    /// The personal root (accounts main and elif), the project's and a pinned one without agents.
    personal: (String, PathBuf),
    project: (String, PathBuf),
    empty: (String, PathBuf),
    trashed: Arc<Mutex<Vec<PathBuf>>>,
    fail_trash: Arc<AtomicBool>,
}

fn world() -> World {
    let (tmp, base) = temp();
    let config_dir = base.join("home").join(".claude");
    fs::create_dir_all(config_dir.join("agents")).unwrap();
    let app = base.join("app");
    fs::create_dir_all(app.join(".git")).unwrap();
    fs::create_dir_all(app.join(".claude").join("agents")).unwrap();
    let other = base.join("other");
    fs::create_dir_all(other.join(".git")).unwrap();
    let inputs = RootInputs {
        profiles: vec![profile("main", &config_dir), profile("elif", &config_dir)],
        folders: vec![text(&app)],
        pinned: vec![text(&other)],
    };
    let roots = find_roots(&inputs);
    let id_of = |dir: &Path| roots.iter().find(|r| r.dir == dir).map(|r| (r.root.id.clone(), r.dir.clone())).unwrap();
    let personal = id_of(&config_dir.join("agents"));
    let project = id_of(&app.join(".claude").join("agents"));
    let empty = id_of(&other.join(".claude").join("agents"));

    let trashed: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let fail_trash = Arc::new(AtomicBool::new(false));
    let (into, fail) = (trashed.clone(), fail_trash.clone());
    let trash: TrashFn = Arc::new(move |file: &Path| {
        if fail.load(Ordering::SeqCst) {
            return Err(io::Error::other("no bin on this drive"));
        }
        into.lock().unwrap().push(file.to_path_buf());
        fs::remove_file(file)
    });
    World { _tmp: tmp, base, service: AgentService::with_parts(None, trash), inputs, personal, project, empty, trashed, fail_trash }
}

#[test]
fn lists_the_agents_of_every_root_with_what_the_list_shows() {
    let w = world();
    add_agent(&w.personal.1, "reviewer.md", Some(&agent_text("reviewer", "model: opus\neffort: xhigh\n")));
    add_agent(&w.personal.1, "README.md", Some("# How we write agents\n"));
    add_agent(&w.project.1, "team/code.md", Some(&agent_text("reviewer", "")));
    let listing = w.service.list(&w.inputs);
    assert_eq!(listing.roots.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [w.personal.0.as_str(), &w.empty.0, &w.project.0]);
    assert!(serde_json::to_value(&listing.roots[0]).unwrap().get("real").is_none());
    let keys: Vec<String> = listing.agents.iter().map(|a| format!("{}/{}", a.root_id, a.file)).collect();
    assert_eq!(keys, [format!("{}/README.md", w.personal.0), format!("{}/reviewer.md", w.personal.0), format!("{}/team/code.md", w.project.0)]);

    let reviewer = &listing.agents[1];
    assert_eq!(
        (reviewer.name.as_str(), reviewer.description.as_str(), reviewer.model.as_str(), reviewer.effort.as_str(), reviewer.loads),
        ("reviewer", "Does reviewer.", "opus", "xhigh", true)
    );
    // the project's agent of the same name wins there
    assert_eq!(reviewer.issues.iter().map(|i| (i.level, i.clash)).collect::<Vec<_>>(), [(IssueLevel::Warning, true)]);
    assert!(reviewer.issues[0].message.contains(&format!("The project {} has", text(&w.base.join("app")))));
    let code = &listing.agents[2];
    assert_eq!(code.issues.iter().map(|i| i.level).collect::<Vec<_>>(), [IssueLevel::Info]);
    assert_eq!(code.issues[0].message, "Takes the place of the personal agent of the same name (main, elif) in this project.");
    assert!(code.loads);
    // a file that is no agent is listed, with the reason it does not load
    let readme = &listing.agents[0];
    assert_eq!((readme.name.as_str(), readme.description.as_str(), readme.loads), ("", "How we write agents", false));
}

#[test]
fn lists_a_file_it_cannot_read_with_the_reason() {
    let w = world();
    write(&w.personal.1.join("wide.md"), [0xff, 0xfe, 0x61, 0x00]);
    let listing = w.service.list(&w.inputs);
    let wide = &listing.agents[0];
    assert!(!wide.loads);
    assert_eq!(wide.issues[0].message, "The file is stored as UTF-16; this editor reads UTF-8.");
}

#[test]
fn reads_and_saves_an_agent_against_the_state_it_was_read_in() {
    let w = world();
    let abs = add_agent(&w.personal.1, "reviewer.md", Some(&agent_text("reviewer", "").replace('\n', "\r\n")));
    let agent = at(&w.personal.0, "reviewer.md");
    let detail = w.service.read(&w.inputs, &agent).unwrap();
    assert_eq!((detail.root_id.as_str(), detail.file.as_str()), (w.personal.0.as_str(), "reviewer.md"));
    assert_eq!((detail.text.as_str(), detail.path.as_str()), (agent_text("reviewer", "").as_str(), text(&abs).as_str()));

    let saved = w.service.save(&w.inputs, &agent, &detail.text.replace("You are", "You were"), Some(&detail.hash)).unwrap();
    let SaveOutcome::Saved(saved) = saved else { panic!("conflict") };
    assert!(saved.text.contains("You were reviewer."));
    assert_eq!(read(&abs), agent_text("reviewer", "").replace("You are", "You were").replace('\n', "\r\n"));

    // a session changed the file meanwhile
    fs::write(&abs, agent_text("reviewer", "model: haiku\n")).unwrap();
    let SaveOutcome::Conflict(on_disk) = w.service.save(&w.inputs, &agent, "mine", Some(&detail.hash)).unwrap() else { panic!("saved") };
    assert!(on_disk.text.contains("model: haiku"));
    assert!(read(&abs).contains("model: haiku"));
}

#[test]
fn accepts_only_files_the_root_really_lists() {
    let w = world();
    add_agent(&w.personal.1, "reviewer.md", None);
    let secret = w.base.join("home").join(".claude").join("CLAUDE.md");
    write(&secret, "secret");
    for file in ["../CLAUDE.md", "nope.md", "reviewer.md/..", "..\\CLAUDE.md", "./reviewer.md", "REVIEWER.md"] {
        let agent = at(&w.personal.0, file);
        assert!(err(w.service.read(&w.inputs, &agent)).contains("no longer there"), "{file}");
        assert!(err(w.service.save(&w.inputs, &agent, "x", None)).contains("no longer there"), "{file}");
        assert!(err(w.service.remove(&w.inputs, &agent)).contains("no longer there"), "{file}");
    }
    assert!(err(w.service.read(&w.inputs, &at("p-999999999999", "reviewer.md"))).contains("no longer in the list"));
    assert_eq!(read(&secret), "secret");
}

#[test]
fn creates_an_agent_also_the_first_one_of_a_project() {
    let w = world();
    let made = w.service.create(&w.inputs, &w.empty.0, "log-reader", "Reads logs.").unwrap();
    assert_eq!(made, at(&w.empty.0, "log-reader.md"));
    let made_text = read(&w.empty.1.join("log-reader.md"));
    assert!(made_text.starts_with("---\nname: log-reader\ndescription: Reads logs.\n---\n\n"), "{made_text}");
    // the root keeps its id once its folder is there
    assert_eq!(find_roots(&w.inputs).iter().find(|r| r.dir == w.empty.1).unwrap().root.id, w.empty.0);

    assert!(err(w.service.create(&w.inputs, &w.empty.0, "log-reader", "")).contains("already an agent named \"log-reader\""));
    assert!(err(w.service.create(&w.inputs, &w.empty.0, "Log Reader", "")).contains("lower-case"));
    // the name is free, the file is not
    add_agent(&w.empty.1, "taken.md", Some(&agent_text("something-else", "")));
    assert!(err(w.service.create(&w.inputs, &w.empty.0, "taken", "")).contains("already a file named \"taken.md\""));
    // whatever its letter case
    add_agent(&w.empty.1, "Upper.md", Some(&agent_text("other-name", "")));
    assert!(err(w.service.create(&w.inputs, &w.empty.0, "upper", "")).contains("already a file named \"Upper.md\""));
    assert_eq!(tree(&w.empty.1), ["Upper.md", "log-reader.md", "taken.md"]);
}

#[test]
fn duplicates_an_agent_next_to_the_original_under_a_name_of_its_own() {
    let w = world();
    add_agent(&w.personal.1, "team/reviewer.md", Some(&agent_text("reviewer", "tools: Read, Grep\n")));
    let copy = w.service.duplicate(&w.inputs, &at(&w.personal.0, "team/reviewer.md"), "reviewer-2").unwrap();
    assert_eq!(copy, at(&w.personal.0, "team/reviewer-2.md"));
    assert_eq!(
        read(&w.personal.1.join("team").join("reviewer-2.md")),
        agent_text("reviewer", "tools: Read, Grep\n").replace("name: reviewer", "name: reviewer-2")
    );
    assert!(read(&w.personal.1.join("team").join("reviewer.md")).contains("name: reviewer\n"));
    assert!(err(w.service.duplicate(&w.inputs, &at(&w.personal.0, "team/reviewer.md"), "reviewer")).contains("already an agent named \"reviewer\""));
}

#[test]
fn renames_the_agent_and_its_file_when_the_file_was_named_after_it() {
    let w = world();
    add_agent(&w.personal.1, "reviewer.md", None);
    let moved = w.service.rename(&w.inputs, &at(&w.personal.0, "reviewer.md"), "code-reviewer").unwrap();
    assert_eq!(moved, AgentMoved { r#ref: at(&w.personal.0, "code-reviewer.md"), warning: None });
    assert_eq!(tree(&w.personal.1), ["code-reviewer.md"]);
    assert_eq!(read(&w.personal.1.join("code-reviewer.md")), agent_text("reviewer", "").replace("name: reviewer", "name: code-reviewer"));

    // a file with a name of its own keeps it
    add_agent(&w.personal.1, "team/qa.md", Some(&agent_text("tester", "")));
    let qa = at(&w.personal.0, "team/qa.md");
    assert_eq!(w.service.rename(&w.inputs, &qa, "test-runner").unwrap(), AgentMoved { r#ref: qa.clone(), warning: None });
    assert!(read(&w.personal.1.join("team").join("qa.md")).contains("name: test-runner\n"));

    // the same name again changes nothing; a name that is taken is refused before anything is written
    assert_eq!(w.service.rename(&w.inputs, &qa, "test-runner").unwrap(), AgentMoved { r#ref: qa.clone(), warning: None });
    assert!(err(w.service.rename(&w.inputs, &qa, "code-reviewer")).contains("already an agent named \"code-reviewer\""));
    assert!(read(&w.personal.1.join("team").join("qa.md")).contains("name: test-runner\n"));
}

#[test]
fn does_not_rename_onto_a_file_that_is_there_and_gives_a_file_without_a_name_one() {
    let w = world();
    add_agent(&w.personal.1, "reviewer.md", None);
    add_agent(&w.personal.1, "notes.md", Some("# notes about agents\n"));
    assert!(err(w.service.rename(&w.inputs, &at(&w.personal.0, "reviewer.md"), "notes")).contains("already a file named \"notes.md\""));
    assert_eq!(read(&w.personal.1.join("reviewer.md")), agent_text("reviewer", ""));

    let notes = at(&w.personal.0, "notes.md");
    assert_eq!(w.service.rename(&w.inputs, &notes, "note-taker").unwrap(), AgentMoved { r#ref: notes, warning: None });
    assert_eq!(read(&w.personal.1.join("notes.md")), "---\nname: note-taker\n---\n\n# notes about agents\n");

    add_agent(&w.personal.1, "broken.md", Some("---\nname: broken\ndescription: Use: now\n---\nx\n"));
    assert!(err(w.service.rename(&w.inputs, &at(&w.personal.0, "broken.md"), "fixed")).contains("Fix it in the Source tab first"));
}

#[test]
fn puts_a_deleted_agent_into_the_bin_and_never_deletes_it_for_good() {
    let w = world();
    let abs = add_agent(&w.personal.1, "reviewer.md", None);
    w.service.remove(&w.inputs, &at(&w.personal.0, "reviewer.md")).unwrap();
    assert_eq!(*w.trashed.lock().unwrap(), [abs]);

    let kept = add_agent(&w.personal.1, "keep.md", None);
    w.fail_trash.store(true, Ordering::SeqCst);
    assert_eq!(
        err(w.service.remove(&w.inputs, &at(&w.personal.0, "keep.md"))),
        "The agent could not be moved to the bin (no bin on this drive). It was not deleted."
    );
    assert!(kept.exists());
}

#[test]
fn copies_an_agent_to_a_project_and_moves_one_back() {
    let w = world();
    let abs = add_agent(&w.personal.1, "team/reviewer.md", Some(&agent_text("reviewer", "")));
    let copied = w.service.transfer(&w.inputs, &at(&w.personal.0, "team/reviewer.md"), &w.empty.0, TransferMode::Copy, "").unwrap();
    assert_eq!(copied, AgentMoved { r#ref: at(&w.empty.0, "reviewer.md"), warning: None });
    assert_eq!(read(&w.empty.1.join("reviewer.md")), agent_text("reviewer", ""));
    assert!(abs.exists());

    // the same name is in the way; another one is not
    let back = at(&w.empty.0, "reviewer.md");
    assert!(err(w.service.transfer(&w.inputs, &back, &w.personal.0, TransferMode::Move, "")).contains("already an agent named \"reviewer\""));
    let moved = w.service.transfer(&w.inputs, &back, &w.personal.0, TransferMode::Move, "reviewer-from-project").unwrap();
    assert_eq!(moved, AgentMoved { r#ref: at(&w.personal.0, "reviewer-from-project.md"), warning: None });
    assert_eq!(*w.trashed.lock().unwrap(), [w.empty.1.join("reviewer.md")]);
    assert!(read(&w.personal.1.join("reviewer-from-project.md")).contains("name: reviewer-from-project\n"));
    assert_eq!(tree(&w.personal.1), ["reviewer-from-project.md", "team/reviewer.md"]);

    assert!(err(w.service.transfer(&w.inputs, &at(&w.personal.0, "team/reviewer.md"), &w.personal.0, TransferMode::Copy, ""))
        .contains("in that folder already"));
}

#[test]
fn copies_the_bytes_as_they_are_when_the_name_stays() {
    let w = world();
    let original = format!("\u{feff}{}", agent_text("reviewer", "").replace('\n', "\r\n"));
    add_agent(&w.personal.1, "reviewer.md", Some(&original));
    let renamed_copy = w.service.transfer(&w.inputs, &at(&w.personal.0, "reviewer.md"), &w.project.0, TransferMode::Copy, "reviewer").unwrap();
    assert_eq!(renamed_copy.r#ref, at(&w.project.0, "reviewer.md"));
    assert_eq!(fs::read(w.project.1.join("reviewer.md")).unwrap(), original.as_bytes());
    // under another name it keeps how it was stored
    w.service.transfer(&w.inputs, &at(&w.personal.0, "reviewer.md"), &w.empty.0, TransferMode::Copy, "checker").unwrap();
    assert_eq!(read(&w.empty.1.join("checker.md")), original.replace("name: reviewer", "name: checker"));
}

#[test]
fn says_so_when_a_move_could_only_copy() {
    let w = world();
    let abs = add_agent(&w.personal.1, "reviewer.md", None);
    w.fail_trash.store(true, Ordering::SeqCst);
    let result = w.service.transfer(&w.inputs, &at(&w.personal.0, "reviewer.md"), &w.project.0, TransferMode::Move, "").unwrap();
    assert_eq!(result.r#ref, at(&w.project.0, "reviewer.md"));
    assert!(result.warning.unwrap().contains("the original is still there"));
    assert!(abs.exists());
}

#[test]
fn removes_only_the_link_of_an_agent_that_is_one() {
    let w = world();
    let elsewhere = add_agent(&w.base.join("shared"), "linked.md", None);
    let at_link = w.personal.1.join("linked.md");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&elsewhere, &at_link).is_ok();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&elsewhere, &at_link).is_ok();
    if !made {
        // Windows without the right to make links
        return;
    }
    w.service.remove(&w.inputs, &at(&w.personal.0, "linked.md")).unwrap();
    assert!(!at_link.exists() && elsewhere.exists());
    assert!(w.trashed.lock().unwrap().is_empty());
}
