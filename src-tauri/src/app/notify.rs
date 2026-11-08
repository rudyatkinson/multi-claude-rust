//! System notifications of the app itself (usage limits; Phase 2: workflow runs). A click on one
//! brings the window to the front. Session alerts have their own (`session_alerts::TauriOutput`),
//! which also brings the session's tab to the front.

use std::collections::VecDeque;
use std::sync::{Mutex, Once};

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::app::window::bring_to_front;
use crate::state::lock;

/// How many shown notifications a click is still traced back to.
const KEPT: usize = 64;

static SHOWN: Mutex<VecDeque<i32>> = Mutex::new(VecDeque::new());
static LISTENING: Once = Once::new();

/// Registers, once, what a click on one of these notifications does. The plugin reports clicks
/// only for notifications shown after a handler exists, so this runs before the first one.
fn listen_for_clicks(app: &AppHandle) {
    LISTENING.call_once(|| {
        let app = app.clone();
        let registered = app.clone().notification().on_action(move |performed| {
            if !matches!(performed.action_id(), "tap" | "default") {
                return;
            }
            let Some(shown) = performed.notification() else { return };
            if !lock(&SHOWN).contains(&shown.id()) {
                return;
            }
            if let Some(state) = crate::app::state(&app) {
                bring_to_front(&state);
            }
        });
        if let Err(e) = registered {
            log::warn!("notification clicks: {e}");
        }
    });
}

/// Shows a desktop notification; clicking it brings the window to the front.
pub fn show(app: &AppHandle, title: &str, body: &str) {
    listen_for_clicks(app);
    let id = rand::random::<i32>() & i32::MAX;
    {
        let mut shown = lock(&SHOWN);
        shown.push_back(id);
        while shown.len() > KEPT {
            shown.pop_front();
        }
    }
    let mut builder = app.notification().builder().id(id).title(title);
    if !body.is_empty() {
        builder = builder.body(body);
    }
    if let Err(e) = builder.show() {
        log::warn!("notification: {e}");
    }
}
