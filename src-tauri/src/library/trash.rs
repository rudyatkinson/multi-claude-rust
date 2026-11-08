//! Moving what the Library deletes into the system's bin (`shell.trashItem` and the `remove` of
//! each Library service in the Electron version). Nothing is ever deleted for good instead: a
//! skill or a memory may be hours of work.

use std::io::{self, ErrorKind};
use std::path::Path;
use std::sync::Arc;

use mc_core::error::{AppError, AppResult};
use mc_core::text_file::{busy_message, is_busy, retry_busy, BUSY_TRIES, BUSY_WAIT};

use crate::sharing::{is_link, remove_link};

/// Moves a file or folder into the bin. Services keep one so that tests can put their own in.
pub type TrashFn = Arc<dyn Fn(&Path) -> io::Result<()> + Send + Sync>;

/// The system's bin, as a [`TrashFn`].
pub fn system_trash_fn() -> TrashFn {
    Arc::new(system_trash)
}

/// Moves `path` into the system's bin with the `trash` crate. A file or folder that Windows says
/// is in use comes back as an error [`is_busy`] knows, so that [`retry_busy`] tries again.
///
/// On a thread of its own: on Windows the bin is reached through COM, and the `trash` crate panics
/// (which aborts a release build) on a thread that other code, a file dialog for one, set up for
/// COM another way.
pub fn system_trash(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    std::thread::spawn(move || trash::delete(&path).map_err(to_io))
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("the system's bin could not be reached")))
}

/// The words for an error of the `trash` crate, without its type names.
fn describe(err: &trash::Error) -> String {
    match err {
        trash::Error::Unknown { description } | trash::Error::Os { description, .. } => description.clone(),
        trash::Error::TargetedRoot => "it is the top folder of a drive".to_owned(),
        trash::Error::CouldNotAccess { target } => format!("{target} cannot be reached"),
        trash::Error::CanonicalizePath { original } => format!("{} cannot be resolved", original.display()),
        other => other.to_string(),
    }
}

/// Whether an HRESULT of the shell says the item is in use, or something wrote into the folder
/// while it was moved (Node's `ENOTEMPTY`).
#[cfg(windows)]
fn busy_hresult(code: i32) -> bool {
    const COPYENGINE_E_SHARING_VIOLATION_SRC: u32 = 0x8027_0027;
    const COPYENGINE_E_SHARING_VIOLATION_DEST: u32 = 0x8027_0028;
    const COPYENGINE_E_DIR_NOT_EMPTY: u32 = 0x8027_003D;
    let code = code as u32;
    // HRESULT_FROM_WIN32 of ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_DIR_NOT_EMPTY,
    // ERROR_PATH_BUSY, ERROR_BUSY
    let win32 = (code & 0xFFFF_0000 == 0x8007_0000).then_some(code & 0xFFFF);
    matches!(code, COPYENGINE_E_SHARING_VIOLATION_SRC | COPYENGINE_E_SHARING_VIOLATION_DEST | COPYENGINE_E_DIR_NOT_EMPTY)
        || matches!(win32, Some(32 | 33 | 145 | 148 | 170))
}

fn to_io(err: trash::Error) -> io::Error {
    let message = describe(&err);
    #[cfg(windows)]
    if let trash::Error::Os { code, .. } = &err {
        if busy_hresult(*code) {
            return io::Error::new(ErrorKind::ResourceBusy, message);
        }
    }
    if matches!(err, trash::Error::CouldNotAccess { .. }) {
        return io::Error::new(ErrorKind::NotFound, message);
    }
    io::Error::other(message)
}

/// Moves a file or folder into the bin with `trash`, trying again while Windows says it is in
/// use. `what` names it for the user ("skill", "agent", "memory", "file"): when there is no bin
/// for that drive the error says that it was not deleted.
pub fn trash_with(trash: &TrashFn, path: &Path, what: &str) -> AppResult<()> {
    match retry_busy(|| trash(path), BUSY_TRIES, BUSY_WAIT) {
        Ok(()) => Ok(()),
        Err(err) if is_busy(&err) => Err(busy_message(err, "Deleting")),
        Err(err) => Err(AppError::io(format!("The {what} could not be moved to the bin ({err}). It was not deleted."))),
    }
}

/// [`trash_with`] the system's bin.
pub fn trash_item(path: &Path, what: &str) -> AppResult<()> {
    trash_with(&system_trash_fn(), path, what)
}

/// Deletes what the Library lists: a link (a junction, a symlink) loses only the link and never
/// what it points at; anything else goes into the bin with [`trash_with`].
pub fn delete_item(trash: &TrashFn, path: &Path, what: &str) -> AppResult<()> {
    if is_link(path) {
        return remove_link(path).map_err(AppError::from);
    }
    trash_with(trash, path, what)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Mutex;

    use super::*;
    use crate::library::test_support::link;

    #[test]
    fn says_it_was_not_deleted_when_there_is_no_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.md");
        fs::write(&file, "x").unwrap();
        let no_bin: TrashFn = Arc::new(|_| Err(io::Error::other("no bin on this drive")));
        let err = trash_with(&no_bin, &file, "memory").unwrap_err().to_string();
        assert_eq!(err, "The memory could not be moved to the bin (no bin on this drive). It was not deleted.");
        assert!(file.exists());
    }

    #[test]
    fn tries_again_while_the_folder_is_in_use_and_then_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(0));
        let counted = calls.clone();
        let busy: TrashFn = Arc::new(move |_| {
            *counted.lock().unwrap() += 1;
            Err(io::Error::new(ErrorKind::ResourceBusy, "in use"))
        });
        let err = trash_with(&busy, tmp.path(), "skill").unwrap_err().to_string();
        assert!(err.starts_with("Deleting did not work: Windows says the folder is in use."), "{err}");
        assert_eq!(*calls.lock().unwrap(), BUSY_TRIES);
    }

    #[test]
    fn removes_only_the_link_of_a_folder_that_lives_elsewhere() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "x").unwrap();
        let at = tmp.path().join("linked");
        link(&target, &at);
        let trashed = Arc::new(Mutex::new(Vec::new()));
        let seen = trashed.clone();
        let trash: TrashFn = Arc::new(move |p| {
            seen.lock().unwrap().push(p.to_path_buf());
            Ok(())
        });
        delete_item(&trash, &at, "skill").unwrap();
        assert!(!at.exists() && !is_link(&at));
        assert!(target.join("SKILL.md").exists());
        assert!(trashed.lock().unwrap().is_empty());
        // anything else goes to the bin
        delete_item(&trash, &target, "skill").unwrap();
        assert_eq!(*trashed.lock().unwrap(), [target]);
    }

    #[test]
    fn moves_a_file_into_the_bin_and_says_when_there_is_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        // nothing is there: the crate's error, worded, and not taken for a busy folder
        let err = system_trash(&tmp.path().join("missing.md")).unwrap_err();
        assert!(!is_busy(&err), "{err}");
    }

    #[test]
    fn words_the_errors_of_the_trash_crate() {
        assert_eq!(describe(&trash::Error::Unknown { description: "boom".into() }), "boom");
        assert_eq!(to_io(trash::Error::CouldNotAccess { target: "x".into() }).kind(), ErrorKind::NotFound);
        #[cfg(windows)]
        {
            assert!(is_busy(&to_io(trash::Error::Os { code: 0x8007_0020_u32 as i32, description: "in use".into() })));
            assert!(is_busy(&to_io(trash::Error::Os { code: 0x8027_0027_u32 as i32, description: "in use".into() })));
            assert!(is_busy(&to_io(trash::Error::Os { code: 0x8027_003D_u32 as i32, description: "not empty".into() })));
            let denied = to_io(trash::Error::Os { code: 0x8007_0005_u32 as i32, description: "windows error: denied".into() });
            assert!(!is_busy(&denied));
            assert_eq!(denied.to_string(), "windows error: denied");
        }
    }
}
