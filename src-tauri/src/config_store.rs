//! `config.json` with atomic writes (`src/main/configStore.ts`).
//!
//! The store holds the parsed file in memory behind a lock; every change goes through
//! [`ConfigStore::update`] or [`ConfigStore::try_update`], which write the whole file again.

use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mc_core::error::{AppError, AppResult};
use mc_core::folders::remember_recent;
use mc_core::types::{AppConfig, AppSettings};
use serde::Serialize;

/// Epoch milliseconds, as `Date.now()`.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `path` with `suffix` appended to its last part: `config.json` + `.tmp-1` = `config.json.tmp-1`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// `writeJsonAtomic`: `JSON.stringify(value, null, 2)` and a newline, written to
/// `<file>.tmp-<pid>-<n>` and renamed over the file, so a reader never sees half a file. Windows
/// can refuse the rename while another process has the target open; it is retried once.
pub fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    // the sequence number keeps two threads that write the same file apart
    let tmp = with_suffix(path, &format!(".tmp-{}-{}", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
    fs::write(&tmp, text)?;
    if fs::rename(&tmp, path).is_err() {
        std::thread::sleep(Duration::from_millis(50));
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(AppError::io(format!("Could not save {}: {e}", path.display())));
        }
    }
    Ok(())
}

/// Reads the config file. A missing file is a fresh start; one that cannot be read as a config
/// is kept next to it as `config.json.corrupt-<ms>` before the defaults take its place, so the
/// next save does not destroy what the user had.
fn load(file: &Path) -> AppConfig {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return AppConfig::default(),
        Err(e) => {
            log::warn!("config: cannot read {}: {e}; starting from the defaults", file.display());
            return AppConfig::default();
        }
    };
    // a file saved by an editor may start with a byte order mark
    match serde_json::from_str::<AppConfig>(text.trim_start_matches('\u{feff}')) {
        Ok(config) => config,
        Err(e) => {
            let backup = with_suffix(file, &format!(".corrupt-{}", now_ms()));
            match fs::copy(file, &backup) {
                Ok(_) => log::warn!("config: {} is damaged ({e}); kept as {}", file.display(), backup.display()),
                Err(copy) => log::error!("config: {} is damaged ({e}) and could not be kept: {copy}", file.display()),
            }
            AppConfig::default()
        }
    }
}

/// `config.json`: the accounts, the settings, the recent folders and the folder pins.
pub struct ConfigStore {
    file: PathBuf,
    data: Mutex<AppConfig>,
}

impl ConfigStore {
    /// The store of `<root>/config.json`. Reads the file; writes nothing until the first change.
    pub fn new(root: PathBuf) -> Self {
        Self::open(root.join("config.json"))
    }

    /// The store of any file.
    pub fn open(file: PathBuf) -> Self {
        let data = load(&file);
        ConfigStore { file, data: Mutex::new(data) }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    fn lock(&self) -> MutexGuard<'_, AppConfig> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A copy of the whole config.
    pub fn read(&self) -> AppConfig {
        self.lock().clone()
    }

    /// Reads part of the config without copying all of it.
    pub fn with<R>(&self, f: impl FnOnce(&AppConfig) -> R) -> R {
        f(&self.lock())
    }

    /// The app's settings.
    pub fn settings(&self) -> AppSettings {
        self.lock().settings.clone()
    }

    /// Changes the config and saves it. When the write fails, the change stands in memory (as it
    /// did in the Electron store) and the error is returned; the next save writes it.
    pub fn update<R>(&self, f: impl FnOnce(&mut AppConfig) -> R) -> AppResult<R> {
        let mut data = self.lock();
        let out = f(&mut data);
        write_json_atomic(&self.file, &*data)?;
        Ok(out)
    }

    /// Like [`update`](Self::update) for a change that can be refused: `f` works on a copy, and
    /// when it fails nothing changes and nothing is written.
    pub fn try_update<R>(&self, f: impl FnOnce(&mut AppConfig) -> AppResult<R>) -> AppResult<R> {
        let mut data = self.lock();
        let mut next = data.clone();
        let out = f(&mut next)?;
        *data = next;
        write_json_atomic(&self.file, &*data)?;
        Ok(out)
    }

    /// Writes the config as it is.
    pub fn save(&self) -> AppResult<()> {
        let data = self.lock();
        write_json_atomic(&self.file, &*data)
    }

    /// `rememberFolder`: puts a folder at the top of the recent folders (at most 12, no other
    /// spelling of it further down; letter case does not count on Windows).
    pub fn remember_folder(&self, cwd: &str) -> AppResult<()> {
        self.update(|c| c.recent_folders = remember_recent(&c.recent_folders, cwd, cfg!(windows)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(tmp.path().to_path_buf());
        assert_eq!(store.read(), AppConfig::default());
        assert!(files(tmp.path()).is_empty());
    }

    #[test]
    fn a_damaged_file_is_kept_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("config.json");
        fs::write(&file, "{ not json").unwrap();
        let store = ConfigStore::new(tmp.path().to_path_buf());
        assert_eq!(store.read(), AppConfig::default());
        let names = files(tmp.path());
        let backup = names.iter().find(|n| n.starts_with("config.json.corrupt-")).expect("a backup");
        assert_eq!(fs::read_to_string(tmp.path().join(backup)).unwrap(), "{ not json");
        // the damaged file itself stays until the first save
        assert_eq!(fs::read_to_string(&file).unwrap(), "{ not json");
    }

    #[test]
    fn a_broken_profile_does_not_silently_drop_the_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("config.json"), r#"{"version":1,"profiles":[{"id":5}]}"#).unwrap();
        let store = ConfigStore::new(tmp.path().to_path_buf());
        assert!(store.read().profiles.is_empty());
        assert!(files(tmp.path()).iter().any(|n| n.starts_with("config.json.corrupt-")));
    }

    #[test]
    fn saves_as_indented_json_and_keeps_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("config.json");
        fs::write(&file, "\u{feff}{\"version\":1,\"future\":{\"x\":1},\"settings\":{\"fontSize\":15}}").unwrap();
        let store = ConfigStore::new(tmp.path().to_path_buf());
        assert_eq!(store.settings().font_size, 15);
        store.update(|c| c.recent_folders.push("C:\\code".into())).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("{\n  \"version\": 1,"));
        assert!(text.ends_with("}\n"));
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["future"], json!({ "x": 1 }));
        assert_eq!(saved["recentFolders"], json!(["C:\\code"]));
        assert_eq!(ConfigStore::new(tmp.path().to_path_buf()).read(), store.read());
        // no temporary file is left behind
        assert_eq!(files(tmp.path()), vec!["config.json".to_owned()]);
    }

    #[test]
    fn a_refused_change_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(tmp.path().to_path_buf());
        let result: AppResult<()> = store.try_update(|c| {
            c.recent_folders.push("x".into());
            Err(AppError::invalid("no"))
        });
        assert_eq!(result.unwrap_err().to_string(), "no");
        assert!(store.read().recent_folders.is_empty());
        assert!(files(tmp.path()).is_empty());
    }

    #[test]
    fn remembers_recent_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(tmp.path().to_path_buf());
        for i in 0..14 {
            store.remember_folder(&format!("/code/p{i}")).unwrap();
        }
        store.remember_folder("/code/p5").unwrap();
        let recent = store.read().recent_folders;
        assert_eq!(recent.len(), 12);
        assert_eq!(recent[0], "/code/p5");
        assert_eq!(recent[1], "/code/p13");
        assert_eq!(recent.iter().filter(|f| *f == "/code/p5").count(), 1);
    }

    #[test]
    fn write_json_atomic_makes_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a").join("b.json");
        write_json_atomic(&file, &json!({ "k": [1, 2] })).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "{\n  \"k\": [\n    1,\n    2\n  ]\n}\n");
        write_json_atomic(&file, &json!({})).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "{}\n");
    }
}
