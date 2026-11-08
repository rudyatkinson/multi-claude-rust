//! System notifications of the app itself (usage limits, workflow runs, the auto switch). A click
//! on one brings the window to the front, and the tab it is about when it names one. Session
//! alerts have their own (`session_alerts::TauriOutput`).

use std::collections::VecDeque;
use std::sync::{Mutex, Once};

use mc_core::types::Activate;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::app::window::bring_to_front;
use crate::events;
use crate::state::lock;

/// How many shown notifications a click is still traced back to.
const KEPT: usize = 64;

/// The notifications shown, with the tab a click brings to the front.
static SHOWN: Mutex<VecDeque<(i32, Option<String>)>> = Mutex::new(VecDeque::new());
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
            let Some(tab) = lock(&SHOWN).iter().find(|(id, _)| *id == shown.id()).map(|(_, tab)| tab.clone()) else { return };
            if let Some(state) = crate::app::state(&app) {
                bring_to_front(&state);
            }
            if let Some(id) = tab {
                events::emit(&app, events::SESSIONS_ACTIVATE, Activate { id });
            }
        });
        if let Err(e) = registered {
            log::warn!("notification clicks: {e}");
        }
    });
}

/// Shows a desktop notification; clicking it brings the window to the front.
pub fn show(app: &AppHandle, title: &str, body: &str) {
    show_for(app, title, body, None);
}

/// Shows a desktop notification; clicking it brings the window to the front and, when `tab` is
/// given, that session's tab (`sessions:activate`).
pub fn show_for(app: &AppHandle, title: &str, body: &str, tab: Option<&str>) {
    listen_for_clicks(app);
    let id = rand::random::<i32>() & i32::MAX;
    {
        let mut shown = lock(&SHOWN);
        shown.push_back((id, tab.map(str::to_owned)));
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
