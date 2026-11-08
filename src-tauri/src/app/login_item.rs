//! The start with the system (`settings.openAtLogin`, `applyLoginItem` of `src/main/index.ts`),
//! which starts the app with `--hidden`.
//!
//! On Windows the app writes the entry itself: the value `Multi Claude` under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, the quoted executable and `--hidden`, as
//! Electron's `setLoginItemSettings` wrote it. The uninstaller (`nsis/hooks.nsh`) removes that
//! very value, and the one Task Manager keeps under `StartupApproved\Run`. On macOS the autostart
//! plugin registers a launch agent. Linux has no such entry.

use crate::state::AppState;

/// The name of the entry: the Run value on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub const NAME: &str = "Multi Claude";

/// Registers or removes the start with the system as `settings.openAtLogin` says. `asked`: the
/// user just turned it on or off; then a "disabled" mark that Task Manager keeps for the entry is
/// taken off as well, while the start of the app only writes the entry again (an update may have
/// moved the executable). Only the installed app has a stable path to register.
pub fn apply(state: &AppState, asked: bool) {
    if tauri::is_dev() || cfg!(target_os = "linux") {
        return;
    }
    let on = state.settings().open_at_login == Some(true);
    match set(state, on, asked) {
        Ok(()) => log::info!("start with the system: {}", if on { "on" } else { "off" }),
        Err(e) => log::warn!("start with the system: {e}"),
    }
}

#[cfg(windows)]
fn set(_state: &AppState, on: bool, asked: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    windows::write(&windows::Keys::user(), on.then_some(exe.as_path()), asked).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn set(state: &AppState, on: bool, _asked: bool) -> Result<(), String> {
    use tauri::Manager;
    let Some(autostart) = state.app.try_state::<tauri_plugin_autostart::AutoLaunchManager>() else {
        return Err("the autostart plugin is missing".to_owned());
    };
    if on { autostart.enable() } else { autostart.disable() }.map_err(|e| e.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn set(_state: &AppState, _on: bool, _asked: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::path::Path;

    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    use super::NAME;

    /// Where the entry and Task Manager's mark for it live.
    pub struct Keys {
        pub run: String,
        pub approved: String,
    }

    impl Keys {
        /// The current user's: the uninstaller removes the value from these keys.
        pub fn user() -> Self {
            Keys {
                run: r"Software\Microsoft\Windows\CurrentVersion\Run".to_owned(),
                approved: r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run".to_owned(),
            }
        }
    }

    /// The command line Windows runs at sign-in: the executable in quotes (its folder has a space
    /// in it), then `--hidden`.
    pub fn command_line(exe: &Path) -> String {
        format!("\"{}\" --hidden", exe.display())
    }

    /// A value that is not there is gone already.
    fn remove_value(key: &str, name: &str) -> io::Result<()> {
        let opened = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key, winreg::enums::KEY_SET_VALUE);
        let result = opened.and_then(|k| k.delete_value(name));
        match result {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    /// Writes the entry for `exe`, or removes it for None. `asked`: Task Manager's "disabled"
    /// mark goes too when the entry is written.
    pub fn write(keys: &Keys, exe: Option<&Path>, asked: bool) -> io::Result<()> {
        match exe {
            Some(exe) => {
                let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(&keys.run)?;
                run.set_value(NAME, &command_line(exe))?;
                if asked {
                    remove_value(&keys.approved, NAME)?;
                }
                Ok(())
            }
            None => {
                remove_value(&keys.run, NAME)?;
                remove_value(&keys.approved, NAME)
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn quotes_the_executable_and_starts_hidden() {
            let exe = Path::new(r"C:\Users\a b\AppData\Local\Multi Claude\Multi Claude.exe");
            assert_eq!(command_line(exe), r#""C:\Users\a b\AppData\Local\Multi Claude\Multi Claude.exe" --hidden"#);
        }

        #[test]
        fn writes_and_removes_the_entry_and_the_mark_of_task_manager() {
            let base = format!(r"Software\Multi Claude tests\login-item-{}", uuid::Uuid::new_v4());
            let keys = Keys { run: format!(r"{base}\Run"), approved: format!(r"{base}\Approved") };
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let (approved, _) = hkcu.create_subkey(&keys.approved).unwrap();
            approved.set_raw_value(NAME, &winreg::RegValue { bytes: vec![3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], vtype: winreg::enums::REG_BINARY }).unwrap();
            let exe = Path::new(r"C:\Apps\Multi Claude\Multi Claude.exe");
            let read = |key: &str| hkcu.open_subkey(key).ok().and_then(|k| k.get_raw_value(NAME).ok());

            // the start of the app writes the entry and leaves Task Manager's choice alone
            write(&keys, Some(exe), false).unwrap();
            let run: String = hkcu.open_subkey(&keys.run).unwrap().get_value(NAME).unwrap();
            assert_eq!(run, command_line(exe));
            assert!(read(&keys.approved).is_some());
            // turning it on in the app takes the "disabled" mark off
            write(&keys, Some(exe), true).unwrap();
            assert!(read(&keys.approved).is_none());
            // off: the entry goes, and going twice is fine
            write(&keys, None, true).unwrap();
            write(&keys, None, false).unwrap();
            assert!(read(&keys.run).is_none());

            hkcu.delete_subkey_all(&base).unwrap();
            let _ = hkcu.delete_subkey(r"Software\Multi Claude tests");
        }
    }
}
