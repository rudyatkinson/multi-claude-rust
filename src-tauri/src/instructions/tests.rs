//! `test/instructions.test.ts`: `findScopes`, `listFiles`, `loadOrder` and `InstructionService`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mc_core::types::{InstructionKind, InstructionRef, InstructionScopeKind, InstructionTarget, LoadWhen, SaveOutcome};

use super::scan::real_of;
use super::*;

/// A temporary folder, by its resolved path (as `fs.realpathSync(mkdtemp)` gives it).
fn temp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let real = real_of(dir.path());
    (dir, real)
}

fn write(file: &Path, content: &str) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn lines(l: &[&str]) -> String {
    format!("{}\n", l.join("\n"))
}

fn numbered(count: usize, line: impl Fn(usize) -> String) -> String {
    (0..count).map(|i| format!("{}\n", line(i))).collect()
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn account(id: &str, config_dir: &Path, synced: bool) -> InstructionAccount {
    InstructionAccount { id: id.to_owned(), config_dir: config_dir.to_path_buf(), synced_claude_md: synced }
}

fn message<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    result.unwrap_err().to_string()
}

// ---------- the scopes and their files ----------

#[test]
fn lists_the_accounts_then_the_folders_in_sight_with_the_top_of_their_repository() {
    let (_guard, tmp) = temp();
    let main = tmp.join("home").join(".claude");
    let other = tmp.join("profiles").join("elif");
    let repo = tmp.join("repo");
    for dir in [&main, &other, &repo.join(".git"), &repo.join("app"), &tmp.join("plain")] {
        fs::create_dir_all(dir).unwrap();
    }
    let folders = [repo.join("app"), tmp.join("plain"), repo.join("app"), main.clone(), tmp.join("gone")].map(|p| text(&p));
    let mut folders = folders.to_vec();
    folders.push("relative".into());
    let scopes = find_scopes(&[account("main", &main, false), account("elif", &other, true)], &folders);
    let shown: Vec<(InstructionScopeKind, String, String, bool)> = scopes
        .iter()
        .map(|s| (s.scope.kind, s.scope.profile_id.clone().unwrap_or_default(), s.scope.dir.clone(), s.scope.synced_claude_md.unwrap_or(false)))
        .collect();
    assert_eq!(
        shown,
        [
            (InstructionScopeKind::Account, "main".into(), text(&main), false),
            (InstructionScopeKind::Account, "elif".into(), text(&other), true),
            (InstructionScopeKind::Project, String::new(), text(&repo), false),
            (InstructionScopeKind::Project, String::new(), text(&repo.join("app")), false),
            (InstructionScopeKind::Project, String::new(), text(&tmp.join("plain")), false),
        ]
    );
    assert_eq!(scopes.iter().map(|s| &s.scope.id[..2]).collect::<Vec<_>>(), ["a-", "a-", "f-", "f-", "f-"]);
    let ids: std::collections::HashSet<&str> = scopes.iter().map(|s| s.scope.id.as_str()).collect();
    assert_eq!(ids.len(), 5);
    assert!(scopes.iter().all(|s| mc_core::validate::instruction_scope(&s.scope.id).is_ok()));
}

#[test]
fn a_worktree_counts_under_the_repository_it_was_added_to() {
    let (_guard, tmp) = temp();
    let repo = tmp.join("repo");
    let linked = repo.join(".git").join("worktrees").join("wt");
    write(&linked.join("commondir"), "../..\n");
    let worktree = tmp.join("wt");
    write(&worktree.join(".git"), &format!("gitdir: {}\n", text(&linked)));
    assert_eq!(scan::git_root_of(&worktree.join("src")), Some(repo.clone()));
    // a submodule's .git file points at no common folder: the folder itself is the top
    let sub = tmp.join("sub");
    write(&sub.join(".git"), "gitdir: ../repo/.git/modules/sub\n");
    assert_eq!(scan::git_root_of(&sub), Some(sub.clone()));
}

#[test]
fn lists_the_fixed_files_whether_they_are_there_or_not_then_the_rules() {
    let (_guard, tmp) = temp();
    let project = tmp.join("proj");
    write(&project.join("CLAUDE.md"), &lines(&["# Proj", "See @docs/a.md."]));
    write(&project.join(".claude/rules/tests.md"), &lines(&["---", "paths:", "  - \"**/*.test.ts\"", "---", "", "Test rules."]));
    write(&project.join(".claude/rules/frontend/style.md"), "Style.\n");
    write(&project.join(".claude/rules/notes.txt"), "not a rule");
    let file = |file: &str, kind, exists, lines, paths: &[&str], imports: &[&str]| InstructionFile {
        file: file.into(),
        kind,
        exists,
        lines,
        paths: paths.iter().map(|s| (*s).to_owned()).collect(),
        imports: imports.iter().map(|s| (*s).to_owned()).collect(),
    };
    assert_eq!(
        list_files(InstructionScopeKind::Project, &project),
        [
            file("CLAUDE.md", InstructionKind::Claude, true, 2, &[], &["docs/a.md"]),
            file(".claude/CLAUDE.md", InstructionKind::Claude, false, 0, &[], &[]),
            file("CLAUDE.local.md", InstructionKind::Local, false, 0, &[], &[]),
            file(".claude/rules/tests.md", InstructionKind::Rule, true, 6, &["**/*.test.ts"], &[]),
            file(".claude/rules/frontend/style.md", InstructionKind::Rule, true, 1, &[], &[]),
        ]
    );

    let config = tmp.join("config");
    write(&config.join("rules/general.md"), "General.\n");
    let shown: Vec<String> = list_files(InstructionScopeKind::Account, &config).iter().map(|f| format!("{}:{}", f.file, f.exists)).collect();
    assert_eq!(shown, ["CLAUDE.md:false", "rules/general.md:true"]);
}

#[test]
fn rules_skip_dependencies_and_stop_at_six_folders_down() {
    let (_guard, tmp) = temp();
    let rules = tmp.join("rules");
    write(&rules.join("node_modules/x.md"), "x\n");
    write(&rules.join(".git/y.md"), "y\n");
    write(&rules.join("1/2/3/4/5/6/deep.md"), "deep\n");
    write(&rules.join("1/2/3/4/5/6/7/deeper.md"), "deeper\n");
    write(&rules.join("b.md"), "b\n");
    write(&rules.join("A.md"), "a\n");
    assert_eq!(find_rules(&rules), ["A.md", "b.md", "1/2/3/4/5/6/deep.md"]);
    assert!(find_rules(&tmp.join("none")).is_empty());
}

// ---------- what a session reads ----------

fn inputs(config_dir: &Path, cwd: &Path, home: &Path, top: &Path) -> LoadInputs {
    LoadInputs {
        config_dir: config_dir.to_path_buf(),
        cwd: cwd.to_path_buf(),
        home_dir: home.to_path_buf(),
        managed_file: None,
        memory_dir: None,
        top: Some(top.to_path_buf()),
    }
}

#[test]
fn goes_from_the_account_down_to_the_folder_with_imports_rules_on_demand_and_memory_last() {
    let (_guard, tmp) = temp();
    let home = tmp.join("home");
    let config = tmp.join("profiles").join("elif");
    let repo = home.join("work").join("repo");
    let cwd = repo.join("app");
    write(&config.join("CLAUDE.md"), &lines(&["# Mine", "Read @~/notes/style.md first."]));
    write(&home.join("notes/style.md"), &lines(&["Style.", "And @more.md, and @missing.md."]));
    write(&home.join("notes/more.md"), "More.\n");
    write(&config.join("rules/always.md"), "Always.\n");
    write(&config.join("rules/ts.md"), &lines(&["---", "paths: \"**/*.ts\"", "---", "Only for TypeScript."]));
    // the main account's own file lies on the way down: it is read as a file of that folder
    write(&home.join(".claude/CLAUDE.md"), "Main account.\n");
    write(&repo.join("CLAUDE.md"), &numbered(250, |i| format!("line {i}")));
    write(&repo.join(".claude/rules/git.md"), "Git.\n");
    write(&repo.join("CLAUDE.local.md"), "Local.\n");
    write(&cwd.join(".claude/CLAUDE.md"), &lines(&["App.", "Again @../../CLAUDE.md, which is read already."]));
    let memory = config.join("projects").join("x").join("memory");
    write(&memory.join("MEMORY.md"), &numbered(230, |i| format!("- [M{i}](m{i}.md)")));

    let mut asked = inputs(&config, &cwd, &home, &tmp);
    asked.managed_file = Some(tmp.join("no-such").join("CLAUDE.md"));
    asked.memory_dir = Some(memory);
    let steps = load_order(&asked);
    let rel = |p: &str| Path::new(p).strip_prefix(&tmp).unwrap().to_string_lossy().replace('\\', "/");
    let shown: Vec<(String, &str, LoadWhen, u32, u32)> = steps.iter().map(|s| (rel(&s.path), s.role.as_str(), s.when, s.lines, s.depth)).collect();
    let step = |p: &str, role, when, lines, depth| (p.to_owned(), role, when, lines, depth);
    assert_eq!(
        shown,
        [
            step("profiles/elif/CLAUDE.md", "Account", LoadWhen::Start, 2, 0),
            step("home/notes/style.md", "Import", LoadWhen::Start, 2, 1),
            step("home/notes/more.md", "Import", LoadWhen::Start, 1, 2),
            step("home/notes/missing.md", "Import", LoadWhen::Missing, 0, 2),
            step("profiles/elif/rules/always.md", "Rule", LoadWhen::Start, 1, 0),
            step("profiles/elif/rules/ts.md", "Rule", LoadWhen::Demand, 4, 0),
            step("home/.claude/CLAUDE.md", "Project", LoadWhen::Start, 1, 0),
            step("home/work/repo/CLAUDE.md", "Project", LoadWhen::Start, 250, 0),
            step("home/work/repo/.claude/rules/git.md", "Rule", LoadWhen::Start, 1, 0),
            step("home/work/repo/CLAUDE.local.md", "Local", LoadWhen::Start, 1, 0),
            step("home/work/repo/app/.claude/CLAUDE.md", "Project", LoadWhen::Start, 2, 0),
            step("profiles/elif/projects/x/memory/MEMORY.md", "Memory", LoadWhen::Start, 200, 0),
        ]
    );
    let note_of = |end: &str| steps.iter().find(|s| s.path.replace('\\', "/").ends_with(end)).and_then(|s| s.note.clone());
    assert!(note_of("repo/CLAUDE.md").unwrap().contains("Longer than the 200 lines"));
    assert_eq!(note_of("rules/ts.md").as_deref(), Some("Read when Claude works on **/*.ts"));
    assert_eq!(note_of("notes/missing.md").as_deref(), Some("\"@missing.md\" leads to no file."));
    assert_eq!(note_of("MEMORY.md").as_deref(), Some("Only the first 200 of its 230 lines are read."));
    assert_eq!(note_of("profiles/elif/CLAUDE.md"), None);
}

#[test]
fn follows_imports_four_hops_and_no_further_and_never_in_a_circle() {
    let (_guard, tmp) = temp();
    let config = tmp.join("config");
    write(&config.join("CLAUDE.md"), "@a.md\n");
    for (name, next) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "e"), ("e", "a")] {
        write(&config.join(format!("{name}.md")), &format!("@{next}.md\n"));
    }
    let steps = load_order(&inputs(&config, &tmp.join("empty"), &tmp, &tmp));
    let shown: Vec<String> = steps.iter().map(|s| format!("{}:{}", Path::new(&s.path).file_name().unwrap().to_string_lossy(), s.depth)).collect();
    assert_eq!(shown, ["CLAUDE.md:0", "a.md:1", "b.md:2", "c.md:3", "d.md:4"]);

    // a circle short of the limit stops where it began
    write(&config.join("CLAUDE.md"), "@x.md\n");
    write(&config.join("x.md"), "@CLAUDE.md\n");
    let steps = load_order(&inputs(&config, &tmp.join("empty"), &tmp, &tmp));
    assert_eq!(steps.iter().map(|s| s.depth).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn the_managed_file_comes_first_and_rules_imports_inherit_on_demand() {
    let (_guard, tmp) = temp();
    let config = tmp.join("config");
    let managed = tmp.join("policy").join("CLAUDE.md");
    write(&managed, &numbered(201, |i| format!("rule {i}")));
    write(&config.join("rules/web.md"), &lines(&["---", "paths: web/**", "---", "See @../web-notes.md."]));
    write(&config.join("web-notes.md"), "Notes.\n");
    let mut asked = inputs(&config, &tmp.join("empty"), &tmp, &tmp);
    asked.managed_file = Some(managed);
    let steps = load_order(&asked);
    let shown: Vec<(&str, LoadWhen, u32)> = steps.iter().map(|s| (s.role.as_str(), s.when, s.depth)).collect();
    // what a rule read on demand imports is read on demand too
    assert_eq!(shown, [("Managed policy", LoadWhen::Start, 0), ("Rule", LoadWhen::Demand, 0), ("Import", LoadWhen::Demand, 1)]);
    assert!(steps[0].note.as_deref().unwrap().starts_with("Longer than"));
}

// ---------- InstructionService ----------

struct TestDeps {
    main: PathBuf,
    other: PathBuf,
    project: PathBuf,
    home: PathBuf,
    top: PathBuf,
    trashed: Mutex<Vec<PathBuf>>,
    refuse: Mutex<Option<io::ErrorKind>>,
    changed: AtomicUsize,
    asked_memory: AtomicBool,
}

impl InstructionDeps for TestDeps {
    fn accounts(&self) -> Vec<InstructionAccount> {
        vec![account("main", &self.main, false), account("elif", &self.other, true)]
    }
    fn folders(&self) -> Vec<String> {
        vec![text(&self.project)]
    }
    fn home_dir(&self) -> PathBuf {
        self.home.clone()
    }
    fn managed_file(&self) -> Option<PathBuf> {
        None
    }
    fn top(&self) -> Option<PathBuf> {
        Some(self.top.clone())
    }
    fn memory_dir(&self, profile_id: &str, _cwd: &Path) -> Option<PathBuf> {
        self.asked_memory.store(true, Ordering::SeqCst);
        (profile_id == "main").then(|| self.main.join("projects").join("p").join("memory"))
    }
    fn trash(&self, file: &Path) -> io::Result<()> {
        if let Some(kind) = *self.refuse.lock().unwrap() {
            return Err(if kind == io::ErrorKind::Other { io::Error::other("no bin here") } else { io::Error::from(kind) });
        }
        self.trashed.lock().unwrap().push(file.to_path_buf());
        fs::remove_file(file)
    }
    fn account_changed(&self) {
        self.changed.fetch_add(1, Ordering::SeqCst);
    }
}

struct World {
    _guard: tempfile::TempDir,
    deps: Arc<TestDeps>,
    service: InstructionService,
}

#[derive(Clone, Copy)]
enum Who {
    Main,
    Other,
    Project,
}

impl World {
    fn new() -> Self {
        let (guard, tmp) = temp();
        let main = tmp.join("home").join(".claude");
        let other = tmp.join("profiles").join("elif");
        let project = tmp.join("proj");
        for dir in [&main, &other, &project] {
            fs::create_dir_all(dir).unwrap();
        }
        let deps = Arc::new(TestDeps {
            main,
            other,
            project,
            home: tmp.join("home"),
            top: tmp.clone(),
            trashed: Mutex::default(),
            refuse: Mutex::default(),
            changed: AtomicUsize::new(0),
            asked_memory: AtomicBool::new(false),
        });
        let service = InstructionService::with_deps(deps.clone());
        World { _guard: guard, deps, service }
    }

    fn scope(&self, who: Who) -> String {
        let scopes = self.service.scopes();
        let found = match who {
            Who::Project => scopes.iter().find(|s| s.kind == InstructionScopeKind::Project),
            Who::Main => scopes.iter().find(|s| s.profile_id.as_deref() == Some("main")),
            Who::Other => scopes.iter().find(|s| s.profile_id.as_deref() == Some("elif")),
        };
        found.unwrap().id.clone()
    }

    fn at(&self, who: Who, file: &str) -> InstructionRef {
        InstructionRef { scope_id: self.scope(who), file: file.to_owned() }
    }

    fn changed(&self) -> usize {
        self.deps.changed.load(Ordering::SeqCst)
    }
}

#[test]
fn hands_out_scopes_without_their_resolved_paths() {
    let w = World::new();
    let scopes = w.service.scopes();
    let shown: Vec<(InstructionScopeKind, Option<&str>, Option<bool>)> = scopes.iter().map(|s| (s.kind, s.profile_id.as_deref(), s.synced_claude_md)).collect();
    assert_eq!(
        shown,
        [
            (InstructionScopeKind::Account, Some("main"), None),
            (InstructionScopeKind::Account, Some("elif"), Some(true)),
            (InstructionScopeKind::Project, None, None),
        ]
    );
    let wire = serde_json::to_value(&scopes[0]).unwrap();
    assert!(wire.get("real").is_none());
    assert!(wire.get("syncedClaudeMd").is_none());
}

#[test]
fn makes_a_file_that_is_not_there_yet_and_a_rule() {
    let w = World::new();
    let project = w.scope(Who::Project);
    let file = |name: &str| InstructionTarget::File(name.into());
    let rule = |name: &str| InstructionTarget::Rule(name.into());
    assert_eq!(w.service.create(&project, &file("CLAUDE.md")).unwrap(), w.at(Who::Project, "CLAUDE.md"));
    let made = fs::read_to_string(w.deps.project.join("CLAUDE.md")).unwrap();
    assert!(made.starts_with("# proj\n\nWhat Claude should know about this project"), "{made}");
    assert!(message(w.service.create(&project, &file("CLAUDE.md"))).contains("already a file named \"CLAUDE.md\""));
    assert!(message(w.service.create(&project, &file("../outside.md"))).contains("cannot be made here"));
    assert!(message(w.service.create(&w.scope(Who::Main), &file("CLAUDE.local.md"))).contains("cannot be made here"));

    assert_eq!(w.service.create(&project, &rule("code-style")).unwrap(), w.at(Who::Project, ".claude/rules/code-style.md"));
    assert!(fs::read_to_string(w.deps.project.join(".claude/rules/code-style.md")).unwrap().starts_with("# Code Style\n"));
    assert!(message(w.service.create(&project, &rule("../x"))).contains("lower-case"));
    assert!(!w.deps.project.parent().unwrap().join("x.md").exists());
    assert_eq!(w.changed(), 0);

    // a file of an account: what is shown about syncing may change
    w.service.create(&w.scope(Who::Main), &file("CLAUDE.md")).unwrap();
    assert!(fs::read_to_string(w.deps.main.join("CLAUDE.md")).unwrap().starts_with("# My instructions\n"));
    w.service.create(&w.scope(Who::Other), &rule("tone")).unwrap();
    assert!(w.deps.other.join("rules").join("tone.md").exists());
    assert_eq!(w.changed(), 2);
}

#[test]
fn reads_and_saves_only_files_the_scope_lists() {
    let w = World::new();
    write(&w.deps.project.join("CLAUDE.md"), "One.\r\n");
    write(&w.deps.project.join("secret.md"), "secret");
    write(&w.deps.project.join(".claude/settings.json"), "{}");
    let r = w.at(Who::Project, "CLAUDE.md");
    let read = w.service.read(&r).unwrap();
    assert_eq!((read.text.as_str(), read.path.as_str()), ("One.\n", text(&w.deps.project.join("CLAUDE.md")).as_str()));
    assert!(matches!(w.service.save(&r, "Two.\n", Some(&read.hash)).unwrap(), SaveOutcome::Saved(_)));
    assert_eq!(fs::read_to_string(w.deps.project.join("CLAUDE.md")).unwrap(), "Two.\r\n");
    assert!(matches!(w.service.save(&r, "Three.\n", Some(&read.hash)).unwrap(), SaveOutcome::Conflict(_)));
    assert_eq!(w.changed(), 0);

    for file in ["secret.md", ".claude/settings.json", "../proj/CLAUDE.md", "CLAUDE.local.md", ".claude/rules/../../secret.md"] {
        let other = InstructionRef { scope_id: r.scope_id.clone(), file: file.to_owned() };
        assert!(message(w.service.read(&other)).contains("no longer there"), "{file}");
        assert!(message(w.service.save(&other, "x", None)).contains("no longer there"), "{file}");
        assert!(message(w.service.delete(&other)).contains("no longer there"), "{file}");
    }
    let unknown = InstructionRef { scope_id: "f-000000000000".into(), file: "CLAUDE.md".into() };
    assert!(message(w.service.read(&unknown)).contains("no longer in the list"));
    assert_eq!(fs::read_to_string(w.deps.project.join("secret.md")).unwrap(), "secret");

    // an account's file tells the accounts' views
    write(&w.deps.main.join("CLAUDE.md"), "Mine.\n");
    w.service.save(&w.at(Who::Main, "CLAUDE.md"), "Mine too.\n", None).unwrap();
    assert_eq!(w.changed(), 1);
}

#[test]
fn puts_a_deleted_file_into_the_bin() {
    let w = World::new();
    let old = w.deps.project.join(".claude").join("rules").join("old.md");
    write(&old, "Old.\n");
    w.service.delete(&w.at(Who::Project, ".claude/rules/old.md")).unwrap();
    assert_eq!(*w.deps.trashed.lock().unwrap(), [old]);
    assert_eq!(w.changed(), 0);
}

#[test]
fn a_file_that_cannot_go_to_the_bin_stays() {
    let w = World::new();
    let file = w.deps.project.join("CLAUDE.md");
    write(&file, "Keep.\n");
    let r = w.at(Who::Project, "CLAUDE.md");
    *w.deps.refuse.lock().unwrap() = Some(io::ErrorKind::Other);
    assert_eq!(message(w.service.delete(&r)), "The file could not be moved to the bin (no bin here). It was not deleted.");
    *w.deps.refuse.lock().unwrap() = Some(io::ErrorKind::PermissionDenied);
    assert!(message(w.service.delete(&r)).starts_with("Deleting did not work: Windows says the folder is in use."));
    assert!(file.exists());
}

#[test]
#[cfg(unix)]
fn a_linked_file_is_only_unlinked() {
    let w = World::new();
    let real = w.deps.project.join("shared.md");
    write(&real, "Shared.\n");
    std::os::unix::fs::symlink(&real, w.deps.project.join("CLAUDE.md")).unwrap();
    w.service.delete(&w.at(Who::Project, "CLAUDE.md")).unwrap();
    assert!(real.exists() && !w.deps.project.join("CLAUDE.md").exists());
    assert!(w.deps.trashed.lock().unwrap().is_empty());
}

#[test]
fn shows_what_a_session_of_an_account_reads_in_a_project_folder() {
    let w = World::new();
    write(&w.deps.main.join("CLAUDE.md"), "Main.\n");
    write(&w.deps.project.join("CLAUDE.md"), "Proj.\n");
    write(&w.deps.main.join("projects/p/memory/MEMORY.md"), "- [A](a.md)\n");
    let project = w.scope(Who::Project);
    let roles = |profile: &str| w.service.view(profile, &project).unwrap().into_iter().map(|s| s.role).collect::<Vec<_>>();
    assert_eq!(roles("main"), ["Account", "Project", "Memory"]);
    assert_eq!(roles("elif"), ["Project"]);
    assert!(message(w.service.view("main", &w.scope(Who::Main))).contains("Choose a project folder"));
    assert!(message(w.service.view("nobody", &project)).contains("Profile not found"));

    // the integration's memory folder takes the place of the deps' answer
    w.deps.asked_memory.store(false, Ordering::SeqCst);
    let shared = w.deps.main.join("projects/p/memory");
    w.service.set_memory_dir(move |_, _| Some(shared.clone()));
    assert_eq!(roles("elif"), ["Project", "Memory"]);
    assert!(!w.deps.asked_memory.load(Ordering::SeqCst));
}

#[test]
fn paths_for_the_file_manager_are_listed_files_or_the_folder() {
    let w = World::new();
    write(&w.deps.project.join("CLAUDE.md"), "Proj.\n");
    let project = w.scope(Who::Project);
    assert_eq!(w.service.path_of(&project, None).unwrap(), w.deps.project);
    assert_eq!(w.service.path_of(&project, Some("CLAUDE.md")).unwrap(), w.deps.project.join("CLAUDE.md"));
    assert!(message(w.service.path_of(&project, Some("CLAUDE.local.md"))).contains("no longer there"));
    let files: Vec<String> = w.service.files(&project).unwrap().into_iter().map(|f| f.file).collect();
    assert_eq!(files, ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"]);
    assert!(message(w.service.reveal(&project, None)).contains("Not available"));
}

// ---------- the app's own deps ----------

#[test]
fn an_accounts_own_memory_lies_under_its_projects_folder() {
    let (_guard, tmp) = temp();
    let config = tmp.join("config");
    let repo = tmp.join("Repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join("app")).unwrap();
    let key = sanitize_path(&text(&repo));
    assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    // memory goes by the top of the repository
    assert_eq!(own_memory_dir(&config, &repo.join("app")), Some(config.join("projects").join(&key).join("memory")));
    // spelled as on disk when it is there
    fs::create_dir_all(config.join("projects").join(key.to_uppercase())).unwrap();
    assert_eq!(own_memory_dir(&config, &repo), Some(config.join("projects").join(key.to_uppercase()).join("memory")));
    assert_eq!(own_memory_dir(&config, Path::new("relative")), None);
    assert_eq!(own_memory_dir(&config, &tmp.join("x".repeat(220))), None);
    assert_eq!(sanitize_path("C:\\a b/ü😀"), "C--a-b----");
}

#[test]
fn the_managed_file_has_its_place_per_system() {
    let managed = managed_claude_md();
    assert!(managed.ends_with(Path::new("ClaudeCode/CLAUDE.md")) || managed.ends_with(Path::new("claude-code/CLAUDE.md")));
}
