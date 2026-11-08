//! Text files with their BOM and line endings, content hashes (`src/main/skills/files.ts`
//! decode/encode, retry and atomic write; `src/main/library/textFile.ts`).
//!
//! A Markdown file as the Library's editors open and save it: a SKILL.md, a subagent, a memory, a
//! CLAUDE.md. The text has plain line feeds; the line ends and the byte order mark of the file on
//! disk come back when it is written.

use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::types::{SaveOutcome, SKILL_LIMITS};

/// A file that was listed is not there any more.
pub const GONE: &str = "That file is no longer there. The list may be out of date.";
const TOO_LARGE: &str = "The file is too large to edit here.";
const UTF16: &str = "The file is stored as UTF-16; this editor reads UTF-8.";
const NOT_UTF8: &str = "The file is not valid UTF-8 text.";
const TEXT_TOO_LARGE: &str = "The text is too large to save here.";

/// How often [`retry_busy`] tries by default, and how long it waits before the second try (the
/// wait grows with each try).
pub const BUSY_TRIES: u32 = 3;
pub const BUSY_WAIT: Duration = Duration::from_millis(150);

/// The line ends a file is stored with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Eol {
    #[default]
    Lf,
    Crlf,
}

/// The text of a file as the editor works on it, and how it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The text with plain line feeds and without a byte order mark.
    pub text: String,
    pub eol: Eol,
    pub bom: bool,
}

/// Why a file is not opened for editing at all; the text is a sentence for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Unreadable(pub String);

/// `decodeSkill`: the text of the bytes, unless they are over `max_bytes`, UTF-16 or not strict
/// UTF-8. A UTF-8 byte order mark is taken off and noted; the line ends are the kind most lines
/// have (CRLF only when CRLF lines outnumber plain LF lines).
pub fn decode(bytes: &[u8], max_bytes: usize) -> Result<Decoded, Unreadable> {
    if bytes.len() > max_bytes {
        return Err(Unreadable(TOO_LARGE.to_owned()));
    }
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return Err(Unreadable(UTF16.to_owned()));
    }
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let raw = std::str::from_utf8(if bom { &bytes[3..] } else { bytes }).map_err(|_| Unreadable(NOT_UTF8.to_owned()))?;
    // TextDecoder drops a byte order mark at the start of what it is given too
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let crlf = raw.matches("\r\n").count();
    let lf = raw.matches('\n').count() - crlf;
    Ok(Decoded { text: raw.replace("\r\n", "\n"), eol: if crlf > lf { Eol::Crlf } else { Eol::Lf }, bom })
}

/// `encodeSkill`: the bytes for a text, with the line ends and the byte order mark asked for.
pub fn encode(text: &str, eol: Eol, bom: bool) -> Vec<u8> {
    let body = match eol {
        Eol::Lf => text.to_owned(),
        // /\r?\n/g -> \r\n
        Eol::Crlf => text.replace("\r\n", "\n").replace('\n', "\r\n"),
    };
    let mut out = Vec::with_capacity(body.len() + 3);
    if bom {
        out.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

/// Stands for the bytes on disk: their sha256, in hex.
pub fn content_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `TextOnDisk`: a file as an editor opens it. On the wire it is `{ text, hash, readOnly? }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRead {
    /// The whole text with plain line feeds. Empty when the file cannot be edited here.
    pub text: String,
    /// Stands for the bytes on disk; a save names the one it started from.
    pub hash: String,
    /// Why the file cannot be edited in the app, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<String>,
    /// How the file is stored; LF without a byte order mark when it cannot be read.
    #[serde(skip)]
    pub eol: Eol,
    #[serde(skip)]
    pub bom: bool,
}

/// What [`read_text`] gives for these bytes.
pub fn open_bytes(bytes: &[u8], max_bytes: usize) -> TextRead {
    let hash = content_hash(bytes);
    match decode(bytes, max_bytes) {
        Ok(d) => TextRead { text: d.text, hash, read_only: None, eol: d.eol, bom: d.bom },
        Err(Unreadable(why)) => TextRead { text: String::new(), hash, read_only: Some(why), eol: Eol::Lf, bom: false },
    }
}

fn read_existing(path: &Path) -> AppResult<Vec<u8>> {
    fs::read(path).map_err(|err| if err.kind() == ErrorKind::NotFound { AppError::io(GONE) } else { AppError::from(err) })
}

/// Opens a file for an editor. A file that is not there is [`GONE`]; one that cannot be edited
/// here comes with an empty text and the reason in `read_only`.
pub fn read_text(path: &Path, max_bytes: usize) -> AppResult<TextRead> {
    Ok(open_bytes(&read_existing(path)?, max_bytes))
}

/// Writes the file. `base_hash` is the state the editor started from: when the file on disk is
/// another one by now (a Claude session changed it), nothing is written and the result is
/// [`SaveOutcome::Conflict`] with what is on disk, unless `base_hash` is `None`, which overwrites.
/// The line ends and byte order mark stay as the file had them (LF without one when it could not
/// be read). A file that is gone is never brought back. The limit is [`SKILL_LIMITS`]`.file_bytes`,
/// as in the Electron version.
pub fn save_text(path: &Path, text: &str, base_hash: Option<&str>) -> AppResult<SaveOutcome<TextRead>> {
    let max = SKILL_LIMITS.file_bytes;
    if text.len() > max {
        return Err(AppError::invalid(TEXT_TOO_LARGE));
    }
    let bytes = read_existing(path)?;
    if base_hash.is_some_and(|base| content_hash(&bytes) != base) {
        return Ok(SaveOutcome::Conflict(open_bytes(&bytes, max)));
    }
    let (eol, bom) = decode(&bytes, max).map_or((Eol::Lf, false), |d| (d.eol, d.bom));
    let next = encode(text, eol, bom);
    write_file_atomic(path, &next)?;
    Ok(SaveOutcome::Saved(open_bytes(&next, max)))
}

/// Makes a file that must not be there yet, with the folders above it.
pub fn create_text(path: &Path, content: impl AsRef<[u8]>) -> AppResult<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|err| {
        if err.kind() == ErrorKind::AlreadyExists {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            AppError::refused(format!("There is already a file named \"{name}\" there."))
        } else {
            AppError::from(err)
        }
    })?;
    file.write_all(content.as_ref())?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(format!(".tmp-{}", std::process::id()));
    PathBuf::from(name)
}

/// Writes a file so that a reader sees the old content or the new one, never half of it: into
/// `<file>.tmp-<pid>`, then renamed over the file (tried again while Windows says it is busy).
/// When something keeps the old file open so that it cannot be replaced, the bytes are written
/// into it instead.
pub fn write_file_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);
    fs::write(&tmp, bytes)?;
    if retry_busy(|| fs::rename(&tmp, path), BUSY_TRIES, BUSY_WAIT).is_err() {
        fs::write(path, bytes)?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

/// Windows (or a Unix) saying a file or folder is in use: `EBUSY`, `EPERM`, `EACCES` and
/// `ENOTEMPTY` as Node names them, which on Windows include a sharing or lock violation.
pub fn is_busy(err: &io::Error) -> bool {
    if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::ResourceBusy | ErrorKind::DirectoryNotEmpty) {
        return true;
    }
    #[cfg(windows)]
    {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_PATH_BUSY, ERROR_BUSY
        if matches!(err.raw_os_error(), Some(32 | 33 | 148 | 170)) {
            return true;
        }
    }
    false
}

/// Tries again when Windows says a file or folder is in use: up to `tries` times in all, waiting
/// `wait` times the number of tries so far in between. A running Claude session watches the
/// skills folders, and a sync client may hold a file for a moment. Any other error ends it at once.
pub fn retry_busy<T>(mut work: impl FnMut() -> io::Result<T>, tries: u32, wait: Duration) -> io::Result<T> {
    let mut attempt = 1u32;
    loop {
        match work() {
            Ok(value) => return Ok(value),
            Err(err) if attempt < tries && is_busy(&err) => {
                thread::sleep(wait * attempt);
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// What a busy error means to the user. Other errors keep their own message.
pub fn busy_message(err: io::Error, what: &str) -> AppError {
    if is_busy(&err) {
        AppError::io(format!("{what} did not work: Windows says the folder is in use. A Claude session that is working there may hold it; try again when it rests."))
    } else {
        AppError::from(err)
    }
}

#[cfg(test)]
mod tests {
    //! `test/textFile.test.ts` (readText, saveText, createText) and the decode, encode and
    //! retryBusy tests of `test/skillFiles.test.ts`.

    use super::*;

    const MAX: usize = SKILL_LIMITS.file_bytes;

    // ---------- the text on disk ----------

    #[test]
    fn reads_line_ends_and_a_byte_order_mark_and_writes_them_back() {
        let crlf = "\u{feff}---\r\nname: x\r\n---\r\nbody\r\n".as_bytes();
        let decoded = decode(crlf, MAX).unwrap();
        assert_eq!(decoded, Decoded { text: "---\nname: x\n---\nbody\n".into(), eol: Eol::Crlf, bom: true });
        assert_eq!(encode(&decoded.text, decoded.eol, decoded.bom), crlf);

        let lf = b"a\nb\n";
        assert_eq!(decode(lf, MAX).unwrap(), Decoded { text: "a\nb\n".into(), eol: Eol::Lf, bom: false });
        assert_eq!(encode("a\nb\n", Eol::Lf, false), lf);
    }

    #[test]
    fn goes_by_the_line_end_most_lines_have_and_makes_them_one_kind_when_writing() {
        let mixed = decode(b"a\r\nb\r\nc\n", MAX).unwrap();
        assert_eq!((mixed.text.as_str(), mixed.eol), ("a\nb\nc\n", Eol::Crlf));
        assert_eq!(encode("a\nb\nc\n", Eol::Crlf, false), b"a\r\nb\r\nc\r\n");
        // as many of each: LF
        assert_eq!(decode(b"a\r\nb\n", MAX).unwrap().eol, Eol::Lf);
        // a CRLF already in the text is not doubled
        assert_eq!(encode("a\r\nb", Eol::Crlf, false), b"a\r\nb");
    }

    #[test]
    fn does_not_open_what_it_could_not_write_back_faithfully() {
        assert_eq!(decode(&[0xff, 0xfe, 0x61, 0x00], MAX), Err(Unreadable("The file is stored as UTF-16; this editor reads UTF-8.".into())));
        assert_eq!(decode(&[0x61, 0xc3, 0x28], MAX), Err(Unreadable("The file is not valid UTF-8 text.".into())));
        assert_eq!(decode(&vec![0x61; 1_000_001], MAX), Err(Unreadable("The file is too large to edit here.".into())));
        assert!(decode(&vec![0x61; 1_000_000], MAX).is_ok());
    }

    #[test]
    fn hashes_the_bytes() {
        assert_eq!(content_hash(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(content_hash(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    // ---------- retryBusy ----------

    fn busy() -> io::Error {
        io::Error::new(ErrorKind::ResourceBusy, "in use")
    }

    #[test]
    fn tries_again_while_windows_says_the_folder_is_in_use() {
        let mut calls = 0;
        let result = retry_busy(
            || {
                calls += 1;
                if calls < 3 {
                    Err(busy())
                } else {
                    Ok("done")
                }
            },
            3,
            Duration::from_millis(1),
        );
        assert_eq!((result.unwrap(), calls), ("done", 3));
    }

    #[test]
    fn gives_up_after_the_last_try_and_at_once_for_any_other_error() {
        let mut calls = 0;
        let err = retry_busy::<()>(
            || {
                calls += 1;
                Err(busy())
            },
            2,
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert_eq!((err.to_string(), calls), ("in use".to_owned(), 2));
        calls = 0;
        let err = retry_busy::<()>(
            || {
                calls += 1;
                Err(io::Error::other("other"))
            },
            3,
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert_eq!((err.to_string(), calls), ("other".to_owned(), 1));
    }

    #[test]
    fn names_a_busy_folder_and_keeps_other_messages() {
        assert!(is_busy(&io::Error::from(ErrorKind::PermissionDenied)));
        assert!(is_busy(&io::Error::from(ErrorKind::DirectoryNotEmpty)));
        assert!(!is_busy(&io::Error::from(ErrorKind::NotFound)));
        assert!(busy_message(busy(), "Renaming").to_string().starts_with("Renaming did not work: Windows says the folder is in use."));
        assert_eq!(busy_message(io::Error::other("other"), "Renaming").to_string(), "other");
    }

    // ---------- a text file of the Library ----------

    #[test]
    fn saves_over_the_state_it_was_read_in_and_keeps_how_the_file_was_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.md");
        fs::write(&file, "\u{feff}one\r\ntwo\r\n").unwrap();
        let read = read_text(&file, MAX).unwrap();
        assert_eq!(read.text, "one\ntwo\n");

        let result = save_text(&file, "one\nthree\n", Some(&read.hash)).unwrap();
        assert_eq!(fs::read(&file).unwrap(), "\u{feff}one\r\nthree\r\n".as_bytes());
        assert_eq!(result, SaveOutcome::Saved(read_text(&file, MAX).unwrap()));
        let SaveOutcome::Saved(saved) = result else { unreachable!() };
        assert_eq!((saved.text.as_str(), saved.eol, saved.bom), ("one\nthree\n", Eol::Crlf, true));
        // no temporary file is left next to it
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn does_not_overwrite_a_file_that_changed_since_it_was_read_unless_told_to() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.md");
        fs::write(&file, "mine\n").unwrap();
        let read = read_text(&file, MAX).unwrap();
        fs::write(&file, "theirs\n").unwrap();

        let refused = save_text(&file, "mine, changed\n", Some(&read.hash)).unwrap();
        assert_eq!(refused, SaveOutcome::Conflict(read_text(&file, MAX).unwrap()));
        assert_eq!(fs::read_to_string(&file).unwrap(), "theirs\n");

        save_text(&file, "mine, changed\n", None).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "mine, changed\n");
    }

    #[test]
    fn shows_a_file_it_cannot_edit_and_says_why() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.md");
        fs::write(&file, [0xff, 0xfe, 0x61, 0x00]).unwrap();
        let read = read_text(&file, MAX).unwrap();
        assert_eq!((read.text.as_str(), read.read_only.as_deref()), ("", Some("The file is stored as UTF-16; this editor reads UTF-8.")));
        assert_eq!(serde_json::to_value(&read).unwrap()["readOnly"], "The file is stored as UTF-16; this editor reads UTF-8.");
        // saving over it writes plain UTF-8 with line feeds
        let SaveOutcome::Saved(saved) = save_text(&file, "a\nb\n", None).unwrap() else { panic!("conflict") };
        assert_eq!((fs::read(&file).unwrap(), saved.read_only), (b"a\nb\n".to_vec(), None));
    }

    #[test]
    fn says_so_when_the_file_is_gone_and_never_brings_it_back_by_saving() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("gone.md");
        assert_eq!(read_text(&file, MAX).unwrap_err().to_string(), GONE);
        assert_eq!(save_text(&file, "text\n", None).unwrap_err().to_string(), GONE);
        assert!(!file.exists());
    }

    #[test]
    fn makes_a_new_file_with_its_folders_and_never_over_an_existing_one() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("rules").join("deep").join("style.md");
        create_text(&file, "first\n").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "first\n");
        assert_eq!(create_text(&file, "second\n").unwrap_err().to_string(), "There is already a file named \"style.md\" there.");
        assert_eq!(fs::read_to_string(&file).unwrap(), "first\n");
    }

    #[test]
    fn refuses_a_text_too_large_to_save() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.md");
        fs::write(&file, "a\n").unwrap();
        assert_eq!(save_text(&file, &"x".repeat(MAX + 1), None).unwrap_err().to_string(), "The text is too large to save here.");
        assert_eq!(fs::read_to_string(&file).unwrap(), "a\n");
    }

    #[test]
    fn writes_atomically_over_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.md");
        fs::write(&file, "old").unwrap();
        write_file_atomic(&file, b"new").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"new");
        assert!(!tmp_path(&file).exists());
    }
}
