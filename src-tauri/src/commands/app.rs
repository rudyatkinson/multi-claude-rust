//! Commands of the app itself (`app:*`, `settings:update`, `shell:openExternal`, `clipboard:*`,
//! `dialog:pickFolder`, `fs:folderFor`, `explorerMenu:*`, `ui:unsaved`).

use std::path::Path;

use mc_core::jsstr::cut16;
use mc_core::types::{AppState as WindowState, ClipboardContent, ExplorerMenuStatus, ExplorerMenuTier, UNSAVED_MAX_CHARS, UNSAVED_MAX_ITEMS};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::app::window::main_window;
use crate::app::{emit_app_changed, locate_claude, refresh_all_profiles};
use crate::commands::stubs::NOT_YET;
use crate::error::{CmdError, CmdResult};
use crate::sharing::{SHARED_DIRS, SHARED_FILES};
use crate::state::{lock, AppState};

/// The application's own version, shown next to Claude Code's in the sidebar footer.
#[tauri::command]
pub fn app_get_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Node's `process.platform` for this build.
fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// `getState`: everything the window starts from.
pub fn window_state(state: &AppState) -> WindowState {
    let main_dir = state.profiles.paths().main_claude_dir();
    let existing = |names: &[&str]| names.iter().filter(|n| main_dir.join(n).exists()).map(|n| (*n).to_owned()).collect::<Vec<_>>();
    let config = state.profiles.config().read();
    WindowState {
        platform: platform().to_owned(),
        packaged: !tauri::is_dev(),
        os_build: state.os_build,
        home_dir: state.home_dir().to_string_lossy().into_owned(),
        profiles: state.profiles.views(),
        sessions: state.sessions.list(),
        layout: Some(state.tabs.layout()),
        usage: state.usage.snapshot(),
        settings: config.settings,
        recent_folders: config.recent_folders,
        folder_accounts: config.folder_accounts,
        claude: state.claude_info(),
        shareable_dirs: existing(&SHARED_DIRS),
        shareable_files: existing(&SHARED_FILES),
        // Phase 2
        workflows: Vec::new(),
        runs: Vec::new(),
    }
}

/// `app:getState`.
#[tauri::command]
pub async fn app_get_state(state: State<'_, AppState>) -> CmdResult<WindowState> {
    Ok(window_state(&state))
}

/// `app:ready`: the renderer has its listeners in place and can take queued "open folder" requests.
#[tauri::command]
pub fn app_ready(state: State<'_, AppState>) -> CmdResult<()> {
    crate::app::boot::renderer_ready(&state);
    Ok(())
}

/// `settings:update`: only the keys the window may change (`validate::settings_patch`). A new
/// claude path is looked for at once and the accounts are checked with it. `tray` and
/// `openAtLogin` are only stored until Phase 4 applies them.
#[tauri::command]
pub async fn settings_update(state: State<'_, AppState>, patch: Option<Value>) -> CmdResult<()> {
    let patch = validate::settings_patch(&patch.unwrap_or(Value::Null));
    let config = state.profiles.config();
    let path_changed = patch.claude_path.as_ref().is_some_and(|next| config.with(|c| *next != c.settings.claude_path));
    config.update(|c| patch.apply_to(&mut c.settings))?;
    if path_changed {
        locate_claude(&state).await;
        let app = state.app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(state) = crate::app::state(&app) {
                refresh_all_profiles(&state).await;
            }
        });
    } else {
        emit_app_changed(&state);
    }
    Ok(())
}

/// `shell:openExternal`: http(s) links only.
#[tauri::command]
pub async fn shell_open_external(app: AppHandle, url: String) -> CmdResult<()> {
    let url = validate::url_http(&url)?;
    app.opener().open_url(url, None::<&str>).map_err(|e| CmdError(e.to_string()))
}

/// `clipboard:read`: the text, and whether there is an image (which the terminal pastes as a file).
#[tauri::command]
pub async fn clipboard_read(app: AppHandle) -> CmdResult<ClipboardContent> {
    let clipboard = app.clipboard();
    Ok(ClipboardContent { text: clipboard.read_text().unwrap_or_default(), has_image: clipboard.read_image().is_ok() })
}

/// `clipboard:writeText`.
#[tauri::command]
pub async fn clipboard_write_text(app: AppHandle, text: String) -> CmdResult<()> {
    let text = validate::clipboard_text(&text)?;
    app.clipboard().write_text(text).map_err(|e| CmdError(e.to_string()))
}

/// `dialog:pickFolder`: asks for a folder (one may be made there); null when that was cancelled.
#[tauri::command]
pub async fn dialog_pick_folder(app: AppHandle, default_path: Option<Value>) -> CmdResult<Option<String>> {
    let mut dialog = app.dialog().file().set_title("Choose a folder").set_can_create_directories(true);
    if let Some(win) = main_window(&app) {
        dialog = dialog.set_parent(&win);
    }
    if let Some(dir) = default_path.as_ref().and_then(Value::as_str).filter(|d| !d.is_empty()) {
        dialog = dialog.set_directory(dir);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder()).await.map_err(|e| CmdError(e.to_string()))?;
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

/// `fs:folderFor`: the folder a dropped path stands for: itself, or the folder of a file. Null when
/// it is neither, or not an absolute path.
#[tauri::command]
pub async fn fs_folder_for(path: String) -> CmdResult<Option<String>> {
    let Some(target) = validate::path(&path)? else { return Ok(None) };
    let folder = match tokio::fs::metadata(&target).await {
        Ok(meta) if meta.is_dir() => Some(target.as_path()),
        Ok(meta) if meta.is_file() => target.parent(),
        _ => None,
    };
    Ok(folder.map(Path::to_string_lossy).map(|p| p.into_owned()))
}

/// `explorerMenu:status`: the File Explorer entry comes with Phase 4; until then it is off.
#[tauri::command]
pub fn explorer_menu_status() -> CmdResult<ExplorerMenuStatus> {
    Ok(ExplorerMenuStatus {
        supported: false,
        tier: None,
        available: ExplorerMenuTier::Classic,
        reason: Some("The context menu entry is not available yet in this build.".into()),
        detail: None,
    })
}

/// `explorerMenu:set` (Phase 4).
#[tauri::command]
pub fn explorer_menu_set() -> CmdResult<ExplorerMenuStatus> {
    Err(CmdError(NOT_YET.into()))
}

/// `ui:unsaved` (send): the drafts with unsaved changes in the window's Library, cut down to a
/// few short texts (`cleanUnsaved`), for the quit question.
#[tauri::command]
pub fn ui_unsaved(state: State<'_, AppState>, payload: Option<Value>) -> CmdResult<()> {
    let items = payload.as_ref().and_then(|p| p.get("items")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let clean: Vec<String> = items
        .iter()
        .filter_map(Value::as_str)
        .filter(|item| !item.trim().is_empty())
        .take(UNSAVED_MAX_ITEMS)
        .map(|item| cut16(&item.split_whitespace().collect::<Vec<_>>().join(" "), UNSAVED_MAX_CHARS).to_owned())
        .collect();
    *lock(&state.unsaved) = clean;
    Ok(())
}
