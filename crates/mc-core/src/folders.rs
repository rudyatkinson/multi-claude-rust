//! `folder_key`, `pinned_account` (`src/shared/folders.ts`), and the lists of folders kept in
//! `config.json`: the recent folders and the folders pinned to an account (`src/main/index.ts`).

use std::collections::BTreeMap;

use crate::open_target::AccountRef;

/// The recent folders list keeps this many.
pub const RECENT_FOLDERS_MAX: usize = 12;

fn collapse(s: &str, is_sep: impl Fn(char) -> bool, sep: char) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_sep = false;
    for c in s.chars() {
        if is_sep(c) {
            if !prev_sep {
                out.push(sep);
            }
            prev_sep = true;
        } else {
            out.push(c);
            prev_sep = false;
        }
    }
    out
}

/// One spelling per folder, for looking a folder up: `C:\Code\App`, `c:/code/app/` and
/// `C:\code\app\` are the same folder on Windows. Works on text alone, without the file system.
pub fn folder_key(folder: &str, windows: bool) -> String {
    let mut key = if windows {
        collapse(folder, |c| c == '\\' || c == '/', '\\')
    } else {
        collapse(folder, |c| c == '/', '/')
    };
    // "C:\" and "/" keep their separator
    let b = key.as_bytes();
    let root = if windows { b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' } else { key == "/" };
    if !root && (key.ends_with('\\') || key.ends_with('/')) {
        key.pop();
    }
    if windows {
        key.to_lowercase()
    } else {
        key
    }
}

/// The account a folder is pinned to, if that account still exists. `accounts` maps folder keys
/// to profile ids.
pub fn pinned_account<P: AccountRef>(
    accounts: &BTreeMap<String, String>,
    folder: &str,
    windows: bool,
    profiles: &[P],
) -> Option<String> {
    let id = accounts.get(&folder_key(folder, windows))?;
    profiles.iter().any(|p| p.id() == id).then(|| id.clone())
}

/// The folder pins after pinning `folder` to `profile_id`, or removing its pin with None. Pins of
/// accounts that no longer exist (`exists` says no) go at the same time.
pub fn set_folder_pin(
    accounts: &BTreeMap<String, String>,
    folder: &str,
    windows: bool,
    profile_id: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> BTreeMap<String, String> {
    let mut next = accounts.clone();
    let key = folder_key(folder, windows);
    match profile_id {
        None => {
            next.remove(&key);
        }
        Some(id) => {
            next.insert(key, id.to_owned());
        }
    }
    next.retain(|_, id| exists(id));
    next
}

/// The recent folders with `cwd` first: no other spelling of it further down (letter case does
/// not count on Windows), at most [`RECENT_FOLDERS_MAX`].
pub fn remember_recent(recent: &[String], cwd: &str, windows: bool) -> Vec<String> {
    let same = |f: &str| if windows { f.to_lowercase() == cwd.to_lowercase() } else { f == cwd };
    std::iter::once(cwd.to_owned())
        .chain(recent.iter().filter(|f| !same(f)).cloned())
        .take(RECENT_FOLDERS_MAX)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_target::tests::acc;

    #[test]
    fn gives_every_spelling_of_a_windows_folder_the_same_key() {
        let key = folder_key(r"C:\Code\My App", true);
        assert_eq!(key, r"c:\code\my app");
        assert_eq!(folder_key("c:/code/my app/", true), key);
        assert_eq!(folder_key(r"C:\Code\\My App\", true), key);
        assert_eq!(folder_key(r"C:\Code/My App", true), key);
    }

    #[test]
    fn keeps_the_separator_of_a_drive_root() {
        assert_eq!(folder_key(r"C:\", true), r"c:\");
        assert_eq!(folder_key("D:/", true), r"d:\");
    }

    #[test]
    fn keeps_case_and_backslashes_on_other_systems() {
        assert_eq!(folder_key("/home/Me/Code/", false), "/home/Me/Code");
        assert_eq!(folder_key("/home//Me/Code", false), "/home/Me/Code");
        assert_eq!(folder_key("/", false), "/");
        assert_eq!(folder_key(r"/home/me/odd\name", false), r"/home/me/odd\name");
    }

    fn pins() -> BTreeMap<String, String> {
        BTreeMap::from([(folder_key(r"C:\Code\App", true), "work".to_owned())])
    }

    #[test]
    fn finds_the_account_a_folder_is_pinned_to_however_it_is_spelled() {
        let both = [acc("main", true), acc("work", true)];
        assert_eq!(pinned_account(&pins(), "c:/code/app/", true, &both).as_deref(), Some("work"));
        assert_eq!(pinned_account(&pins(), r"C:\Code\Other", true, &both), None);
        // a subfolder is its own folder
        assert_eq!(pinned_account(&pins(), r"C:\Code\App\src", true, &both), None);
    }

    #[test]
    fn ignores_the_pin_of_an_account_that_no_longer_exists() {
        assert_eq!(pinned_account(&pins(), r"C:\Code\App", true, &[acc("main", true)]), None);
    }

    #[test]
    fn pins_unpins_and_drops_pins_of_deleted_accounts() {
        let mut accounts = BTreeMap::from([("x:\\gone".to_owned(), "deleted".to_owned())]);
        let exists = |id: &str| id != "deleted";
        accounts = set_folder_pin(&accounts, r"C:\Code\App\", true, Some("work"), exists);
        assert_eq!(accounts, pins());
        accounts = set_folder_pin(&accounts, "c:/code/app", true, None, exists);
        assert!(accounts.is_empty());
    }

    #[test]
    fn recent_folders_are_newest_first_without_doubles() {
        let recent: Vec<String> = (0..12).map(|i| format!(r"C:\p{i}")).collect();
        let next = remember_recent(&recent, r"c:\P5", true);
        assert_eq!(next.len(), 12);
        assert_eq!(next[0], r"c:\P5");
        assert!(!next[1..].iter().any(|f| f.eq_ignore_ascii_case(r"c:\p5")));
        assert_eq!(next[11], r"C:\p11");

        let fresh = remember_recent(&recent, r"D:\new", true);
        assert_eq!(fresh.len(), 12);
        assert_eq!(fresh[0], r"D:\new");
        assert_eq!(fresh[11], r"C:\p10");

        // letter case counts elsewhere
        let unix = remember_recent(&["/a/B".to_owned()], "/a/b", false);
        assert_eq!(unix, vec!["/a/b".to_owned(), "/a/B".to_owned()]);
    }
}
