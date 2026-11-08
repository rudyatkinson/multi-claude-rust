//! Reading conversation files: `summarize` and `ConversationIndex` of `src/main/conversations.ts`.
//!
//! The Electron version read six files at a time; here a scan reads them one after the other.
//! Running scans off the UI thread, or in parallel, is up to the caller.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::parse::{parse_head, parse_tail, Head, Tail};
use crate::jsstr::cut16;
use crate::types::ConversationEntry;

/// How much of the beginning of a file is read for its folder and first message.
pub const HEAD_BYTES: u64 = 128 * 1024;
/// How far into a file the first message is looked for when it is not near the start.
pub const HEAD_MAX_BYTES: u64 = 512 * 1024;
/// How much of the end of a file is read for its title and last message.
pub const TAIL_BYTES: u64 = 256 * 1024;
/// An untitled conversation is named after this many UTF-16 units of a message.
pub const TITLE_CHARS: usize = 80;

/// What [`scan_profile`] read of each file: its time and size when it was read, and what it gave.
pub type ScanCache = HashMap<PathBuf, (SystemTime, u64, Option<ConversationEntry>)>;

/// A UUID, as Claude Code names conversations; nothing else is used in a path.
pub(crate) fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// `<uuid>.jsonl` (in any case, as the `/i` pattern took it): the id as written.
fn conversation_id(name: &str) -> Option<&str> {
    let (id, ext) = name.split_at_checked(36)?;
    (is_uuid(id) && ext.eq_ignore_ascii_case(".jsonl")).then_some(id)
}

/// `length` bytes from `start`, or fewer at the end of the file.
fn read_at(file: &mut File, start: u64, length: u64) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(start))?;
    let mut buffer = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    file.by_ref().take(length).read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn head_of(file: &mut File, size: u64) -> io::Result<Head> {
    let head = parse_head(&read_at(file, 0, size.min(HEAD_BYTES))?, size > HEAD_BYTES);
    if head.first_prompt.is_some() || size <= HEAD_BYTES {
        return Ok(head);
    }
    // a conversation that began with a slash command or a pasted document
    let more = parse_head(&read_at(file, 0, size.min(HEAD_MAX_BYTES))?, size > HEAD_MAX_BYTES);
    Ok(Head { cwd: head.cwd.or(more.cwd), first_prompt: more.first_prompt })
}

fn tail_of(file: &mut File, size: u64) -> io::Result<Tail> {
    let start = size.saturating_sub(TAIL_BYTES);
    let bytes = read_at(file, start, size - start)?;
    // from the first whole line on; cutting at the byte keeps a split character out of the text
    let from = if start > 0 { bytes.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1) } else { 0 };
    Ok(parse_tail(&bytes[from..]))
}

/// The folder and the first typed message of a conversation file: the first 128 KiB, or up to
/// 512 KiB when those hold no typed message.
pub fn read_head(path: &Path) -> io::Result<Head> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    head_of(&mut file, size)
}

/// The title, the last message and the folder of a conversation file, from its last 256 KiB.
pub fn read_tail(path: &Path) -> io::Result<Tail> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    tail_of(&mut file, size)
}

/// What a list shows of one conversation. `None` for a file in which nothing was ever typed, or
/// that names no folder. The beginning names the folder the conversation started in; the end is
/// the fallback. The title is Claude Code's, else the start of a message, else the folder's name.
pub fn entry_of(profile_id: &str, id: &str, head: Head, tail: Tail, updated_at: i64) -> Option<ConversationEntry> {
    let cwd = head.cwd.or(tail.cwd).filter(|cwd| !cwd.is_empty())?;
    let prompt = head.first_prompt.as_deref().or(tail.last_prompt.as_deref());
    if prompt.or(tail.title.as_deref()).is_none_or(str::is_empty) {
        return None;
    }
    let title = match (&tail.title, prompt) {
        (Some(title), _) => title.clone(),
        (None, Some(prompt)) => cut16(prompt, TITLE_CHARS).to_string(),
        (None, None) => Path::new(&cwd).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    Some(ConversationEntry {
        profile_id: profile_id.to_string(),
        id: id.to_string(),
        cwd,
        title,
        first_prompt: head.first_prompt,
        last_prompt: tail.last_prompt,
        updated_at,
    })
}

/// Reads one file, with the size its stat gave.
fn summarize(path: &Path, size: u64, profile_id: &str, id: &str, updated_at: i64) -> io::Result<Option<ConversationEntry>> {
    let mut file = File::open(path)?;
    let head = head_of(&mut file, size)?;
    let tail = tail_of(&mut file, size)?;
    Ok(entry_of(profile_id, id, head, tail, updated_at))
}

fn epoch_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The names in a folder, sorted as Node's `readdir` returns them on Unix; `None` when it cannot
/// be read (missing, or a file).
fn names(dir: &Path) -> Option<Vec<OsString>> {
    let mut names: Vec<OsString> = fs::read_dir(dir).ok()?.filter_map(|entry| entry.ok().map(|e| e.file_name())).collect();
    names.sort();
    Some(names)
}

/// The conversation files of a `projects` folder: `(id, path)`.
fn conversation_files(projects: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    for key in names(projects).unwrap_or_default() {
        let folder = projects.join(key);
        for name in names(&folder).unwrap_or_default() {
            if let Some(id) = name.to_str().and_then(conversation_id) {
                files.push((id.to_string(), folder.join(&name)));
            }
        }
    }
    files
}

/// One file, from the cache when its time and size are the same. `None` when it gave nothing, or
/// was deleted or locked in the meantime (then it is not listed this time and its old reading stays).
fn load(path: &Path, profile_id: &str, id: &str, cache: &mut ScanCache) -> Option<ConversationEntry> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
    let size = meta.len();
    let fresh = matches!(cache.get(path), Some((time, len, _)) if *time == mtime && *len == size);
    if !fresh {
        let entry = summarize(path, size, profile_id, id, epoch_ms(mtime)).ok()?;
        cache.insert(path.to_path_buf(), (mtime, size, entry));
    }
    let mut entry = cache.get(path)?.2.clone()?;
    entry.profile_id = profile_id.to_string();
    Some(entry)
}

/// The conversations of one account: every `projects/<key>/<uuid>.jsonl` of its config dir, in
/// the order of the folder names. A file is read again only when its size or time changed. What
/// the cache holds of files under this `projects` folder that are gone is dropped.
pub fn scan_profile(config_dir: &Path, profile_id: &str, cache: &mut ScanCache) -> Vec<ConversationEntry> {
    let projects = config_dir.join("projects");
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for (id, path) in conversation_files(&projects) {
        if let Some(entry) = load(&path, profile_id, &id, cache) {
            entries.push(entry);
        }
        seen.insert(path);
    }
    cache.retain(|path, _| seen.contains(path) || !path.starts_with(&projects));
    entries
}

/// The conversations of all accounts, with what was read kept in memory (not on disk).
#[derive(Debug, Default)]
pub struct ConversationIndex {
    cache: ScanCache,
    entries: Vec<ConversationEntry>,
}

impl ConversationIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lists the conversations of these accounts, `(profile id, config dir)`. Forgets what it read
    /// of files that are no longer there and of accounts that are not in the list.
    pub fn list<S: AsRef<str>, P: AsRef<Path>>(&mut self, profiles: &[(S, P)]) -> &[ConversationEntry] {
        let mut entries = Vec::new();
        for (id, dir) in profiles {
            entries.extend(scan_profile(dir.as_ref(), id.as_ref(), &mut self.cache));
        }
        let roots: Vec<PathBuf> = profiles.iter().map(|(_, dir)| dir.as_ref().join("projects")).collect();
        self.cache.retain(|path, _| roots.iter().any(|root| path.starts_with(root)));
        self.entries = entries;
        &self.entries
    }

    /// An index with what this one read of the files and no listing, to list with while this one
    /// still answers [`find`](Self::find) from its last listing; put it in this one's place after.
    pub fn take_reading(&mut self) -> ConversationIndex {
        ConversationIndex { cache: std::mem::take(&mut self.cache), entries: Vec::new() }
    }

    /// A conversation from the last listing.
    pub fn find(&self, profile_id: &str, id: &str) -> Option<&ConversationEntry> {
        self.entries.iter().find(|e| e.profile_id == profile_id && e.id == id)
    }

    /// The last listing.
    pub fn entries(&self) -> &[ConversationEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse::tests::{js, title, user, user_rec, APP, ID};
    use super::*;

    fn write(dir: &Path, profile: &str, key: &str, id: &str, lines: &[String]) -> PathBuf {
        let file = dir.join(profile).join("projects").join(key).join(format!("{id}.jsonl"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, lines.join("\n") + "\n").unwrap();
        file
    }

    fn summary(file: &Path) -> Option<ConversationEntry> {
        entry_of("main", ID, read_head(file).unwrap(), read_tail(file).unwrap(), 0)
    }

    fn assistant(text: &str) -> String {
        format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":{}}}]}}}}"#, js(text))
    }

    #[test]
    fn knows_a_conversation_file_by_its_name() {
        assert_eq!(conversation_id(&format!("{ID}.jsonl")), Some(ID));
        assert_eq!(conversation_id(&format!("{}.JSONL", ID.to_uppercase())), Some(ID.to_uppercase().as_str()));
        assert_eq!(conversation_id("notes.txt"), None);
        assert_eq!(conversation_id(&format!("{ID}.jsonl.bak")), None);
        assert_eq!(conversation_id("2bf04019-6df2-46c6-93d8-91c02cde383ç.jsonl"), None);
    }

    // ---------- reading conversation files (test/conversations.test.ts) ----------

    #[test]
    fn summarizes_a_small_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "main", "C--code-app", ID, &[user("Fix the login bug"), title("Login bug"), r#"{"type":"last-prompt","lastPrompt":"thanks"}"#.into()]);
        let entry = summary(&file).unwrap();
        assert_eq!(
            (entry.cwd.as_str(), entry.title.as_str(), entry.first_prompt.as_deref(), entry.last_prompt.as_deref()),
            (APP, "Login bug", Some("Fix the login bug"), Some("thanks"))
        );
    }

    #[test]
    fn reads_only_the_ends_of_a_large_file_and_still_finds_title_and_first_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = vec![user("Refactor the parser")];
        lines.extend((0..400).map(|i| assistant(&format!("step {i} {}", "x".repeat(4000)))));
        lines.push(title("Parser refactor"));
        let file = write(dir.path(), "main", "C--code-app", ID, &lines);
        assert!(fs::metadata(&file).unwrap().len() > 1024 * 1024);
        let entry = summary(&file).unwrap();
        assert_eq!((entry.title.as_str(), entry.first_prompt.as_deref()), ("Parser refactor", Some("Refactor the parser")));
    }

    #[test]
    fn looks_further_in_when_the_first_message_is_not_near_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let caveat = js(&format!("<local-command-stdout>{}</local-command-stdout>", "y".repeat(4000)));
        let mut lines: Vec<String> = (0..50).map(|_| user_rec(&caveat, false, APP, true)).collect();
        lines.push(user("The actual question"));
        lines.push(r#"{"type":"assistant","message":{"content":[]}}"#.into());
        let file = write(dir.path(), "main", "C--code-app", ID, &lines);
        assert!(fs::metadata(&file).unwrap().len() > HEAD_BYTES);
        assert_eq!(summary(&file).unwrap().first_prompt.as_deref(), Some("The actual question"));
    }

    #[test]
    fn names_an_untitled_conversation_after_its_first_message_and_skips_one_in_which_nothing_was_typed() {
        let dir = tempfile::tempdir().unwrap();
        let untitled = write(dir.path(), "main", "C--code-app", ID, &[user(&format!("How do I {}do this?", "really ".repeat(40)))]);
        let entry = summary(&untitled).unwrap();
        assert_eq!(entry.title.len(), 80);
        assert!(entry.title.starts_with("How do I really"));
        let empty = write(
            dir.path(),
            "main",
            "C--code-app",
            "00000000-0000-4000-8000-000000000000",
            &[r#"{"type":"mode","mode":"default"}"#.into(), format!(r#"{{"type":"system","cwd":{}}}"#, js(APP))],
        );
        assert_eq!(summary(&empty), None);
    }

    #[test]
    fn falls_back_to_the_end_for_the_folder_and_names_a_conversation_after_its_last_message() {
        let head = Head { cwd: None, first_prompt: None };
        let tail = Tail { title: None, last_prompt: Some("go on".into()), cwd: Some(r"D:\x".into()) };
        let entry = entry_of("p", "i", head, tail, 7).unwrap();
        assert_eq!((entry.cwd.as_str(), entry.title.as_str(), entry.updated_at), (r"D:\x", "go on", 7));
        // no folder at all
        assert_eq!(entry_of("p", "i", Head::default(), Tail { title: Some("t".into()), ..Tail::default() }, 0), None);
    }

    #[test]
    fn lists_the_conversations_of_every_account_and_reads_a_file_again_only_when_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = write(root, "main", "C--code-app", ID, &[user("one"), title("First")]);
        write(root, "elif", "C--code-app", ID, &[user("one"), title("First, moved")]);
        write(root, "elif", "D--other", "11111111-2222-4333-8444-555555555555", &[user("two")]);
        fs::write(root.join("main").join("projects").join("C--code-app").join("notes.txt"), "not a conversation").unwrap();
        let profiles = [("main", root.join("main")), ("elif", root.join("elif")), ("new", root.join("no-such-folder"))];
        let mut index = ConversationIndex::new();
        let mut first: Vec<String> = index.list(&profiles).iter().map(|e| format!("{}:{}", e.profile_id, e.title)).collect();
        first.sort();
        assert_eq!(first, ["elif:First, moved", "elif:two", "main:First"]);
        // the same id in two accounts stays two entries
        assert_eq!(index.find("main", ID).map(|e| e.title.as_str()), Some("First"));
        assert_eq!(index.find("elif", ID).map(|e| e.title.as_str()), Some("First, moved"));
        assert_eq!(index.find("main", "nope"), None);

        let mut more = fs::OpenOptions::new().append(true).open(&a).unwrap();
        std::io::Write::write_all(&mut more, (title("Renamed") + "\n").as_bytes()).unwrap();
        drop(more);
        assert_eq!(index.list(&profiles).iter().find(|e| e.profile_id == "main").map(|e| e.title.as_str()), Some("Renamed"));
        fs::remove_file(&a).unwrap();
        assert!(!index.list(&profiles).iter().any(|e| e.profile_id == "main"));
        assert!(!index.cache.contains_key(&a));
    }

    #[test]
    fn uses_what_it_read_while_a_file_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "main", "C--code-app", ID, &[user("one")]);
        let mut cache = ScanCache::new();
        assert_eq!(scan_profile(&dir.path().join("main"), "main", &mut cache)[0].title, "one");
        // a reading the cache holds for the same time and size is not read again
        let (time, size, _) = cache[&file].clone();
        let mut stale = summary(&file).unwrap();
        stale.title = "from the cache".into();
        cache.insert(file.clone(), (time, size, Some(stale)));
        assert_eq!(scan_profile(&dir.path().join("main"), "main", &mut cache)[0].title, "from the cache");
        // another account's readings are left alone
        cache.insert(dir.path().join("elif").join("projects").join("k").join("x.jsonl"), (time, size, None));
        scan_profile(&dir.path().join("main"), "main", &mut cache);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn lists_with_what_was_read_while_the_last_listing_still_answers() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "main", "C--code-app", ID, &[user("one")]);
        let profiles = [("main", dir.path().join("main"))];
        let mut shared = ConversationIndex::new();
        shared.list(&profiles);
        let mut reading = shared.take_reading();
        // the old index still finds what it listed; the readings went with the new one
        assert_eq!(shared.find("main", ID).map(|e| e.title.as_str()), Some("one"));
        assert!(shared.cache.is_empty() && reading.entries().is_empty());
        let (time, size, _) = reading.cache[&file].clone();
        let mut cached = summary(&file).unwrap();
        cached.title = "from the cache".into();
        reading.cache.insert(file.clone(), (time, size, Some(cached)));
        assert_eq!(reading.list(&profiles)[0].title, "from the cache");
        shared = reading;
        assert_eq!(shared.find("main", ID).map(|e| e.title.as_str()), Some("from the cache"));
    }
}
