//! `test/token.test.ts` (ProfileService parts) and the rest of the service, against a temporary
//! home: creating accounts in the real `~/.multi-claude` or touching the real `~/.claude` would be a bug.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use mc_core::types::{SyncStatus, PROFILE_COLORS};
use serde_json::{json, Value};

use super::*;
use crate::token_vault::MemoryTokenStore;

const TOKEN: &str = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz";

struct Fixture {
    tmp: tempfile::TempDir,
    tokens: Arc<MemoryTokenStore>,
    service: ProfileService,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("mc");
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());
        let config = Arc::new(ConfigStore::new(root.clone()));
        let service = ProfileService::with_parts(None, Paths::with_home(root, home), config, tokens.clone());
        // never the real recycle bin
        service.set_trash(|dir| fs::remove_dir_all(dir).map_err(|e| e.to_string()));
        Fixture { tmp, tokens, service }
    }

    fn main_dir(&self) -> PathBuf {
        self.service.paths().main_claude_dir()
    }

    fn stored_text(&self) -> String {
        fs::read_to_string(self.service.config().file()).unwrap()
    }

    fn stored(&self) -> Vec<Profile> {
        self.service.list()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // links first, so the recursive delete of the temp dir can never follow one
        if let Ok(entries) = fs::read_dir(self.service.paths().profiles_dir()) {
            for entry in entries.flatten() {
                let _ = sharing::remove_all_links(&entry.path());
            }
        }
    }
}

fn input(name: &str) -> CreateProfileInput {
    CreateProfileInput {
        name: name.to_owned(),
        color: "#5b9bd5".to_owned(),
        auth: None,
        token: None,
        email: None,
        share_dirs: vec![],
        share_files: vec![],
        keep_model_separate: true,
    }
}

fn token_input(name: &str, token: Option<&str>) -> CreateProfileInput {
    CreateProfileInput { auth: Some(AuthKind::Token), token: token.map(str::to_owned), ..input(name) }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

// ---------- the main account ----------

#[test]
fn makes_the_main_account_once() {
    let f = Fixture::new();
    let list = f.stored();
    assert_eq!(list.len(), 1);
    let main = &list[0];
    assert_eq!((main.id.as_str(), main.name.as_str(), main.slug.as_str()), ("main", "Main", "main"));
    assert!(main.is_default);
    assert_eq!(main.config_dir, None);
    assert_eq!(main.color, PROFILE_COLORS[0]);
    assert!(f.stored_text().contains("\"isDefault\": true"));

    // a second service on the same file finds it
    let again = ProfileService::with_parts(None, f.service.paths().clone(), Arc::new(ConfigStore::new(f.service.paths().root().to_path_buf())), f.tokens.clone());
    assert_eq!(again.list().len(), 1);
}

#[test]
fn runs_against_the_test_home_not_the_real_one() {
    let f = Fixture::new();
    assert!(f.service.paths().profiles_dir().starts_with(f.tmp.path()));
    assert!(f.main_dir().starts_with(f.tmp.path()));
}

// ---------- token accounts (token.test.ts) ----------

#[test]
fn stores_the_token_in_the_vault_and_never_hands_it_to_the_renderer() {
    let f = Fixture::new();
    let (profile, warnings) = f.service.create(&token_input("CI", Some(&format!("  {TOKEN}  ")))).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(profile.auth, Some(AuthKind::Token));
    assert_eq!(profile.token_enc.as_deref(), Some(TOKEN_MARKER));
    assert_eq!(f.tokens.load(&profile.id).as_deref(), Some(TOKEN));
    assert_eq!(f.service.token_for(&profile.id).as_deref(), Some(TOKEN));
    let account = profile.account.as_ref().unwrap();
    assert!(account.logged_in);
    assert_eq!(account.auth_method.as_deref(), Some("oauth_token"));
    assert!(Path::new(profile.config_dir.as_deref().unwrap()).is_dir());

    let view = f.service.view(&profile.id).unwrap();
    assert_eq!(view.auth, Some(AuthKind::Token));
    let view_json = serde_json::to_value(&view).unwrap();
    assert!(!view_json.to_string().contains(TOKEN));
    assert!(view_json.get("tokenEnc").is_none());
    assert!(!f.stored_text().contains(TOKEN));
}

#[test]
fn rejects_a_missing_or_malformed_token() {
    let f = Fixture::new();
    let missing = f.service.create(&token_input("A", None)).unwrap_err().to_string();
    assert!(missing.contains("OAuth token"), "{missing}");
    let bad = f.service.create(&token_input("B", Some("hunter2"))).unwrap_err().to_string();
    assert!(bad.contains("OAuth token"), "{bad}");
    assert!(!f.stored().iter().any(|p| p.name == "A" || p.name == "B"));
    assert!(f.tokens.is_empty());
    assert!(!f.service.paths().profiles_dir().join("a").exists());
}

#[test]
fn replaces_the_token_of_a_token_account_only() {
    let f = Fixture::new();
    let (ci, _) = f.service.create(&token_input("CI", Some(TOKEN))).unwrap();
    f.service.set_token(&ci.id, "sk-ant-oat01-0123456789abcdefghij").unwrap();
    assert_eq!(f.service.token_for(&ci.id).as_deref(), Some("sk-ant-oat01-0123456789abcdefghij"));
    assert!(f.service.set_token(&ci.id, "nope").unwrap_err().to_string().contains("OAuth token"));

    let (login, _) = f.service.create(&input("Browser")).unwrap();
    assert_eq!(login.auth, None);
    assert_eq!(login.token_enc, None);
    assert_eq!(f.service.set_token(&login.id, TOKEN).unwrap_err().to_string(), "This account logs in through the browser");
    assert_eq!(f.service.token_for(&login.id), None);
    assert!(f.service.set_token("gone", TOKEN).is_err());
}

#[test]
fn a_token_from_the_electron_version_has_to_be_entered_again() {
    let f = Fixture::new();
    let (ci, _) = f.service.create(&token_input("CI", Some(TOKEN))).unwrap();
    // what safeStorage left in config.json
    f.service.config().update(|c| c.profiles.iter_mut().find(|p| p.id == ci.id).unwrap().token_enc = Some("djEwAAAAbase64==".into())).unwrap();
    assert_eq!(f.service.token_for(&ci.id), None);
    assert!(f.service.token_missing(&ci.id));
    f.service.set_token(&ci.id, TOKEN).unwrap();
    assert!(!f.service.token_missing(&ci.id));
    assert_eq!(f.service.get(&ci.id).unwrap().token_enc.as_deref(), Some(TOKEN_MARKER));
}

#[test]
fn the_environment_carries_the_token_of_a_token_account_only() {
    let f = Fixture::new();
    let (ci, _) = f.service.create(&token_input("CI", Some(TOKEN))).unwrap();
    let (home, _) = f.service.create(&input("Home")).unwrap();
    let env = f.service.env_for(&ci);
    assert_eq!(env.get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str), Some(TOKEN));
    assert_eq!(env.get("CLAUDE_CONFIG_DIR"), ci.config_dir.as_ref());
    assert!(!f.service.env_for(&home).contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
    let main = f.service.get(MAIN_ID).unwrap();
    assert!(!f.service.env_for(&main).contains_key("CLAUDE_CONFIG_DIR"));

    // a token from the Electron version is not passed on
    f.service.config().update(|c| c.profiles.iter_mut().find(|p| p.id == ci.id).unwrap().token_enc = Some("djEwAAAA".into())).unwrap();
    let legacy = f.service.get(&ci.id).unwrap();
    assert!(!f.service.env_for(&legacy).contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
}

// ---------- creating ----------

#[test]
fn gives_each_account_its_own_slug() {
    let f = Fixture::new();
    let slug = |name: &str| f.service.create(&input(name)).unwrap().0.slug;
    assert_eq!(slug("İş Hesabı"), "is-hesabi");
    assert_eq!(slug("iş hesabı"), "is-hesabi-2");
    assert_eq!(slug("Main"), "main-2");
    // a folder left behind counts as taken
    fs::create_dir_all(f.service.paths().profiles_dir().join("left")).unwrap();
    assert_eq!(slug("Left"), "left-2");
}

#[test]
fn checks_name_color_and_email() {
    let f = Fixture::new();
    let err = |i: CreateProfileInput| f.service.create(&i).unwrap_err().to_string();
    assert_eq!(err(input("  ")), "Enter an account name (up to 40 characters)");
    assert_eq!(err(CreateProfileInput { color: "red".into(), ..input("X") }), "Invalid color");
    assert_eq!(err(CreateProfileInput { email: Some("--help@x".into()), ..input("X") }), "Invalid email address");
    assert_eq!(f.stored().len(), 1);

    let (p, _) = f.service.create(&CreateProfileInput { email: Some("me@example.com".into()), ..input("  Work  ") }).unwrap();
    assert_eq!(p.name, "Work");
    assert_eq!(p.email_hint.as_deref(), Some("me@example.com"));
    assert!(!p.account.as_ref().unwrap().logged_in);
    // a token account takes no email hint
    let (t, _) = f.service.create(&CreateProfileInput { email: Some("bad".into()), ..token_input("CI", Some(TOKEN)) }).unwrap();
    assert_eq!(t.email_hint, None);
}

#[test]
fn links_and_copies_what_the_account_shares_before_claude_runs() {
    let f = Fixture::new();
    let main = f.main_dir();
    fs::create_dir_all(main.join("agents")).unwrap();
    fs::write(main.join("agents").join("reviewer.md"), "agent").unwrap();
    fs::write(main.join("settings.json"), r#"{"model":"opus","theme":"dark","apiKeyHelper":"x"}"#).unwrap();
    fs::write(main.join("CLAUDE.md"), "rules").unwrap();

    let (p, warnings) = f
        .service
        .create(&CreateProfileInput {
            share_dirs: vec!["agents".into(), "skills".into(), "evil".into()],
            share_files: vec!["settings.json".into(), "CLAUDE.md".into(), "../x".into()],
            ..input("Work")
        })
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(p.share.dirs, vec!["agents", "skills"]);
    assert_eq!(p.share.files, vec!["settings.json", "CLAUDE.md"]);
    assert_eq!(p.share.local_settings_keys, vec!["model", "effortLevel", "modelSettings"]);
    let dir = PathBuf::from(p.config_dir.as_deref().unwrap());
    assert!(sharing::is_link(&dir.join("agents")));
    assert_eq!(fs::read_to_string(dir.join("agents").join("reviewer.md")).unwrap(), "agent");
    let settings: Value = serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
    assert_eq!(settings, json!({ "model": "opus", "theme": "dark" }));
    assert_eq!(fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), "rules");
    assert_eq!(p.last_sync.as_ref().unwrap().main_hashes.len(), 2);

    // in sync, with a badge once Main changes
    assert_eq!(f.service.view(&p.id).unwrap().sync, Some(SyncStatus::default()));
    fs::write(main.join("CLAUDE.md"), "new rules").unwrap();
    fs::create_dir_all(main.join("skills")).unwrap();
    let badge = f.service.view(&p.id).unwrap().sync.unwrap();
    assert_eq!(badge.main_changed, vec!["CLAUDE.md", "skills"]);
    // Main has no badge
    assert_eq!(f.service.view(MAIN_ID).unwrap().sync, None);
}

// ---------- updating ----------

#[test]
fn updates_name_color_and_memory_sharing() {
    let f = Fixture::new();
    let (p, _) = f.service.create(&input("Work")).unwrap();
    f.service.update(&p.id, &ProfilePatch { name: Some(" Job ".into()), color: Some("#E06C75".into()), share_memory: Some(true) }).unwrap();
    let now = f.service.get(&p.id).unwrap();
    assert_eq!((now.name.as_str(), now.color.as_str(), now.share_memory), ("Job", "#E06C75", Some(true)));
    f.service.update(&p.id, &ProfilePatch { share_memory: Some(false), ..Default::default() }).unwrap();
    assert_eq!(f.service.get(&p.id).unwrap().share_memory, None);
    assert!(!f.stored_text().contains("shareMemory"));

    // a refused part leaves the rest unchanged
    let err = f.service.update(&p.id, &ProfilePatch { name: Some("Other".into()), color: Some("bad".into()), share_memory: None });
    assert_eq!(err.unwrap_err().to_string(), "Invalid color");
    assert_eq!(f.service.get(&p.id).unwrap().name, "Job");

    let main = f.service.update(MAIN_ID, &ProfilePatch { share_memory: Some(true), ..Default::default() });
    assert_eq!(main.unwrap_err().to_string(), "The main profile has the memory the others can share.");
    assert_eq!(f.service.update("gone", &ProfilePatch::default()).unwrap_err().to_string(), "Profile not found");
}

// ---------- syncing ----------

#[test]
fn syncs_from_main_and_asks_before_overwriting_local_edits() {
    let f = Fixture::new();
    let main = f.main_dir();
    fs::write(main.join("CLAUDE.md"), "v1").unwrap();
    let (p, _) = f.service.create(&CreateProfileInput { share_files: vec!["CLAUDE.md".into()], ..input("Work") }).unwrap();
    let dir = PathBuf::from(p.config_dir.as_deref().unwrap());

    fs::write(dir.join("CLAUDE.md"), "my edit").unwrap();
    fs::write(main.join("CLAUDE.md"), "v2").unwrap();
    fs::create_dir_all(main.join("commands")).unwrap();
    let asked = f.service.sync_from_main(&p.id, false).unwrap();
    assert!(asked.needs_confirm);
    assert_eq!(asked.local_changed, vec!["CLAUDE.md"]);
    assert_eq!(fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), "my edit");

    let forced = f.service.sync_from_main(&p.id, true).unwrap();
    assert!(!forced.needs_confirm);
    assert_eq!(forced.copied, vec!["CLAUDE.md"]);
    assert_eq!(forced.backed_up, vec!["CLAUDE.md.local-backup"]);
    assert_eq!(fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), "v2");
    assert_eq!(f.service.view(&p.id).unwrap().sync, Some(SyncStatus::default()));

    assert_eq!(f.service.sync_from_main(MAIN_ID, false).unwrap_err().to_string(), "The main profile is not synced");
}

// ---------- status ----------

/// A stand-in for `claude auth`: answers from a table and counts logouts.
#[derive(Default)]
struct FakeCli {
    answers: Mutex<HashMap<String, AccountInfo>>,
    logouts: AtomicUsize,
}

impl AccountCli for FakeCli {
    fn available(&self) -> bool {
        true
    }
    fn status(&self, profile: Profile) -> BoxFuture<'static, AccountInfo> {
        let answer = self.answers.lock().unwrap().get(&profile.id).cloned().unwrap_or_else(|| signed_in(None));
        Box::pin(async move { answer })
    }
    fn logout(&self, profile: Profile) -> BoxFuture<'static, ()> {
        self.logouts.fetch_add(1, Ordering::SeqCst);
        self.answers.lock().unwrap().insert(profile.id, AccountInfo { logged_in: false, ..signed_in(None) });
        Box::pin(async {})
    }
}

fn signed_in(email: Option<&str>) -> AccountInfo {
    AccountInfo {
        logged_in: email.is_some(),
        email: email.map(str::to_owned),
        org_id: Some("org".into()),
        org_name: None,
        subscription_type: None,
        auth_method: None,
        checked_at: 1,
        error: None,
    }
}

#[test]
fn refreshes_the_status_and_warns_about_twins() {
    let f = Fixture::new();
    let (work, _) = f.service.create(&input("Work")).unwrap();
    // nothing happens without claude
    assert_eq!(block_on(f.service.refresh_status(&work.id)), None);

    let cli = Arc::new(FakeCli::default());
    cli.answers.lock().unwrap().insert(MAIN_ID.into(), signed_in(Some("a@b.co")));
    cli.answers.lock().unwrap().insert(work.id.clone(), signed_in(Some("a@b.co")));
    f.service.set_cli(cli.clone());

    assert_eq!(block_on(f.service.refresh_status(MAIN_ID)), None);
    let warning = block_on(f.service.refresh_status(&work.id));
    assert_eq!(warning.as_deref(), Some("\"Work\" and \"Main\" are signed in to the same account (a@b.co)."));
    assert_eq!(f.service.get(&work.id).unwrap().account.unwrap().email.as_deref(), Some("a@b.co"));

    let warnings = block_on(f.service.refresh_all());
    assert_eq!(warnings.len(), 2);
    assert_eq!(block_on(f.service.refresh_status("gone")), None);
}

#[test]
fn logout_closes_the_sessions_first() {
    let f = Fixture::new();
    let (work, _) = f.service.create(&input("Work")).unwrap();
    assert_eq!(block_on(f.service.logout(&work.id)).unwrap_err().to_string(), "claude was not found");

    let closed = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = closed.clone();
    f.service.set_close_sessions(move |id| seen.lock().unwrap().push(id.to_owned()));
    let cli = Arc::new(FakeCli::default());
    cli.answers.lock().unwrap().insert(work.id.clone(), signed_in(Some("w@b.co")));
    f.service.set_cli(cli.clone());
    block_on(f.service.refresh_status(&work.id));

    block_on(f.service.logout(&work.id)).unwrap();
    assert_eq!(*closed.lock().unwrap(), vec![work.id.clone()]);
    assert_eq!(cli.logouts.load(Ordering::SeqCst), 1);
    assert!(!f.service.get(&work.id).unwrap().account.unwrap().logged_in);
}

// ---------- deleting ----------

#[test]
fn deletes_an_account_without_reaching_into_main() {
    let f = Fixture::new();
    let main = f.main_dir();
    fs::create_dir_all(main.join("skills").join("deploy")).unwrap();
    fs::write(main.join("skills").join("deploy").join("SKILL.md"), "skill").unwrap();
    let (work, _) = f.service.create(&CreateProfileInput { share_dirs: vec!["skills".into()], ..input("Work") }).unwrap();
    let (ci, _) = f.service.create(&token_input("CI", Some(TOKEN))).unwrap();
    let dir = PathBuf::from(work.config_dir.as_deref().unwrap());
    assert!(sharing::is_link(&dir.join("skills")));

    let cli = Arc::new(FakeCli::default());
    f.service.set_cli(cli.clone());
    f.service.set_account(&work.id, signed_in(Some("w@b.co"))).unwrap();
    let closed = Arc::new(AtomicUsize::new(0));
    let count = closed.clone();
    f.service.set_close_sessions(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    });
    let changes = Arc::new(AtomicUsize::new(0));
    let changed = changes.clone();
    f.service.set_on_changed(move || {
        changed.fetch_add(1, Ordering::SeqCst);
    });

    block_on(f.service.delete(&work.id)).unwrap();
    assert!(!dir.exists());
    assert_eq!(fs::read_to_string(main.join("skills").join("deploy").join("SKILL.md")).unwrap(), "skill");
    assert!(f.service.get(&work.id).is_none());
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(cli.logouts.load(Ordering::SeqCst), 1);
    assert_eq!(changes.load(Ordering::SeqCst), 1);

    // a token account is not logged out, and its token goes
    block_on(f.service.delete(&ci.id)).unwrap();
    assert_eq!(cli.logouts.load(Ordering::SeqCst), 1);
    assert!(f.tokens.is_empty());
    assert_eq!(f.stored().len(), 1);
}

#[test]
fn refuses_to_delete_main_or_a_folder_outside_the_profiles() {
    let f = Fixture::new();
    assert_eq!(block_on(f.service.delete(MAIN_ID)).unwrap_err().to_string(), "The main profile cannot be deleted");
    let (work, _) = f.service.create(&input("Work")).unwrap();
    let outside = f.tmp.path().join("elsewhere");
    fs::create_dir_all(&outside).unwrap();
    let moved = outside.to_string_lossy().into_owned();
    f.service.config().update(|c| c.profiles.iter_mut().find(|p| p.id == work.id).unwrap().config_dir = Some(moved)).unwrap();
    let err = block_on(f.service.delete(&work.id)).unwrap_err().to_string();
    assert_eq!(err, "The profile folder is not where it is expected; nothing was deleted");
    assert!(outside.is_dir());
    assert!(f.service.get(&work.id).is_some());
    assert_eq!(block_on(f.service.delete("gone")).unwrap_err().to_string(), "Profile not found");
}

#[test]
fn deletes_an_account_whose_folder_is_already_gone() {
    let f = Fixture::new();
    let (work, _) = f.service.create(&input("Work")).unwrap();
    fs::remove_dir_all(work.config_dir.as_deref().unwrap()).unwrap();
    block_on(f.service.delete(&work.id)).unwrap();
    assert!(f.service.get(&work.id).is_none());
}

// ---------- folders and launchers ----------

#[test]
fn pins_folders_to_accounts_that_exist() {
    let f = Fixture::new();
    let (work, _) = f.service.create(&input("Work")).unwrap();
    let folder = if cfg!(windows) { r"C:\Code\App" } else { "/code/app" };
    f.service.set_folder_account(folder, Some(&work.id)).unwrap();
    assert_eq!(f.service.pinned_account(folder).as_deref(), Some(work.id.as_str()));
    assert_eq!(f.service.set_folder_account(folder, Some("gone")).unwrap_err().to_string(), "Profile not found");

    block_on(f.service.delete(&work.id)).unwrap();
    assert_eq!(f.service.pinned_account(folder), None);
    // the stale pin goes with the next change
    f.service.set_folder_account("/other", None).unwrap();
    assert!(f.service.config().read().folder_accounts.is_empty());
}

#[test]
fn rewrites_the_launchers_when_asked() {
    let f = Fixture::new();
    f.service.create(&input("Work")).unwrap();
    f.service.create(&token_input("CI", Some(TOKEN))).unwrap();
    f.service.refresh_launchers(None);
    assert!(!f.service.paths().bin_dir().exists());
    f.service.refresh_launchers(Some("/usr/bin/claude"));
    let mut names: Vec<String> = fs::read_dir(f.service.paths().bin_dir()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let ext = if cfg!(windows) { ".cmd" } else { "" };
    assert_eq!(names, vec![format!("claude-main{ext}"), format!("claude-work{ext}")]);
}
