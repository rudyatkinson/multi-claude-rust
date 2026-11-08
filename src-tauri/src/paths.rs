//! The app's folders under `~/.multi-claude` (`src/main/paths.ts`).
//!
//! The root is `~/.multi-claude`, or `MULTI_CLAUDE_HOME` when set (an isolated instance for
//! development); `AppState` decides which and hands it to [`Paths::new`].

use std::path::{Path, PathBuf};

use mc_core::types::Profile;

/// Where the app keeps its files, and where Claude Code keeps the main account's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
    home: PathBuf,
}

impl Paths {
    /// The app's folders under `root`, with the user's home folder for the main account.
    pub fn new(root: PathBuf) -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        Paths { root, home }
    }

    /// With another home folder: the main account's `.claude` and `.claude.json` are looked for
    /// there. For tests, which must never touch the real ones.
    pub fn with_home(root: PathBuf, home: PathBuf) -> Self {
        Paths { root, home }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The user's home folder.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// One folder per account other than Main: its CLAUDE_CONFIG_DIR.
    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    /// The settings files of running sessions.
    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }

    /// The `claude-<slug>` launchers.
    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    /// Kept empty: the usage probe runs here so that it loads no project settings.
    pub fn probe_dir(&self) -> PathBuf {
        self.root.join("probe")
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn tabs_file(&self) -> PathBuf {
        self.root.join("tabs.json")
    }

    pub fn workflows_file(&self) -> PathBuf {
        self.root.join("workflows.json")
    }

    pub fn usage_cache_file(&self) -> PathBuf {
        self.root.join("usage-cache.json")
    }

    /// The last failed turns and limit waits, as Claude Code reported them.
    pub fn limit_events_file(&self) -> PathBuf {
        self.root.join("limit-events.json")
    }

    /// One folder per workflow run, with the prompts and the outputs of its steps.
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("workflow-runs")
    }

    /// One folder per request to Claude about a skill.
    pub fn skill_requests_dir(&self) -> PathBuf {
        self.root.join("skill-requests")
    }

    /// Creates the folders the app expects at startup: the root, profiles, run, bin and probe.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [self.root.clone(), self.profiles_dir(), self.run_dir(), self.bin_dir(), self.probe_dir()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// The default Claude Code config dir, `~/.claude`; the main account lives here.
    pub fn main_claude_dir(&self) -> PathBuf {
        self.home.join(".claude")
    }

    /// `effectiveConfigDir`: the account's CLAUDE_CONFIG_DIR, or `~/.claude` for Main.
    pub fn config_dir_of(&self, profile: &Profile) -> PathBuf {
        match &profile.config_dir {
            Some(dir) => PathBuf::from(dir),
            None => self.main_claude_dir(),
        }
    }

    /// `claudeJsonFile`: where Claude Code keeps an account's user-level MCP servers, `.claude.json`
    /// in the home folder for Main and inside the config dir otherwise.
    pub fn claude_json_of(&self, config_dir: Option<&str>) -> PathBuf {
        match config_dir {
            Some(dir) => Path::new(dir).join(".claude.json"),
            None => self.home.join(".claude.json"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::types::ShareConfig;

    fn profile(config_dir: Option<&str>) -> Profile {
        Profile {
            id: "p".into(),
            name: "P".into(),
            slug: "p".into(),
            color: "#5b9bd5".into(),
            is_default: config_dir.is_none(),
            config_dir: config_dir.map(str::to_owned),
            share: ShareConfig::default(),
            email_hint: None,
            auth: None,
            token_enc: None,
            last_sync: None,
            account: None,
            share_memory: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn files_live_under_the_root() {
        let paths = Paths::with_home(PathBuf::from("/r"), PathBuf::from("/h"));
        assert_eq!(paths.profiles_dir(), Path::new("/r").join("profiles"));
        assert_eq!(paths.config_file(), Path::new("/r").join("config.json"));
        assert_eq!(paths.runs_dir(), Path::new("/r").join("workflow-runs"));
        assert_eq!(paths.main_claude_dir(), Path::new("/h").join(".claude"));
    }

    #[test]
    fn main_uses_the_home_folder() {
        let paths = Paths::with_home(PathBuf::from("/r"), PathBuf::from("/h"));
        assert_eq!(paths.config_dir_of(&profile(None)), Path::new("/h").join(".claude"));
        assert_eq!(paths.config_dir_of(&profile(Some("/r/profiles/w"))), PathBuf::from("/r/profiles/w"));
        assert_eq!(paths.claude_json_of(None), Path::new("/h").join(".claude.json"));
        assert_eq!(paths.claude_json_of(Some("/r/profiles/elif")), Path::new("/r/profiles/elif").join(".claude.json"));
    }

    #[test]
    fn ensure_dirs_creates_the_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_home(tmp.path().join("mc"), tmp.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        for dir in [paths.profiles_dir(), paths.run_dir(), paths.bin_dir(), paths.probe_dir()] {
            assert!(dir.is_dir());
        }
    }
}
