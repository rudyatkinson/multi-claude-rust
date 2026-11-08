//! Commands of the `layout` package: the window reports how its tabs are arranged.

use mc_core::layout::parse_layout;
use mc_core::types::UiLayout;
use tauri::State;

use crate::error::CmdResult;
use crate::state::AppState;

/// `ui:layout` (send). The layout arrives typed but unchecked; `parse_layout` decides what is kept
/// (ids, uniqueness, depth, ratios, focus, `seq`), and a layout it refuses is ignored, as the
/// Electron version ignored it. A payload that is not even shaped like a layout fails to
/// deserialize, which a send ignores as well.
///
/// The Electron version then reconciled the layout with the session list and scheduled a save of
/// `tabs.json` (`setLayout` in `src/main/index.ts`). The session list lives in the session
/// service, so integration adds those two steps here, after `note_layout`:
/// `state.tabs.update_layout(|l| reconcile(l, &refs_of_the_sessions))` and
/// `state.tabs.persist(&open_tabs)`.
#[tauri::command]
pub fn ui_layout(payload: UiLayout, state: State<'_, AppState>) -> CmdResult<()> {
    let raw = serde_json::to_value(&payload).map_err(|e| e.to_string())?;
    if let Some(layout) = parse_layout(&raw) {
        state.tabs.note_layout(layout);
    }
    Ok(())
}
