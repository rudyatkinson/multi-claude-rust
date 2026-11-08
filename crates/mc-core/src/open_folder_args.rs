//! Startup arguments `--open-folder`, `--hidden`, `--quit` (`src/main/openFolder.ts`).
//!
//! The first element of `argv` is the program path and is never read as a switch.

const SWITCH: &str = "--open-folder";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartupArgs {
    /// The folder passed by the Explorer context menu, if any.
    pub open_folder: Option<String>,
    /// `--hidden`: start in the tray without showing the window (the login item uses it).
    pub hidden: bool,
    /// `--quit`: end the running instance instead of starting one (for scripts and installers).
    pub quit: bool,
}

/// Cleans up a folder argument. The classic context menu verb passes `"--open-folder=%V"`, which
/// for a drive root expands to `"--open-folder=C:\"`: the backslash escapes the closing quote, so
/// the value arrives as `C:"`.
pub fn normalize_folder_arg(raw: &str) -> Option<String> {
    let mut value = raw.trim().trim_matches('"').to_string();
    if is_bare_drive(&value) {
        value.push('\\');
    }
    (!value.is_empty() && is_absolute(&value)).then_some(value)
}

fn is_bare_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// `path.isAbsolute` as Node applies it on this platform: on Windows a drive (`C:\`), a rooted
/// path (`\foo`) or a UNC path; elsewhere a leading `/`.
fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    if cfg!(windows) {
        let sep = |c: u8| c == b'/' || c == b'\\';
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && sep(b[2])) || (!b.is_empty() && sep(b[0]))
    } else {
        b.first() == Some(&b'/')
    }
}

pub fn parse_startup_args<S: AsRef<str>>(argv: &[S]) -> StartupArgs {
    let flags: Vec<&str> = argv.iter().skip(1).map(|s| s.as_ref()).collect();
    StartupArgs {
        open_folder: parse_open_folder_arg(argv),
        hidden: flags.contains(&"--hidden"),
        quit: flags.contains(&"--quit"),
    }
}

/// The folder passed as `--open-folder=<dir>` or `--open-folder <dir>`, if any.
pub fn parse_open_folder_arg<S: AsRef<str>>(argv: &[S]) -> Option<String> {
    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_ref();
        if let Some(rest) = arg.strip_prefix(SWITCH) {
            if let Some(value) = rest.strip_prefix('=') {
                return normalize_folder_arg(value);
            }
            if rest.is_empty() && i + 1 < argv.len() {
                return normalize_folder_arg(argv[i + 1].as_ref());
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    mod windows_paths {
        use super::*;

        #[test]
        fn reads_the_switch_from_a_packaged_and_a_dev_command_line() {
            assert_eq!(parse_open_folder_arg(&["C:\\app\\Multi Claude.exe", "--open-folder=D:\\code\\my app"]).as_deref(), Some("D:\\code\\my app"));
            assert_eq!(parse_open_folder_arg(&["electron.exe", "C:\\src\\multi-claude", "--open-folder=D:\\code"]).as_deref(), Some("D:\\code"));
            assert_eq!(parse_open_folder_arg(&["app.exe", "--open-folder", "D:\\code"]).as_deref(), Some("D:\\code"));
        }

        #[test]
        fn keeps_turkish_and_special_characters() {
            assert_eq!(
                parse_open_folder_arg(&["app.exe", "--open-folder=C:\\Kullanıcılar\\Çağrı\\iş & 100% güç"]).as_deref(),
                Some("C:\\Kullanıcılar\\Çağrı\\iş & 100% güç")
            );
        }

        #[test]
        fn returns_none_without_the_switch_a_value_or_for_a_relative_path() {
            assert_eq!(parse_open_folder_arg(&["app.exe"]), None);
            assert_eq!(parse_open_folder_arg(&["app.exe", "--some-flag", "--open-folder"]), None);
            assert_eq!(parse_open_folder_arg(&["app.exe", "--open-folder=relative\\path"]), None);
            assert_eq!(parse_open_folder_arg(&["app.exe", "--open-folder="]), None);
        }

        #[test]
        fn does_not_treat_the_executable_path_as_an_argument() {
            assert_eq!(parse_open_folder_arg(&["--open-folder=C:\\x"]), None);
        }

        #[test]
        fn repairs_a_drive_root_mangled_by_the_classic_verb_quoting() {
            assert_eq!(normalize_folder_arg("C:\"").as_deref(), Some("C:\\"));
            assert_eq!(normalize_folder_arg("D:").as_deref(), Some("D:\\"));
            assert_eq!(normalize_folder_arg("C:\\").as_deref(), Some("C:\\"));
            assert_eq!(normalize_folder_arg("\"C:\\Program Files\"").as_deref(), Some("C:\\Program Files"));
        }

        #[test]
        fn still_finds_the_folder_from_the_explorer_menu() {
            assert_eq!(
                parse_startup_args(&["app.exe", "--open-folder=D:\\code", "--hidden"]),
                StartupArgs { open_folder: Some("D:\\code".into()), hidden: true, quit: false }
            );
        }
    }

    #[test]
    fn reads_the_switches_of_the_login_item_and_of_scripts() {
        assert_eq!(parse_startup_args(&["app.exe"]), StartupArgs::default());
        assert_eq!(parse_startup_args(&["app.exe", "--hidden"]), StartupArgs { hidden: true, ..Default::default() });
        assert_eq!(parse_startup_args(&["electron.exe", "C:\\src\\app", "--quit"]), StartupArgs { quit: true, ..Default::default() });
    }

    #[test]
    fn does_not_take_the_program_path_for_a_switch() {
        assert!(!parse_startup_args(&["--hidden"]).hidden);
    }
}
