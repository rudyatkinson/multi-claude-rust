//! Where Git Bash is (`tapSupported` of `src/main/usage/statuslineTap.ts`). The status line tap is
//! a POSIX shell command. On Windows Claude Code runs status lines through Git Bash when it is
//! installed and through PowerShell otherwise, where the tap would not work.

use std::path::PathBuf;

/// The bash that Claude Code would run status lines with on Windows, by the same search it makes.
/// Elsewhere status lines run in a POSIX shell anyway, so this is always `/bin/sh` there.
pub fn git_bash_path() -> Option<PathBuf> {
    if !cfg!(windows) {
        return Some(PathBuf::from("/bin/sh"));
    }
    candidates().into_iter().find(|candidate| candidate.is_file())
}

fn candidates() -> Vec<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let mut list = Vec::new();
    list.extend(env("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from));
    list.push(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"));
    list.push(PathBuf::from(r"C:\Program Files (x86)\Git\bin\bash.exe"));
    list.extend(env("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Programs").join("Git").join("bin").join("bash.exe")));
    if let Some(path) = env("PATH") {
        for dir in std::env::split_paths(&path).filter(|d| !d.as_os_str().is_empty()) {
            if dir.join("git.exe").is_file() {
                list.push(dir.join("..").join("bin").join("bash.exe"));
            }
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_shell_where_one_is_installed() {
        if cfg!(windows) {
            let standard = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
            if standard.is_file() && std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").is_none() {
                assert_eq!(git_bash_path(), Some(standard));
            }
        } else {
            assert_eq!(git_bash_path(), Some(PathBuf::from("/bin/sh")));
        }
    }
}
