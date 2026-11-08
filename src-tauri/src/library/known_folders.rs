//! The folders the Library looks at (`libraryRoots`, `knownFolders`, `pickLibraryFolder`,
//! `addLibraryFolder` and `removeLibraryFolder` of `src/main/index.ts`): where Claude sessions
//! run, what was opened lately, and the project folders the user added by hand (kept in
//! `settings.skillFolders`, at most [`PINNED_MAX`]).

use mc_core::error::AppResult;
use mc_core::folders::folder_key;
use mc_core::types::{SessionKind, SkillRootKind};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::app::window::main_window;
use crate::config_store::ConfigStore;
use crate::error::{CmdError, CmdResult};
use crate::library::roots::{find_roots, RootInputs, RootSpec, RootSub};
use crate::state::AppState;

/// How many folders the user can add to the Library's list; the oldest go first.
pub const PINNED_MAX: usize = 30;

fn key(folder: &str) -> String {
    folder_key(folder, cfg!(windows))
}

/// Where the Claude sessions run.
pub fn session_folders(state: &AppState) -> Vec<String> {
    state.sessions.list().into_iter().filter(|s| s.kind == SessionKind::Claude).map(|s| s.cwd).collect()
}

/// `settings.skillFolders`: the folders the user added to the Library by hand.
pub fn pinned_folders(state: &AppState) -> Vec<String> {
    state.profiles.config().with(|c| c.settings.skill_folders.clone().unwrap_or_default())
}

/// `knownFolders`: the folders the app has to do with: where sessions run, what was opened lately,
/// what was added to the Library. Memory and instructions look at these.
pub fn known_folders(state: &AppState) -> Vec<String> {
    let mut folders = session_folders(state);
    folders.extend(state.profiles.config().with(|c| c.recent_folders.clone()));
    folders.extend(pinned_folders(state));
    folders
}

/// What [`find_roots`] looks at right now: every account with its Claude Code folder, the
/// sessions' and the recent folders, and the pinned ones.
pub fn root_inputs(state: &AppState) -> RootInputs {
    let paths = state.profiles.paths();
    let mut folders = session_folders(state);
    folders.extend(state.profiles.config().with(|c| c.recent_folders.clone()));
    RootInputs {
        profiles: state.profiles.list().into_iter().map(|p| (p.id.clone(), paths.config_dir_of(&p))).collect(),
        folders,
        pinned: pinned_folders(state),
    }
}

/// `libraryRoots`: the folders of the Library that hold skills, or agents, as they are right now.
pub fn library_roots(state: &AppState, sub: RootSub) -> Vec<RootSpec> {
    find_roots(sub, &root_inputs(state))
}

/// `accountNames`: the names of accounts, for telling whose personal folder a root is.
pub fn account_names(state: &AppState, ids: &[String]) -> String {
    ids.iter().filter_map(|id| state.profiles.get(id)).map(|p| p.name).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ")
}

/// The pinned folders with `folder` added at the end (the oldest go beyond [`PINNED_MAX`]); None
/// when it is in the list already, in any spelling.
pub fn pinned_with(pinned: &[String], folder: &str) -> Option<Vec<String>> {
    let wanted = key(folder);
    if pinned.iter().any(|f| key(f) == wanted) {
        return None;
    }
    let mut next = pinned.to_vec();
    next.push(folder.to_owned());
    let over = next.len().saturating_sub(PINNED_MAX);
    next.drain(..over);
    Some(next)
}

/// The pinned folders without `folder`, in any spelling.
pub fn pinned_without(pinned: &[String], folder: &str) -> Vec<String> {
    let gone = key(folder);
    pinned.iter().filter(|f| key(f) != gone).cloned().collect()
}

/// Keeps `folder` in the Library's list.
pub fn pin_folder(config: &ConfigStore, folder: &str) -> AppResult<()> {
    let pinned = config.with(|c| c.settings.skill_folders.clone().unwrap_or_default());
    match pinned_with(&pinned, folder) {
        Some(next) => config.update(|c| c.settings.skill_folders = Some(next)),
        None => Ok(()),
    }
}

/// Takes `folder` out of the Library's list.
pub fn unpin_folder(config: &ConfigStore, folder: &str) -> AppResult<()> {
    config.update(|c| {
        let pinned = c.settings.skill_folders.clone().unwrap_or_default();
        c.settings.skill_folders = Some(pinned_without(&pinned, folder));
    })
}

/// `pickLibraryFolder`: asks for a project folder and keeps it in the Library's list. None when
/// that was cancelled (or there is no window to ask in).
pub async fn pick_library_folder(app: &AppHandle, state: &AppState) -> CmdResult<Option<String>> {
    let Some(win) = main_window(app) else { return Ok(None) };
    let dialog = app.dialog().file().set_title("Choose a project folder").set_parent(&win);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder()).await.map_err(|e| CmdError(e.to_string()))?;
    let Some(folder) = picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()) else { return Ok(None) };
    pin_folder(state.profiles.config(), &folder)?;
    Ok(Some(folder))
}

/// The id of the project root a folder's `.claude/<sub>` is among `roots`; None when that folder's
/// skills are an account's own, as with the home folder.
pub fn project_root_for(roots: &[RootSpec], folder: &str, sub: RootSub) -> Option<String> {
    let dir = key(&std::path::Path::new(folder).join(".claude").join(sub.as_str()).to_string_lossy());
    roots.iter().find(|r| r.kind == SkillRootKind::Project && key(&r.dir.to_string_lossy()) == dir).map(|r| r.id.clone())
}

/// `addLibraryFolder`: asks for a project folder and keeps it in the Library's list; the id of its
/// skills or agents root.
pub async fn add_library_folder(app: &AppHandle, state: &AppState, sub: RootSub) -> CmdResult<Option<String>> {
    let Some(folder) = pick_library_folder(app, state).await? else { return Ok(None) };
    Ok(project_root_for(&library_roots(state, sub), &folder, sub))
}

/// `removeLibraryFolder`: takes the folder of a project root out of the Library's list. A root
/// that is not in the list, or not a project, changes nothing.
pub fn remove_library_folder(state: &AppState, root_id: &str, sub: RootSub) -> AppResult<()> {
    let Some(folder) = library_roots(state, sub).into_iter().find(|r| r.id == root_id).and_then(|r| r.folder) else { return Ok(()) };
    unpin_folder(state.profiles.config(), &folder.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn adds_a_folder_once_and_keeps_the_last_thirty() {
        let sep = std::path::MAIN_SEPARATOR;
        let a = format!("{sep}code{sep}a");
        assert_eq!(pinned_with(&[], &a), Some(vec![a.clone()]));
        // the same folder in another spelling is there already
        let spelled = format!("{a}{sep}");
        assert_eq!(pinned_with(std::slice::from_ref(&a), &spelled), None);
        if cfg!(windows) {
            assert_eq!(pinned_with(&strs(&[r"C:\Code\App"]), r"c:/code/app"), None);
        }
        let many: Vec<String> = (0..30).map(|i| format!("{sep}p{i}")).collect();
        let next = pinned_with(&many, &a).unwrap();
        assert_eq!(next.len(), 30);
        assert_eq!((next[0].as_str(), next[29].as_str()), (many[1].as_str(), a.as_str()));
    }

    #[test]
    fn takes_a_folder_out_in_any_spelling() {
        let sep = std::path::MAIN_SEPARATOR;
        let pinned = vec![format!("{sep}code{sep}a"), format!("{sep}code{sep}b")];
        assert_eq!(pinned_without(&pinned, &format!("{sep}code{sep}a{sep}")), [format!("{sep}code{sep}b")]);
    }

    #[test]
    fn keeps_the_list_in_the_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ConfigStore::new(tmp.path().to_path_buf());
        let folder = tmp.path().join("proj").to_string_lossy().into_owned();
        pin_folder(&config, &folder).unwrap();
        pin_folder(&config, &folder).unwrap();
        assert_eq!(config.settings().skill_folders, Some(vec![folder.clone()]));
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config.file()).unwrap()).unwrap();
        assert_eq!(saved["settings"]["skillFolders"][0], folder.as_str());
        unpin_folder(&config, &folder).unwrap();
        assert_eq!(config.settings().skill_folders, Some(vec![]));
    }

    #[test]
    fn finds_the_project_root_of_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join(".claude").join("skills")).unwrap();
        let inputs = RootInputs {
            profiles: vec![("main".into(), home.join(".claude"))],
            folders: vec![],
            pinned: vec![proj.to_string_lossy().into_owned(), home.to_string_lossy().into_owned()],
        };
        let roots = find_roots(RootSub::Skills, &inputs);
        let id = project_root_for(&roots, &proj.to_string_lossy(), RootSub::Skills);
        assert_eq!(id.as_deref(), roots.iter().find(|r| r.folder == Some(PathBuf::from(&proj))).map(|r| r.id.as_str()));
        assert!(id.is_some());
        // the home folder's skills are the account's own
        assert_eq!(project_root_for(&roots, &home.to_string_lossy(), RootSub::Skills), None);
        assert_eq!(project_root_for(&roots, &proj.to_string_lossy(), RootSub::Agents), None);
    }
}
