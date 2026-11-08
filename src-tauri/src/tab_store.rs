//! `tabs.json`: the open tabs and their arrangement, remembered between runs of the app
//! (`src/main/tabStore.ts`, plus `persistTabs`/`saveTabsNow`/`restoreTabs` of `src/main/index.ts`).
//!
//! [`TabStore`] also keeps the app's copy of the layout (`layout` in `index.ts`): the window
//! reports it (`ui:layout` → [`TabStore::note_layout`]), session changes reconcile it
//! ([`TabStore::update_layout`]), and saving writes it in its saved form.
//!
//! Saving is debounced as it was: the first change arms a 500 ms timer and later changes only
//! replace what it will write, so a burst of changes is one write with the newest state. On quit
//! [`TabStore::quit`] writes at once, while the sessions are still alive, and from then on nothing
//! is written any more: ending the sessions would otherwise save an empty list.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mc_core::error::AppResult;
use mc_core::layout::{active_id_of, deserialize, flatten, parse_saved_layout, remap_saved, serialize};
use mc_core::types::{
    Layout, ResumeConversation, RestorePlan, RestoreRequest, SavedLayout, SavedNode, SavedTab, SavedTabs,
};
use serde::Serialize;
use serde_json::Value;

/// How long a change waits before it is written, so that a burst of changes is one write.
pub const SAVE_DELAY: Duration = Duration::from_millis(500);

pub struct TabStore {
    inner: Arc<Inner>,
}

struct Inner {
    file: PathBuf,
    saves: Mutex<Saves>,
    layout: Mutex<Layout>,
}

#[derive(Default)]
struct Saves {
    /// The newest state, waiting for the timer.
    pending: Option<SavedTabs>,
    /// The timer that will write `pending`. Writing at once forgets it, so that a timer still
    /// asleep finds nothing to do when it wakes.
    timer: Option<u64>,
    timers_started: u64,
    /// Set by [`TabStore::quit`]: nothing is written any more.
    quitting: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    /// Writes while `saves` is held, so that an older state can never land after a newer one.
    fn write(&self, saves: &mut Saves, saved: &SavedTabs) {
        saves.timer = None;
        saves.pending = None;
        if let Err(e) = save_tabs(&self.file, saved) {
            // the tabs are simply not restored next time
            log::warn!("saving {}: {e}", self.file.display());
        }
    }

    fn timer_fired(&self, timer: u64) {
        let mut saves = lock(&self.saves);
        if saves.timer != Some(timer) || saves.quitting {
            return;
        }
        if let Some(saved) = saves.pending.take() {
            self.write(&mut saves, &saved);
        }
        saves.timer = None;
    }
}

impl TabStore {
    /// The store of `<root>/tabs.json`. The layout starts as one empty area.
    pub fn new(root: PathBuf) -> Self {
        TabStore {
            inner: Arc::new(Inner {
                file: root.join("tabs.json"),
                saves: Mutex::new(Saves::default()),
                layout: Mutex::new(Layout::default()),
            }),
        }
    }

    pub fn file(&self) -> &Path {
        &self.inner.file
    }

    /// What the file holds (`loadTabs`); nothing for a missing or damaged file.
    pub fn load(&self) -> SavedTabs {
        load_tabs(&self.inner.file)
    }

    /// Writes these tabs at once (`saveTabs`), in place of any save that is waiting. Does nothing
    /// once the app is quitting.
    pub fn save(&self, tabs: &[SavedTab], active: i64, layout: Option<SavedLayout>) -> AppResult<()> {
        let mut saves = lock(&self.inner.saves);
        if saves.quitting {
            return Ok(());
        }
        saves.timer = None;
        saves.pending = None;
        save_tabs(&self.inner.file, &SavedTabs { tabs: tabs.to_vec(), active, layout })
    }

    /// Remembers the open tabs for the next start (`persistTabs`): written [`SAVE_DELAY`] after
    /// the first of a burst of changes, with the newest state. Does nothing once the app is
    /// quitting. Needs the Tauri async runtime (any tokio runtime in tests).
    pub fn schedule_save(&self, saved: SavedTabs) {
        let mut saves = lock(&self.inner.saves);
        if saves.quitting {
            return;
        }
        saves.pending = Some(saved);
        // `saveTabsTimer ??= setTimeout(...)`: a timer that is running is not started again
        if saves.timer.is_some() {
            return;
        }
        saves.timers_started += 1;
        let timer = saves.timers_started;
        saves.timer = Some(timer);
        drop(saves);
        let inner = Arc::clone(&self.inner);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            inner.timer_fired(timer);
        });
    }

    /// [`schedule_save`](Self::schedule_save) of these open tabs, arranged as the layout the store
    /// keeps: what `persistTabs` did whenever the sessions or the layout changed.
    pub fn persist(&self, open: &[(String, SavedTab)]) {
        let saved = snapshot(&lock(&self.inner.layout), open);
        self.schedule_save(saved);
    }

    /// Writes a waiting save now instead of when its timer fires. Does nothing once the app is
    /// quitting.
    pub fn flush_now(&self) {
        let mut saves = lock(&self.inner.saves);
        saves.timer = None;
        if saves.quitting {
            return;
        }
        if let Some(saved) = saves.pending.take() {
            self.inner.write(&mut saves, &saved);
        }
    }

    /// The app is quitting, or Windows is ending the session (`shutdown`, `session-end`): a waiting
    /// save is written now, and from then on nothing is written. Call it while the sessions are
    /// still alive, after the last [`persist`](Self::persist). Safe to call more than once.
    pub fn quit(&self) {
        let mut saves = lock(&self.inner.saves);
        if saves.quitting {
            return;
        }
        saves.timer = None;
        if let Some(saved) = saves.pending.take() {
            self.inner.write(&mut saves, &saved);
        }
        saves.quitting = true;
    }

    pub fn is_quitting(&self) -> bool {
        lock(&self.inner.saves).quitting
    }

    /// The app's copy of the layout.
    pub fn layout(&self) -> Layout {
        lock(&self.inner.layout).clone()
    }

    /// Replaces the app's copy of the layout: the window reported one (`ui:layout`, after
    /// `parse_layout`), or tabs were restored. The caller reconciles it with the session list
    /// first and persists afterwards; the sessions live in another service.
    pub fn note_layout(&self, layout: Layout) {
        *lock(&self.inner.layout) = layout;
    }

    /// Runs a transition on the app's copy of the layout, e.g. `|l| reconcile(l, &refs)` when the
    /// session list changed. True when it changed the layout.
    pub fn update_layout<F>(&self, f: F) -> bool
    where
        F: for<'a> FnOnce(&'a Layout) -> Cow<'a, Layout>,
    {
        let mut layout = lock(&self.inner.layout);
        let next = match f(&layout) {
            Cow::Owned(next) => Some(next),
            Cow::Borrowed(_) => None,
        };
        let changed = next.is_some();
        if let Some(next) = next {
            *layout = next;
        }
        changed
    }
}

// ---------- the file ----------

/// `text`: a non-empty string, else None.
fn text(value: Option<&Value>) -> Option<String> {
    value?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// `readJson`: the parsed file, None when it is missing or not JSON. Invalid UTF-8 is replaced, as
/// Node's `readFileSync(file, 'utf8')` did.
fn read_json(file: &Path) -> Option<Value> {
    let bytes = std::fs::read(file).ok()?;
    serde_json::from_str(&String::from_utf8_lossy(&bytes)).ok()
}

/// `loadTabs`: the tabs of the file. Entries without an account or a folder are dropped, empty
/// strings count as missing, and the active index and the layout follow the entries that stay.
pub fn load_tabs(file: &Path) -> SavedTabs {
    let Some(raw) = read_json(file) else { return SavedTabs::default() };
    let Some(entries) = raw.get("tabs").and_then(Value::as_array) else { return SavedTabs::default() };
    let raw_active = raw.get("active").and_then(Value::as_f64);
    let mut tabs = Vec::new();
    let mut active = -1;
    // where each entry of the file ended up, for the layout that refers to them by position
    let mut moved: HashMap<i64, i64> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let (Some(profile_id), Some(cwd)) = (text(entry.get("profileId")), text(entry.get("cwd"))) else { continue };
        if raw_active == Some(index as f64) {
            active = tabs.len() as i64;
        }
        moved.insert(index as i64, tabs.len() as i64);
        tabs.push(SavedTab {
            profile_id,
            cwd,
            claude_session_id: text(entry.get("claudeSessionId")),
            transcript_path: text(entry.get("transcriptPath")),
            name: text(entry.get("name")),
        });
    }
    let layout = raw
        .get("layout")
        .and_then(parse_saved_layout)
        .and_then(|parsed| remap_saved(&parsed, |index| moved.get(&index).copied()));
    SavedTabs { tabs, active, layout }
}

/// `saveTabs`.
pub fn save_tabs(file: &Path, saved: &SavedTabs) -> AppResult<()> {
    write_json_atomic(file, saved)
}

/// `writeJsonAtomic`: pretty JSON with a final newline, written to a temporary file that then takes
/// the place of the old one. (`config_store` has the same helper; a small duplicate so that the
/// packages stay apart.)
fn write_json_atomic(file: &Path, data: &impl Serialize) -> AppResult<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let mut text = serde_json::to_string_pretty(data)?;
    text.push('\n');
    std::fs::write(&tmp, text)?;
    if std::fs::rename(&tmp, file).is_err() {
        // Windows can refuse the rename while another process has the target open; retry once.
        if let Err(e) = std::fs::rename(&tmp, file) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
    }
    Ok(())
}

/// What `saveTabsNow` wrote: the open tabs (session id, tab) in the order they are seen in the
/// layout, so that a version without areas still shows them in that order; tabs the layout does
/// not have go last, in their own order.
pub fn snapshot(layout: &Layout, open: &[(String, SavedTab)]) -> SavedTabs {
    let seen = flatten(layout);
    let rank = |id: &str| seen.iter().position(|s| *s == id).unwrap_or(seen.len());
    let mut sorted: Vec<&(String, SavedTab)> = open.iter().collect();
    sorted.sort_by_key(|(id, _)| rank(id));
    let ids: Vec<&str> = sorted.iter().map(|(id, _)| id.as_str()).collect();
    let active = active_id_of(layout).and_then(|a| ids.iter().position(|id| *id == a)).map_or(-1, |i| i as i64);
    SavedTabs {
        tabs: sorted.iter().map(|(_, tab)| tab.clone()).collect(),
        active,
        layout: serialize(layout, &ids),
    }
}

// ---------- restoring ----------

/// Whether the tab's conversation exists on disk. Claude Code only writes it once a message has
/// been sent, and resuming an id without one fails.
pub fn has_transcript(config_dir: &Path, claude_session_id: Option<&str>, transcript_path: Option<&str>) -> bool {
    let Some(id) = claude_session_id.filter(|id| !id.is_empty()) else { return false };
    if transcript_path.is_some_and(|p| !p.is_empty() && Path::new(p).exists()) {
        return true;
    }
    // no path reported yet: conversations live in <config>/projects/<folder key>/<id>.jsonl
    let Ok(dirs) = std::fs::read_dir(config_dir.join("projects")) else { return false };
    let file = format!("{id}.jsonl");
    dirs.flatten().any(|dir| dir.path().join(&file).exists())
}

/// Whether a saved tab's folder is still there.
pub fn folder_exists(cwd: &str) -> bool {
    std::fs::metadata(cwd).is_ok_and(|m| m.is_dir())
}

/// `planRestore`: which sessions to start for the saved tabs. Tabs whose account or folder is gone
/// are dropped; a tab resumes its conversation when that exists on disk, else it starts fresh.
pub fn plan_restore(
    saved: &SavedTabs,
    profile_exists: impl Fn(&str) -> bool,
    folder_exists: impl Fn(&str) -> bool,
    has_transcript: impl Fn(&SavedTab) -> bool,
) -> RestorePlan {
    let mut requests = Vec::new();
    let mut active = -1;
    let mut moved: HashMap<i64, i64> = HashMap::new();
    for (index, tab) in saved.tabs.iter().enumerate() {
        if !profile_exists(&tab.profile_id) || !folder_exists(&tab.cwd) {
            continue;
        }
        if index as i64 == saved.active {
            active = requests.len() as i64;
        }
        moved.insert(index as i64, requests.len() as i64);
        let resume = tab
            .claude_session_id
            .as_ref()
            .filter(|id| !id.is_empty() && has_transcript(tab))
            .map(|id| ResumeConversation { id: id.clone() });
        requests.push(RestoreRequest { profile_id: tab.profile_id.clone(), cwd: tab.cwd.clone(), resume, name: tab.name.clone() });
    }
    let layout = saved.layout.as_ref().and_then(|layout| remap_saved(layout, |index| moved.get(&index).copied()));
    RestorePlan { requests, active, layout }
}

/// [`plan_restore`] against the disk, as `restoreTabs` called it: `config_dir_of` gives the
/// effective config folder of an account, None when the account is gone.
pub fn plan_restore_on_disk(saved: &SavedTabs, config_dir_of: impl Fn(&str) -> Option<PathBuf>) -> RestorePlan {
    plan_restore(saved, |id| config_dir_of(id).is_some(), folder_exists, |tab| {
        config_dir_of(&tab.profile_id).is_some_and(|dir| {
            has_transcript(&dir, tab.claude_session_id.as_deref(), tab.transcript_path.as_deref())
        })
    })
}

/// The layout restored tabs come back in (end of `restoreTabs`). `started[i]` is the session
/// started for `plan.requests[i]`, if it started; `live` is every session there is now. A file
/// without areas gives one area with the remembered tab in front.
pub fn restored_layout(plan: &RestorePlan, started: &[Option<String>], live: &[String]) -> Layout {
    let flat = SavedLayout {
        root: SavedNode::Group { tabs: (0..plan.requests.len() as i64).collect(), active: plan.active },
        focused: 0,
    };
    deserialize(plan.layout.as_ref().unwrap_or(&flat), started, live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::layout::{groups_of, reconcile, split_with, Edge, SessionRef};
    use serde_json::json;

    fn tab(profile_id: &str, cwd: &str) -> SavedTab {
        SavedTab { profile_id: profile_id.into(), cwd: cwd.into(), claude_session_id: None, transcript_path: None, name: None }
    }

    fn with_id(profile_id: &str, cwd: &str, id: &str) -> SavedTab {
        SavedTab { claude_session_id: Some(id.into()), ..tab(profile_id, cwd) }
    }

    fn named(profile_id: &str, cwd: &str, name: &str) -> SavedTab {
        SavedTab { name: Some(name.into()), ..tab(profile_id, cwd) }
    }

    fn saved_layout(value: Value) -> SavedLayout {
        serde_json::from_value(value).unwrap()
    }

    fn write_raw(file: &Path, value: Value) {
        std::fs::write(file, serde_json::to_string(&value).unwrap()).unwrap();
    }

    fn everything(_: &str) -> bool {
        true
    }

    fn never(_: &SavedTab) -> bool {
        false
    }

    mod saved_tabs {
        use super::*;

        #[test]
        fn round_trips_through_the_file() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("tabs.json");
            let first = SavedTab { transcript_path: Some(r"C:\t\id-1.jsonl".into()), ..with_id("main", r"C:\code\app", "id-1") };
            let saved = SavedTabs { tabs: vec![first.clone(), tab("work", r"D:\x")], active: 1, layout: None };
            save_tabs(&file, &saved).unwrap();
            assert_eq!(load_tabs(&file), SavedTabs { tabs: vec![first, tab("work", r"D:\x")], active: 1, layout: None });
        }

        #[test]
        fn keeps_the_name_the_user_gave_a_tab() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("tabs.json");
            let saved =
                SavedTabs { tabs: vec![named("main", r"C:\a", "Release notes"), named("main", r"C:\b", "")], active: 0, layout: None };
            save_tabs(&file, &saved).unwrap();
            let loaded = load_tabs(&file);
            assert_eq!(loaded.tabs.iter().map(|t| t.name.as_deref()).collect::<Vec<_>>(), [Some("Release notes"), None]);
            let plan = plan_restore(&loaded, everything, everything, never);
            assert_eq!(plan.requests.iter().map(|r| r.name.as_deref()).collect::<Vec<_>>(), [Some("Release notes"), None]);
        }

        #[test]
        fn keeps_the_areas_the_tabs_were_in_by_tab_position() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("tabs.json");
            let layout = saved_layout(json!({ "root": { "dir": "row", "ratio": 0.3, "first": { "tabs": [0, 2], "active": 2 }, "second": { "tabs": [1], "active": 1 } }, "focused": 1 }));
            let tabs = vec![tab("main", r"C:\a"), tab("work", r"C:\b"), tab("main", r"C:\c")];
            save_tabs(&file, &SavedTabs { tabs, active: 1, layout: Some(layout.clone()) }).unwrap();
            assert_eq!(load_tabs(&file).layout, Some(layout));
        }

        #[test]
        fn follows_the_tabs_when_entries_of_the_file_are_unreadable_or_cannot_be_reopened() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("tabs.json");
            let layout = json!({ "root": { "dir": "row", "ratio": 0.5, "first": { "tabs": [0, 1], "active": 1 }, "second": { "tabs": [2, 3], "active": 3 } }, "focused": 1 });
            // entry 1 has no folder
            write_raw(
                &file,
                json!({ "tabs": [{ "profileId": "main", "cwd": r"C:\a" }, { "profileId": "main" }, { "profileId": "gone", "cwd": r"C:\c" }, { "profileId": "main", "cwd": r"C:\d" }], "active": 3, "layout": layout }),
            );
            let loaded = load_tabs(&file);
            assert_eq!(
                loaded.layout,
                Some(saved_layout(json!({ "root": { "dir": "row", "ratio": 0.5, "first": { "tabs": [0], "active": 0 }, "second": { "tabs": [1, 2], "active": 2 } }, "focused": 1 })))
            );
            // then the account of what is now entry 1 turns out to be deleted
            let plan = plan_restore(&loaded, |id| id != "gone", everything, never);
            assert_eq!(plan.requests.iter().map(|r| r.cwd.as_str()).collect::<Vec<_>>(), [r"C:\a", r"C:\d"]);
            assert_eq!(
                plan.layout,
                Some(saved_layout(json!({ "root": { "dir": "row", "ratio": 0.5, "first": { "tabs": [0], "active": 0 }, "second": { "tabs": [1], "active": 1 } }, "focused": 1 })))
            );
        }

        #[test]
        fn ignores_a_layout_that_is_damaged_and_has_none_for_files_of_older_versions() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("tabs.json");
            write_raw(&file, json!({ "tabs": [{ "profileId": "main", "cwd": r"C:\a" }], "active": 0, "layout": { "root": { "tabs": [0, 0] } } }));
            assert_eq!(load_tabs(&file).layout, None);
            write_raw(&file, json!({ "tabs": [{ "profileId": "main", "cwd": r"C:\a" }], "active": 0 }));
            let loaded = load_tabs(&file);
            assert_eq!(loaded.layout, None);
            assert_eq!(plan_restore(&loaded, everything, everything, never).layout, None);
        }

        #[test]
        fn loads_nothing_from_a_missing_or_damaged_file_and_skips_broken_entries() {
            let dir = tempfile::tempdir().unwrap();
            assert_eq!(load_tabs(&dir.path().join("missing.json")), SavedTabs::default());
            let file = dir.path().join("tabs.json");
            std::fs::write(&file, r#"{"tabs": "nope"}"#).unwrap();
            assert_eq!(load_tabs(&file), SavedTabs::default());

            // the active index follows its tab when an entry before it is dropped
            write_raw(&file, json!({ "tabs": [{ "cwd": "no profile" }, { "profileId": "main", "cwd": r"C:\a" }], "active": 1 }));
            assert_eq!(load_tabs(&file), SavedTabs { tabs: vec![tab("main", r"C:\a")], active: 0, layout: None });
        }

        #[test]
        fn finds_a_conversation_by_its_reported_path_or_by_its_id_under_projects() {
            let dir = tempfile::tempdir().unwrap();
            let reported = dir.path().join("elsewhere.jsonl");
            std::fs::write(&reported, "").unwrap();
            assert!(has_transcript(dir.path(), Some("id-1"), reported.to_str()));

            std::fs::create_dir_all(dir.path().join("projects").join("C--code-app")).unwrap();
            std::fs::write(dir.path().join("projects").join("C--code-app").join("id-2.jsonl"), "").unwrap();
            assert!(has_transcript(dir.path(), Some("id-2"), None));
            assert!(!has_transcript(dir.path(), Some("id-3"), None));
            assert!(!has_transcript(dir.path(), None, None));
            assert!(!has_transcript(&dir.path().join("no-such-config"), Some("id-2"), None));
        }
    }

    mod planning_a_restore {
        use super::*;

        fn saved() -> SavedTabs {
            SavedTabs {
                tabs: vec![
                    with_id("gone", r"C:\a", "id-0"),
                    with_id("main", r"C:\a", "id-1"),
                    with_id("main", r"C:\deleted", "id-2"),
                    with_id("work", r"C:\b", "id-3"),
                    tab("work", r"C:\b"),
                ],
                active: 3,
                layout: None,
            }
        }

        fn plan(saved: &SavedTabs) -> RestorePlan {
            plan_restore(saved, |id| id != "gone", |cwd| cwd != r"C:\deleted", |t| t.claude_session_id.as_deref() == Some("id-1"))
        }

        #[test]
        fn drops_tabs_whose_account_or_folder_is_gone_and_keeps_the_active_tab_in_front() {
            let plan = plan(&saved());
            let names: Vec<String> = plan.requests.iter().map(|r| format!("{} {}", r.profile_id, r.cwd)).collect();
            assert_eq!(names, [r"main C:\a", r"work C:\b", r"work C:\b"]);
            assert_eq!(plan.active, 1);
        }

        #[test]
        fn resumes_only_conversations_that_exist_on_disk() {
            let plan = plan(&saved());
            assert_eq!(plan.requests[0].resume, Some(ResumeConversation { id: "id-1".into() }));
            // opened but never used: no conversation was written, so it starts fresh
            assert_eq!(plan.requests[1].resume, None);
            assert_eq!(plan.requests[2].resume, None);
        }

        #[test]
        fn has_no_active_tab_when_the_active_one_could_not_be_restored() {
            assert_eq!(plan(&SavedTabs { active: 0, ..saved() }).active, -1);
            assert_eq!(plan(&SavedTabs::default()), RestorePlan { requests: vec![], active: -1, layout: None });
        }
    }

    // ---------- not in the TypeScript tests ----------

    fn layout_of(ids: &[&str]) -> Layout {
        let refs: Vec<SessionRef> = ids.iter().map(|id| SessionRef { id: id.to_string(), replaces: None, place: None }).collect();
        reconcile(&Layout::default(), &refs).into_owned()
    }

    #[test]
    fn a_snapshot_orders_the_tabs_as_the_layout_shows_them() {
        // s1 s3 | s2 with s2 in front; s9 is open but not in the layout (it just started)
        let l = split_with(&layout_of(&["s1", "s2", "s3"]), "s2", "g1", Edge::Right).into_owned();
        let open = vec![
            ("s9".to_string(), tab("p", "nine")),
            ("s2".to_string(), tab("p", "two")),
            ("s3".to_string(), tab("p", "three")),
            ("s1".to_string(), tab("p", "one")),
        ];
        let saved = snapshot(&l, &open);
        assert_eq!(saved.tabs.iter().map(|t| t.cwd.as_str()).collect::<Vec<_>>(), ["one", "three", "two", "nine"]);
        assert_eq!(saved.active, 2);
        assert_eq!(
            saved.layout,
            Some(saved_layout(json!({ "root": { "dir": "row", "ratio": 0.5, "first": { "tabs": [0, 1], "active": 0 }, "second": { "tabs": [2], "active": 2 } }, "focused": 1 })))
        );
        // and back: the plan's layout puts the restarted sessions where they were
        let plan = plan_restore(&saved, everything, everything, never);
        let started: Vec<Option<String>> = ["n1", "n3", "n2", "n9"].iter().map(|id| Some(id.to_string())).collect();
        let live: Vec<String> = ["n1", "n3", "n2", "n9"].iter().map(|id| id.to_string()).collect();
        let back = restored_layout(&plan, &started, &live);
        assert_eq!(flatten(&back), ["n1", "n3", "n2", "n9"]);
        assert_eq!(active_id_of(&back), Some("n2"));
        let groups: Vec<Vec<String>> = groups_of(&back.root).iter().map(|g| g.tabs.clone()).collect();
        // the session the layout did not know joins the area with the keyboard
        assert_eq!(groups, [vec!["n1", "n3"], vec!["n2", "n9"]]);
    }

    #[test]
    fn a_file_without_areas_comes_back_as_one_area_with_the_remembered_tab_in_front() {
        let plan = plan_restore(&SavedTabs { tabs: vec![tab("p", "a"), tab("p", "b")], active: 1, layout: None }, everything, everything, never);
        let back = restored_layout(&plan, &[Some("x".into()), Some("y".into())], &["x".into(), "y".into()]);
        assert_eq!(flatten(&back), ["x", "y"]);
        assert_eq!(active_id_of(&back), Some("y"));
    }

    #[test]
    fn saves_once_per_burst_and_never_after_quitting() {
        let dir = tempfile::tempdir().unwrap();
        let store = TabStore::new(dir.path().to_path_buf());
        let state = |name: &str| SavedTabs { tabs: vec![tab("p", name)], active: 0, layout: None };
        store.schedule_save(state("first"));
        store.schedule_save(state("second"));
        // nothing before the delay
        assert!(!store.file().exists());
        std::thread::sleep(SAVE_DELAY * 3);
        assert_eq!(store.load().tabs[0].cwd, "second");

        // flushing writes at once, and the sleeping timer then has nothing to do
        store.schedule_save(state("third"));
        store.flush_now();
        assert_eq!(store.load().tabs[0].cwd, "third");

        // quitting writes what waits, then nothing more
        store.schedule_save(state("fourth"));
        store.quit();
        assert_eq!(store.load().tabs[0].cwd, "fourth");
        assert!(store.is_quitting());
        store.schedule_save(state("fifth"));
        store.flush_now();
        store.save(&[tab("p", "sixth")], 0, None).unwrap();
        std::thread::sleep(SAVE_DELAY * 3);
        assert_eq!(store.load().tabs[0].cwd, "fourth");
    }

    #[test]
    fn keeps_the_layout_and_says_when_a_transition_changed_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = TabStore::new(dir.path().to_path_buf());
        assert_eq!(store.layout(), Layout::default());
        store.note_layout(layout_of(&["a", "b"]));
        let refs = |ids: &[&str]| ids.iter().map(|id| SessionRef { id: id.to_string(), replaces: None, place: None }).collect::<Vec<_>>();
        let same = refs(&["a", "b"]);
        assert!(!store.update_layout(|l| reconcile(l, &same)));
        let fewer = refs(&["a"]);
        assert!(store.update_layout(|l| reconcile(l, &fewer)));
        assert_eq!(flatten(&store.layout()), ["a"]);

        // persisting arranges the open tabs as that layout has them
        store.persist(&[("a".to_string(), tab("p", "a"))]);
        store.flush_now();
        let loaded = store.load();
        assert_eq!(loaded.active, 0);
        assert_eq!(loaded.layout, Some(saved_layout(json!({ "root": { "tabs": [0], "active": 0 }, "focused": 0 }))));
    }

    #[test]
    fn writes_pretty_json_with_a_final_newline_and_no_temporary_file_left() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("tabs.json");
        save_tabs(&file, &SavedTabs { tabs: vec![tab("p", "c")], active: 0, layout: None }).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("{\n  \"tabs\": [\n"));
        assert!(text.ends_with("}\n"));
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    }
}
