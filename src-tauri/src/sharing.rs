//! Linked folders and copied files between the main account and the others (`src/main/sharing.ts`).
//!
//! A shared folder (agents, skills, ...) is a link from the account's config dir to the main
//! account's folder of the same name: a junction on Windows, which needs no special rights, and a
//! symlink elsewhere. A shared file is copied, and `settings.json` is merged so that the account
//! keeps its own login and, on request, its own model.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use mc_core::settings_merge::{fingerprint_bytes, fingerprint_settings, merge_settings, to_js_json_pretty};
use mc_core::types::{Profile, ShareConfig, SyncSnapshot, SyncStatus};
use serde_json::Value;

use crate::config_store::now_ms;

/// `SHAREABLE_DIRS`: the folders an account can link from Main.
pub const SHARED_DIRS: [&str; 4] = ["agents", "skills", "commands", "plugins"];
/// `SHAREABLE_FILES`: the files an account can copy from Main.
pub const SHARED_FILES: [&str; 3] = ["settings.json", "CLAUDE.md", "keybindings.json"];
/// `MODEL_SETTINGS_KEYS`: the settings an account with "keep the model separate" keeps for itself.
pub const MODEL_KEYS: [&str; 3] = ["model", "effortLevel", "modelSettings"];

const SETTINGS_FILE: &str = "settings.json";

fn other(msg: String) -> io::Error {
    io::Error::other(msg)
}

/// True for a symlink or a junction (lstat: the link itself, not what it points at).
pub fn is_link(path: &Path) -> bool {
    // std reports both symlinks and junctions (name surrogate reparse points) as symlinks
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return true;
    }
    #[cfg(windows)]
    {
        junction::exists(path).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Removes the link itself and never what it points at.
pub fn remove_link(path: &Path) -> io::Result<()> {
    if !is_link(path) {
        return Err(other(format!("Not a link: {}", path.display())));
    }
    // a file symlink goes with remove_file; a junction or a directory symlink on Windows with remove_dir
    fs::remove_file(path).or_else(|_| fs::remove_dir(path))
}

fn make_link(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        junction::create(target, link)
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link)
    }
}

fn same_target(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase(),
        _ => false,
    }
}

/// What [`apply_dir_links`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkResult {
    pub linked: Vec<String>,
    /// Names of the local folders that were renamed to make room for a link.
    pub backed_up: Vec<String>,
    pub warnings: Vec<String>,
}

/// Links each shared folder of the account to the main account's folder of the same name. A
/// folder Main does not have is skipped; a link that already points there is kept; another link
/// is replaced; an empty local folder is removed; a local folder with something in it is renamed
/// to `<name>.local-backup-<ms>`.
pub fn apply_dir_links<S: AsRef<str>>(main_dir: &Path, profile_dir: &Path, names: &[S]) -> LinkResult {
    let mut result = LinkResult::default();
    for name in names.iter().map(AsRef::as_ref) {
        let src = main_dir.join(name);
        let dst = profile_dir.join(name);
        let mut step = || -> io::Result<bool> {
            if !src.is_dir() {
                return Ok(false);
            }
            match fs::symlink_metadata(&dst) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
                Ok(_) if is_link(&dst) => {
                    if same_target(&dst, &src) {
                        return Ok(false);
                    }
                    remove_link(&dst)?;
                }
                Ok(meta) if meta.is_dir() && fs::read_dir(&dst)?.next().is_none() => fs::remove_dir(&dst)?,
                Ok(_) => {
                    let backup = format!("{name}.local-backup-{}", now_ms());
                    fs::rename(&dst, profile_dir.join(&backup))?;
                    result.backed_up.push(backup);
                }
            }
            make_link(&src, &dst)?;
            Ok(true)
        };
        match step() {
            Ok(true) => result.linked.push(name.to_owned()),
            Ok(false) => {}
            Err(e) => result.warnings.push(format!("{name}: {e}")),
        }
    }
    result
}

/// Shared folders that exist in Main but are not linked from the account (yet).
pub fn missing_dir_links<S: AsRef<str>>(main_dir: &Path, profile_dir: &Path, names: &[S]) -> Vec<String> {
    names
        .iter()
        .map(AsRef::as_ref)
        .filter(|name| {
            let src = main_dir.join(name);
            src.is_dir() && !same_target(&profile_dir.join(name), &src)
        })
        .map(str::to_owned)
        .collect()
}

/// What [`remove_all_links`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemovedLinks {
    pub removed: Vec<String>,
    /// Links that are still there.
    pub remaining: Vec<String>,
}

fn link_names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if is_link(&entry.path()) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

/// Removes every top-level link in the account's folder, so that deleting the folder cannot reach
/// through a junction into the main account's.
pub fn remove_all_links(dir: &Path) -> io::Result<RemovedLinks> {
    let mut removed = Vec::new();
    for name in link_names(dir)? {
        if remove_link(&dir.join(&name)).is_ok() {
            removed.push(name);
        }
    }
    Ok(RemovedLinks { removed, remaining: link_names(dir)? })
}

/// A JSON object from a file; None when the file is missing, unreadable or not an object.
fn read_json_object(file: &Path) -> Option<Value> {
    let text = fs::read_to_string(file).ok()?;
    serde_json::from_str::<Value>(&text).ok().filter(Value::is_object)
}

/// The fingerprint of a shared file: for `settings.json` the part that sync owns (see
/// [`fingerprint_settings`]), else the sha256 of its bytes. None when the file is missing, or
/// for a `settings.json` that is not a JSON object.
pub fn fingerprint(file: &Path, name: &str, local_keys: &[String]) -> io::Result<Option<String>> {
    if !file.exists() {
        return Ok(None);
    }
    if name == SETTINGS_FILE {
        return Ok(read_json_object(file).map(|json| fingerprint_settings(&json, local_keys)));
    }
    Ok(Some(fingerprint_bytes(&fs::read(file)?)))
}

/// The fingerprints of an account's shared files that are there, by name.
pub fn fingerprints(dir: &Path, share: &ShareConfig) -> io::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for name in &share.files {
        if let Some(fp) = fingerprint(&dir.join(name), name, &share.local_settings_keys)? {
            out.insert(name.clone(), fp);
        }
    }
    Ok(out)
}

/// Which shared files changed in Main and which in the account since the last sync.
pub fn sync_status(
    main_dir: &Path,
    profile_dir: &Path,
    share: &ShareConfig,
    snapshot: Option<&SyncSnapshot>,
) -> io::Result<SyncStatus> {
    let mut status = SyncStatus::default();
    let keys = &share.local_settings_keys;
    for name in &share.files {
        let Some(main_fp) = fingerprint(&main_dir.join(name), name, keys)? else {
            continue;
        };
        let local_fp = fingerprint(&profile_dir.join(name), name, keys)?;
        if local_fp.as_deref() == Some(main_fp.as_str()) {
            continue;
        }
        if snapshot.and_then(|s| s.main_hashes.get(name)) != Some(&main_fp) {
            status.main_changed.push(name.clone());
        }
        if let (Some(local_fp), Some(snapshot)) = (&local_fp, snapshot) {
            if snapshot.local_hashes.get(name) != Some(local_fp) {
                status.local_changed.push(name.clone());
            }
        }
    }
    Ok(status)
}

/// The sync badge of an account in the window: its changed files, and the shared folders it does
/// not link yet among Main's changes. None for Main, or when the files cannot be read.
pub fn profile_sync_status(profile: &Profile, main_dir: &Path) -> Option<SyncStatus> {
    let dir = Path::new(profile.config_dir.as_deref()?);
    let mut status = sync_status(main_dir, dir, &profile.share, profile.last_sync.as_ref()).ok()?;
    status.main_changed.extend(missing_dir_links(main_dir, dir, &profile.share.dirs));
    Some(status)
}

/// What [`sync_files`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileSyncResult {
    /// True when nothing was written because local edits would be lost; retry with `force`.
    pub needs_confirm: bool,
    pub local_changed: Vec<String>,
    pub copied: Vec<String>,
    pub backed_up: Vec<String>,
    pub warnings: Vec<String>,
    /// The state to compare with next time; None when nothing was written.
    pub snapshot: Option<SyncSnapshot>,
}

/// Copies the shared files from Main. Without `force`, nothing is written if local edits would be
/// lost; with it, each edited file is first copied to `<name>.local-backup`. `settings.json` is
/// merged (see [`merge_settings`]) and written as 2-space JSON with a newline.
pub fn sync_files(
    main_dir: &Path,
    profile_dir: &Path,
    share: &ShareConfig,
    snapshot: Option<&SyncSnapshot>,
    force: bool,
) -> io::Result<FileSyncResult> {
    let local_changed = sync_status(main_dir, profile_dir, share, snapshot)?.local_changed;
    let mut result = FileSyncResult { local_changed, ..Default::default() };
    if !result.local_changed.is_empty() && !force {
        result.needs_confirm = true;
        return Ok(result);
    }

    let keys = &share.local_settings_keys;
    let mut next = SyncSnapshot { at: now_ms(), main_hashes: BTreeMap::new(), local_hashes: BTreeMap::new() };
    for name in &share.files {
        let src = main_dir.join(name);
        let dst = profile_dir.join(name);
        if !src.exists() {
            continue;
        }
        let mut step = || -> io::Result<()> {
            if result.local_changed.contains(name) {
                let backup = format!("{name}.local-backup");
                fs::copy(&dst, profile_dir.join(&backup))?;
                result.backed_up.push(backup);
            }
            if name == SETTINGS_FILE {
                let Some(main) = read_json_object(&src) else {
                    result.warnings.push("settings.json: could not read the main profile's file".to_owned());
                    return Ok(());
                };
                let merged = merge_settings(&main, read_json_object(&dst).as_ref(), keys);
                fs::write(&dst, to_js_json_pretty(&merged) + "\n")?;
            } else {
                fs::copy(&src, &dst)?;
            }
            result.copied.push(name.clone());
            let fp = fingerprint(&src, name, keys)?.unwrap_or_default();
            let local = fingerprint(&dst, name, keys)?.unwrap_or_else(|| fp.clone());
            next.main_hashes.insert(name.clone(), fp);
            next.local_hashes.insert(name.clone(), local);
            Ok(())
        };
        if let Err(e) = step() {
            result.warnings.push(format!("{name}: {e}"));
        }
    }
    result.snapshot = Some(next);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    struct Dirs {
        _tmp: tempfile::TempDir,
        main: PathBuf,
        profile: PathBuf,
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            // links first, so the recursive delete can never follow one
            if self.profile.exists() {
                let _ = remove_all_links(&self.profile);
            }
        }
    }

    fn dirs() -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        let profile = tmp.path().join("profile");
        fs::create_dir(&main).unwrap();
        fs::create_dir(&profile).unwrap();
        Dirs { _tmp: tmp, main, profile }
    }

    fn write(file: &Path, content: &str) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn read(file: &Path) -> String {
        fs::read_to_string(file).unwrap()
    }

    fn read_json(file: &Path) -> Value {
        serde_json::from_str(&read(file)).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    // ---- directory links ----

    #[test]
    fn links_shared_dirs_and_removing_the_links_leaves_main_untouched() {
        let d = dirs();
        write(&d.main.join("agents").join("reviewer.md"), "agent");
        write(&d.main.join("skills").join("deploy").join("SKILL.md"), "skill");

        let result = apply_dir_links(&d.main, &d.profile, &["agents", "skills", "commands"]);
        assert_eq!(result.linked, vec!["agents", "skills"]);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert!(is_link(&d.profile.join("agents")));
        assert_eq!(read(&d.profile.join("agents").join("reviewer.md")), "agent");

        // a file added in Main shows up through the link
        write(&d.main.join("skills").join("new").join("SKILL.md"), "new");
        assert!(d.profile.join("skills").join("new").join("SKILL.md").exists());

        let mut gone = remove_all_links(&d.profile).unwrap();
        gone.removed.sort();
        assert_eq!(gone.removed, vec!["agents", "skills"]);
        assert!(gone.remaining.is_empty());
        assert!(names(&d.profile).is_empty());
        assert_eq!(read(&d.main.join("agents").join("reviewer.md")), "agent");
        assert_eq!(read(&d.main.join("skills").join("deploy").join("SKILL.md")), "skill");
    }

    #[test]
    fn is_idempotent_and_reports_missing_links() {
        let d = dirs();
        write(&d.main.join("agents").join("a.md"), "a");
        assert_eq!(missing_dir_links(&d.main, &d.profile, &["agents", "commands"]), vec!["agents"]);
        apply_dir_links(&d.main, &d.profile, &["agents"]);
        assert!(apply_dir_links(&d.main, &d.profile, &["agents"]).linked.is_empty());
        assert!(missing_dir_links(&d.main, &d.profile, &["agents", "commands"]).is_empty());
    }

    #[test]
    fn keeps_a_non_empty_local_directory_as_a_backup_instead_of_deleting_it() {
        let d = dirs();
        write(&d.main.join("agents").join("main.md"), "main");
        write(&d.profile.join("agents").join("mine.md"), "mine");

        let result = apply_dir_links(&d.main, &d.profile, &["agents"]);
        assert_eq!(result.backed_up.len(), 1);
        assert!(result.backed_up[0].starts_with("agents.local-backup-"));
        assert_eq!(read(&d.profile.join(&result.backed_up[0]).join("mine.md")), "mine");
        assert!(d.profile.join("agents").join("main.md").exists());
    }

    #[test]
    fn replaces_an_empty_folder_and_a_link_elsewhere() {
        let d = dirs();
        write(&d.main.join("agents").join("a.md"), "a");
        write(&d.main.join("skills").join("s.md"), "s");
        fs::create_dir(d.profile.join("agents")).unwrap();
        let elsewhere = d.main.parent().unwrap().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        make_link(&elsewhere, &d.profile.join("skills")).unwrap();

        let result = apply_dir_links(&d.main, &d.profile, &["agents", "skills"]);
        assert_eq!(result.linked, vec!["agents", "skills"]);
        assert!(result.backed_up.is_empty());
        assert!(d.profile.join("skills").join("s.md").exists());
        assert!(elsewhere.is_dir());
    }

    #[test]
    fn remove_link_refuses_a_real_folder() {
        let d = dirs();
        fs::create_dir(d.profile.join("real")).unwrap();
        assert!(remove_link(&d.profile.join("real")).is_err());
        assert!(!is_link(&d.profile.join("real")));
        assert!(d.profile.join("real").is_dir());
    }

    // ---- file sync ----

    fn share() -> ShareConfig {
        ShareConfig {
            dirs: vec![],
            files: SHARED_FILES.iter().map(|s| (*s).to_owned()).collect(),
            local_settings_keys: vec!["model".to_owned()],
        }
    }

    fn status(main: &[&str], local: &[&str]) -> SyncStatus {
        SyncStatus {
            main_changed: main.iter().map(|s| (*s).to_owned()).collect(),
            local_changed: local.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn copies_files_then_tracks_changes_on_either_side() {
        let d = dirs();
        let share = share();
        write(&d.main.join("settings.json"), r#"{"model":"opus","theme":"dark","apiKeyHelper":"x"}"#);
        write(&d.main.join("CLAUDE.md"), "rules");

        assert_eq!(sync_status(&d.main, &d.profile, &share, None).unwrap().main_changed, vec!["settings.json", "CLAUDE.md"]);

        let first = sync_files(&d.main, &d.profile, &share, None, false).unwrap();
        assert_eq!(first.copied, vec!["settings.json", "CLAUDE.md"]);
        assert_eq!(read_json(&d.profile.join("settings.json")), json!({ "model": "opus", "theme": "dark" }));
        let snap = first.snapshot.as_ref();
        assert_eq!(sync_status(&d.main, &d.profile, &share, snap).unwrap(), status(&[], &[]));

        // changing a local key in the profile is not drift
        write(&d.profile.join("settings.json"), r#"{"model":"sonnet","theme":"dark"}"#);
        assert_eq!(sync_status(&d.main, &d.profile, &share, snap).unwrap(), status(&[], &[]));

        // Main changes: flagged, and the sync keeps the profile's model
        write(&d.main.join("settings.json"), r#"{"model":"opus","theme":"light"}"#);
        assert_eq!(sync_status(&d.main, &d.profile, &share, snap).unwrap(), status(&["settings.json"], &[]));
        let second = sync_files(&d.main, &d.profile, &share, snap, false).unwrap();
        assert!(!second.needs_confirm);
        assert_eq!(read_json(&d.profile.join("settings.json")), json!({ "model": "sonnet", "theme": "light" }));
        // written as JSON.stringify(merged, null, 2) + "\n"
        assert_eq!(read(&d.profile.join("settings.json")), "{\n  \"model\": \"sonnet\",\n  \"theme\": \"light\"\n}\n");
    }

    #[test]
    fn does_not_overwrite_local_edits_without_force_and_backs_them_up_with_it() {
        let d = dirs();
        let share = share();
        write(&d.main.join("CLAUDE.md"), "v1");
        let first = sync_files(&d.main, &d.profile, &share, None, false).unwrap();

        write(&d.profile.join("CLAUDE.md"), "my edit");
        write(&d.main.join("CLAUDE.md"), "v2");

        let blocked = sync_files(&d.main, &d.profile, &share, first.snapshot.as_ref(), false).unwrap();
        assert!(blocked.needs_confirm);
        assert_eq!(blocked.local_changed, vec!["CLAUDE.md"]);
        assert!(blocked.snapshot.is_none());
        assert_eq!(read(&d.profile.join("CLAUDE.md")), "my edit");

        let forced = sync_files(&d.main, &d.profile, &share, first.snapshot.as_ref(), true).unwrap();
        assert_eq!(forced.backed_up, vec!["CLAUDE.md.local-backup"]);
        assert_eq!(read(&d.profile.join("CLAUDE.md")), "v2");
        assert_eq!(read(&d.profile.join("CLAUDE.md.local-backup")), "my edit");
    }

    #[test]
    fn warns_about_a_main_settings_file_that_is_not_an_object() {
        let d = dirs();
        write(&d.main.join("settings.json"), "[1]");
        write(&d.main.join("keybindings.json"), "{}");
        let result = sync_files(&d.main, &d.profile, &share(), None, false).unwrap();
        assert_eq!(result.copied, vec!["keybindings.json"]);
        assert_eq!(result.warnings, vec!["settings.json: could not read the main profile's file"]);
        assert!(!d.profile.join("settings.json").exists());
    }

    #[test]
    fn the_badge_of_an_account_names_unlinked_folders() {
        let d = dirs();
        write(&d.main.join("agents").join("a.md"), "a");
        write(&d.main.join("CLAUDE.md"), "rules");
        let mut profile: Profile = serde_json::from_value(json!({
            "id": "w", "name": "W", "slug": "w", "color": "#5b9bd5", "isDefault": false,
            "configDir": d.profile.to_string_lossy(),
            "share": { "dirs": ["agents"], "files": ["CLAUDE.md"], "localSettingsKeys": [] }
        }))
        .unwrap();
        assert_eq!(profile_sync_status(&profile, &d.main), Some(status(&["CLAUDE.md", "agents"], &[])));
        apply_dir_links(&d.main, &d.profile, &profile.share.dirs);
        profile.last_sync = sync_files(&d.main, &d.profile, &profile.share, None, false).unwrap().snapshot;
        assert_eq!(profile_sync_status(&profile, &d.main), Some(status(&[], &[])));
        assert_eq!(fingerprints(&d.profile, &profile.share).unwrap().len(), 1);
        profile.config_dir = None;
        assert_eq!(profile_sync_status(&profile, &d.main), None);
    }
}
