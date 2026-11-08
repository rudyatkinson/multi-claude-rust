//! "Open in Multi Claude" in File Explorer's context menu for folders (`ExplorerMenu` of
//! `src/main/explorerMenu/index.ts`). Windows only; elsewhere it is not supported.
//!
//! There are two tiers. The classic one ([`classic`]) is a registry verb, which Windows 11 shows
//! under "Show more options". The package one puts the entry into the Windows 11 menu itself and
//! needs a sparse package with a COM command; that comes with Phase 4 (TODO(Phase 4): the package
//! files, `Add-AppxPackage` through PowerShell, its status and its removal). Until then the
//! package tier is never chosen: this build has no package files.

pub mod classic;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use mc_core::error::AppResult;
use mc_core::types::{ExplorerMenuStatus, ExplorerMenuTier};

use classic::{install_classic, is_classic_installed, reg_dword, reg_key_exists, remove_classic, ClassicVerb};

pub const MENU_TITLE: &str = "Open in Multi Claude";

const WINDOWS_11_BUILD: u32 = 22000;
const DEV_MODE_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock";
/// Present when the user switched Windows 11 back to the classic context menu.
const CLASSIC_MENU_KEY: &str = r"HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32";

/// What decides the tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierInputs {
    pub packaged: bool,
    pub os_build: Option<u32>,
    pub developer_mode: bool,
    pub classic_menu_forced: bool,
    pub package_files_present: bool,
}

/// `chooseTier`: the Windows 11 menu only takes commands from apps with package identity. The
/// package is registered unsigned, which Windows allows in Developer Mode only; everything else
/// falls back to registry verbs, with the reason.
pub fn choose_tier(i: &TierInputs) -> (ExplorerMenuTier, Option<String>) {
    let classic = |reason: &str| (ExplorerMenuTier::Classic, Some(reason.to_owned()));
    if !i.packaged {
        return classic("A development run can only use the classic menu.");
    }
    if i.os_build.unwrap_or(0) < WINDOWS_11_BUILD {
        return classic("The new context menu needs Windows 11.");
    }
    if i.classic_menu_forced {
        return classic("This PC is set to the classic context menu.");
    }
    if !i.package_files_present {
        return classic("The menu package files are missing.");
    }
    if !i.developer_mode {
        return classic("The Windows 11 menu needs Developer Mode (Settings > System > For developers).");
    }
    (ExplorerMenuTier::Package, None)
}

pub struct ExplorerMenuOptions {
    pub packaged: bool,
    pub os_build: Option<u32>,
    /// Executable the menu entry starts.
    pub exe_path: String,
    /// Arguments that must precede the folder switch.
    pub app_args: Vec<String>,
    /// Folder holding `AppxManifest.xml` and the command DLL; None in this build (Phase 4).
    pub package_dir: Option<PathBuf>,
}

pub struct ExplorerMenu {
    o: ExplorerMenuOptions,
    /// One registry or package operation at a time.
    queue: tokio::sync::Mutex<()>,
    last_error: Mutex<Option<String>>,
}

impl ExplorerMenu {
    pub fn new(o: ExplorerMenuOptions) -> Self {
        ExplorerMenu { o, queue: tokio::sync::Mutex::new(()), last_error: Mutex::new(None) }
    }

    /// The menu of this build: the running executable, packaged or not.
    pub fn for_this_build(os_build: Option<u32>) -> Self {
        let exe_path = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        ExplorerMenu::new(ExplorerMenuOptions { packaged: !tauri::is_dev(), os_build, exe_path, app_args: Vec::new(), package_dir: None })
    }

    fn supported(&self) -> bool {
        cfg!(windows)
    }

    fn last_error(&self) -> MutexGuard<'_, Option<String>> {
        self.last_error.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A development run gets its own verb so it never replaces the installed app's entry.
    fn verb(&self) -> ClassicVerb {
        let (key, title) = if self.o.packaged { ("MultiClaude", MENU_TITLE.to_owned()) } else { ("MultiClaudeDev", format!("{MENU_TITLE} (Dev)")) };
        ClassicVerb { key: key.into(), title, exe: self.o.exe_path.clone(), prefix_args: self.o.app_args.clone() }
    }

    async fn tier_choice(&self) -> (ExplorerMenuTier, Option<String>) {
        let (developer_mode, classic_menu_forced) =
            tokio::join!(reg_dword(DEV_MODE_KEY, "AllowDevelopmentWithoutDevLicense"), reg_key_exists(CLASSIC_MENU_KEY));
        choose_tier(&TierInputs {
            packaged: self.o.packaged,
            os_build: self.o.os_build,
            developer_mode: developer_mode == Some(1),
            classic_menu_forced,
            package_files_present: self.o.package_dir.as_ref().is_some_and(|dir| dir.join("AppxManifest.xml").exists()),
        })
    }

    /// TODO(Phase 4): whether the sparse package is registered and its status is `Ok`.
    async fn package_registered(&self) -> bool {
        false
    }

    async fn read_status(&self) -> ExplorerMenuStatus {
        if !self.supported() {
            return ExplorerMenuStatus { supported: false, tier: None, available: ExplorerMenuTier::Classic, reason: None, detail: None };
        }
        let key = self.verb().key;
        let ((available, reason), has_package, has_classic) = tokio::join!(self.tier_choice(), self.package_registered(), is_classic_installed(&key));
        let tier = if has_package {
            Some(ExplorerMenuTier::Package)
        } else if has_classic {
            Some(ExplorerMenuTier::Classic)
        } else {
            None
        };
        ExplorerMenuStatus { supported: true, tier, available, reason, detail: self.last_error().clone() }
    }

    pub async fn status(&self) -> ExplorerMenuStatus {
        let _one_at_a_time = self.queue.lock().await;
        self.read_status().await
    }

    /// Registers the best tier this machine allows and removes the other, so there is one entry.
    async fn register(&self) -> AppResult<()> {
        *self.last_error() = None;
        let (tier, _) = self.tier_choice().await;
        if tier == ExplorerMenuTier::Package {
            // TODO(Phase 4): register the package (`ENSURE_SCRIPT`), remove the classic verb and
            // return; on failure keep the error in `last_error` and fall back to the classic verb.
            // Not reached in this build: it has no package files.
            log::warn!("explorer menu: the package tier is not in this build; using the classic verb");
        }
        install_classic(&self.verb()).await
    }

    pub async fn enable(&self) -> AppResult<ExplorerMenuStatus> {
        let _one_at_a_time = self.queue.lock().await;
        if self.supported() {
            self.register().await?;
        }
        Ok(self.read_status().await)
    }

    pub async fn disable(&self) -> AppResult<ExplorerMenuStatus> {
        let _one_at_a_time = self.queue.lock().await;
        if self.supported() {
            *self.last_error() = None;
            remove_classic(&self.verb().key).await;
            // TODO(Phase 4): a packaged app also removes its package (`REMOVE_SCRIPT`).
        }
        Ok(self.read_status().await)
    }

    /// Startup repair: points the entry at this install again after an update or a moved folder.
    pub async fn ensure(&self) -> AppResult<()> {
        let _one_at_a_time = self.queue.lock().await;
        if self.supported() && self.o.packaged {
            self.register().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> TierInputs {
        TierInputs { packaged: true, os_build: Some(26200), developer_mode: true, classic_menu_forced: false, package_files_present: true }
    }

    #[test]
    fn uses_the_package_when_everything_it_needs_is_there() {
        assert_eq!(choose_tier(&ok()), (ExplorerMenuTier::Package, None));
        assert_eq!(choose_tier(&TierInputs { os_build: Some(22000), ..ok() }), (ExplorerMenuTier::Package, None));
    }

    #[test]
    fn falls_back_to_the_classic_verbs_with_a_reason_otherwise() {
        for change in [
            TierInputs { packaged: false, ..ok() },
            TierInputs { os_build: Some(19045), ..ok() },
            TierInputs { os_build: None, ..ok() },
            TierInputs { developer_mode: false, ..ok() },
            TierInputs { classic_menu_forced: true, ..ok() },
            TierInputs { package_files_present: false, ..ok() },
        ] {
            let (tier, reason) = choose_tier(&change);
            assert_eq!(tier, ExplorerMenuTier::Classic, "{change:?}");
            assert!(reason.is_some_and(|r| !r.is_empty()), "{change:?}");
        }
    }

    #[test]
    fn gives_a_development_run_a_verb_of_its_own() {
        let menu = |packaged| {
            ExplorerMenu::new(ExplorerMenuOptions { packaged, os_build: None, exe_path: "C:\\mc.exe".into(), app_args: vec![], package_dir: None })
        };
        assert_eq!((menu(true).verb().key, menu(true).verb().title), ("MultiClaude".to_owned(), "Open in Multi Claude".to_owned()));
        assert_eq!((menu(false).verb().key, menu(false).verb().title), ("MultiClaudeDev".to_owned(), "Open in Multi Claude (Dev)".to_owned()));
    }
}
