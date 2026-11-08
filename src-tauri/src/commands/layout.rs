//! Commands of the `layout` package: the window reports how its tabs are arranged.

use mc_core::layout::{parse_layout, reconcile};
use mc_core::types::{SessionRef, UiLayout, UiVisible};
use mc_core::validate;
use tauri::State;

use crate::error::CmdResult;
use crate::state::{is, lock, AppState};

/// `ui:layout` (send). The layout arrives typed but unchecked; `parse_layout` decides what is kept
/// (ids, uniqueness, depth, ratios, focus, `seq`), and a layout it refuses is ignored, as the
/// Electron version ignored it. A payload that is not even shaped like a layout fails to
/// deserialize, which a send ignores as well.
///
/// Then, as `setLayout` in `src/main/index.ts` did, the layout is reconciled with the session
/// list, so that the app's copy never names a session that is gone, and `tabs.json` is saved soon.
///
/// While the tabs of the last run are reopened, a report is of the layout from before them and
/// is ignored: the restored arrangement is on its way to the window.
#[tauri::command]
pub fn ui_layout(payload: UiLayout, state: State<'_, AppState>) -> CmdResult<()> {
    if is(&state.flags.restoring_layout) {
        return Ok(());
    }
    let raw = serde_json::to_value(&payload).map_err(|e| e.to_string())?;
    if let Some(layout) = parse_layout(&raw) {
        let refs: Vec<SessionRef> = state.sessions.list().into_iter().map(|s| SessionRef { id: s.id, replaces: None, place: None }).collect();
        state.tabs.note_layout(reconcile(&layout, &refs).into_owned());
        state.tabs.persist(&state.sessions.saved_tabs());
    }
    Ok(())
}

/// `ui:visible` (send): the session the user is looking at, the tab in front in the area that has
/// the keyboard while the window has the focus; null for none. The idle check leaves it awake.
/// An id that is not one is taken for none.
#[tauri::command]
pub fn ui_visible(payload: UiVisible, state: State<'_, AppState>) -> CmdResult<()> {
    let id = payload.id.filter(|id| validate::id(id, "session").is_ok());
    *lock(&state.visible) = id;
    Ok(())
}
