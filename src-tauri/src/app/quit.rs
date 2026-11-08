//! Quitting from the window and the tray (`confirmQuit`, `quitFromTray`, `trayHint` of
//! `src/main/index.ts`): the question before running sessions, workflow runs or unsaved drafts
//! are lost, and closing the window into the tray.
//!
//! The question is a system dialog that blocks until it is answered, so it is asked off the main
//! thread ([`request_quit`]); the main thread has to keep running to show it.

use std::sync::atomic::Ordering;

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::app::boot::shutdown;
use crate::app::window::main_window;
use crate::app::{notify, state as app_state};
use crate::state::{lock, AppState};

/// What the quit question says: its first line and the paragraphs after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuitQuestion {
    pub message: String,
    pub detail: String,
}

/// `unsavedNote`: the paragraph of the quit question about the drafts that are not saved; empty
/// when there are none. Each item starts a sentence: `The agent "reviewer"`.
pub fn unsaved_note(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => format!("{one} has changes that are not saved. They are lost when the app quits."),
        many => {
            let list: Vec<String> = many.iter().map(|item| format!("  • {item}")).collect();
            format!("These have changes that are not saved, which are lost when the app quits:\n{}", list.join("\n"))
        }
    }
}

/// The question to ask before quitting, None when nothing would be lost: sessions are running,
/// workflow runs (by name) are active, or drafts are not saved.
pub fn quit_question(running: bool, restore_tabs: Option<bool>, workflows: &[String], unsaved: &[String]) -> Option<QuitQuestion> {
    if !running && workflows.is_empty() && unsaved.is_empty() {
        return None;
    }
    let sessions = match (running, restore_tabs) {
        (false, _) => String::new(),
        (true, Some(false)) => "Quitting ends every open Claude session.".to_owned(),
        (true, _) => "Quitting ends the running Claude processes. Your tabs reopen next time and continue their conversations.".to_owned(),
    };
    let runs = if workflows.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = workflows.iter().map(|name| format!("\"{name}\"")).collect();
        format!("The workflow {} ends here as well: a run does not go on after a restart.", names.join(", "))
    };
    let notes = [sessions, runs, unsaved_note(unsaved)];
    let message = if running {
        "Sessions are still running"
    } else if !unsaved.is_empty() {
        "There are unsaved changes"
    } else {
        "A workflow is still running"
    };
    Some(QuitQuestion { message: message.to_owned(), detail: notes.into_iter().filter(|n| !n.is_empty()).collect::<Vec<_>>().join("\n\n") })
}

/// `confirmQuit`: asks before running sessions, active workflow runs or unsaved drafts are lost.
/// True when there is nothing to lose or the user chose Quit. Blocks until answered: never call it
/// on the main thread, which shows the dialog.
pub fn confirm_quit(app: &AppHandle) -> bool {
    let Some(state) = app_state(app) else { return true };
    let workflows: Vec<String> = state.runs.active().into_iter().map(|run| run.name).collect();
    let unsaved = lock(&state.unsaved).clone();
    let Some(question) = quit_question(state.sessions.has_running(), state.settings().restore_tabs, &workflows, &unsaved) else {
        return true;
    };
    let text = if question.detail.is_empty() { question.message } else { format!("{}\n\n{}", question.message, question.detail) };
    let mut dialog = app
        .dialog()
        .message(text)
        .title("Multi Claude")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("Quit".into(), "Cancel".into()));
    // on top of the window while it is shown, else on its own
    if let Some(win) = main_window(app).filter(|w| w.is_visible().unwrap_or(false)) {
        dialog = dialog.parent(&win);
    }
    dialog.blocking_show()
}

/// `quitFromTray` and closing the window without a tray icon: asks (off the main thread), then
/// quits when the answer is Quit. While a question is open, asking again does nothing.
pub fn request_quit(app: &AppHandle) {
    let Some(state) = app_state(app) else { return };
    if state.flags.asking_to_quit.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let confirmed = confirm_quit(&app);
        if let Some(state) = app_state(&app) {
            state.flags.asking_to_quit.store(false, Ordering::SeqCst);
        }
        if confirmed {
            quit_now(&app);
        }
    });
}

/// Quits without asking: the tabs are saved and the sessions end ([`shutdown`]), then the app exits.
pub fn quit_now(app: &AppHandle) {
    if let Some(state) = app_state(app) {
        shutdown(&state);
    }
    app.exit(0);
}

/// The window was closed while there is a tray icon: it only hides, the sessions keep running,
/// and the first time a notification says so (`trayHint`).
pub fn hide_to_tray(state: &AppState) {
    if let Some(win) = main_window(&state.app) {
        if let Err(e) = win.hide() {
            log::warn!("hiding the window: {e}");
        }
    }
    if state.settings().tray_hint_shown == Some(true) {
        return;
    }
    if let Err(e) = state.profiles.config().update(|c| c.settings.tray_hint_shown = Some(true)) {
        log::warn!("settings: {e}");
    }
    notify::show(
        &state.app,
        "Multi Claude is still running",
        "Sessions keep working in the background. The tray icon opens the window again; its menu has Quit.",
    );
}

/// macOS: the Quit of the app menu (Cmd+Q).
#[cfg(target_os = "macos")]
pub const APP_QUIT: &str = "app:quit";

/// macOS: the app menu of the Electron version (the app's menu, Edit, Window) with a Quit item
/// that asks first. The predefined Quit ends the app through `terminate:`, which reaches the app
/// only as `RunEvent::Exit`, too late to ask.
#[cfg(target_os = "macos")]
pub fn install_app_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    let quit = MenuItemBuilder::with_id(APP_QUIT, "Quit Multi Claude").accelerator("CmdOrCtrl+Q").build(app)?;
    let name = app.package_info().name.clone();
    let app_menu = SubmenuBuilder::new(app, &name)
        .about(Some(AboutMetadata::default()))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&quit)
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let window = SubmenuBuilder::new(app, "Window").minimize().maximize().separator().bring_all_to_front().build()?;
    let menu = MenuBuilder::new(app).item(&app_menu).item(&edit).item(&window).build()?;
    app.set_menu(menu)?;
    Ok(())
}

/// macOS: the items of the app menu that are the app's own.
#[cfg(target_os = "macos")]
pub fn on_menu_event(app: &AppHandle, event: &tauri::menu::MenuEvent) {
    if event.id().as_ref() == APP_QUIT {
        request_quit(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn names_the_unsaved_drafts() {
        assert_eq!(unsaved_note(&[]), "");
        assert_eq!(unsaved_note(&strings(&["The agent \"a\""])), "The agent \"a\" has changes that are not saved. They are lost when the app quits.");
        assert_eq!(
            unsaved_note(&strings(&["The skill \"s\"", "The agent \"a\""])),
            "These have changes that are not saved, which are lost when the app quits:\n  • The skill \"s\"\n  • The agent \"a\""
        );
    }

    #[test]
    fn asks_only_when_something_would_be_lost() {
        assert_eq!(quit_question(false, None, &[], &[]), None);
        assert_eq!(
            quit_question(true, None, &[], &[]),
            Some(QuitQuestion {
                message: "Sessions are still running".into(),
                detail: "Quitting ends the running Claude processes. Your tabs reopen next time and continue their conversations.".into(),
            })
        );
        assert_eq!(quit_question(true, Some(false), &[], &[]).unwrap().detail, "Quitting ends every open Claude session.");
    }

    #[test]
    fn tells_of_runs_and_drafts_after_the_sessions() {
        let question = quit_question(true, Some(true), &strings(&["Review", "Ship"]), &strings(&["The agent \"a\""])).unwrap();
        assert_eq!(question.message, "Sessions are still running");
        assert_eq!(
            question.detail,
            "Quitting ends the running Claude processes. Your tabs reopen next time and continue their conversations.\n\n\
             The workflow \"Review\", \"Ship\" ends here as well: a run does not go on after a restart.\n\n\
             The agent \"a\" has changes that are not saved. They are lost when the app quits."
        );
        let drafts = quit_question(false, None, &[], &strings(&["The skill \"s\""])).unwrap();
        assert_eq!(drafts.message, "There are unsaved changes");
        assert_eq!(drafts.detail, "The skill \"s\" has changes that are not saved. They are lost when the app quits.");
        let run = quit_question(false, None, &strings(&["Review"]), &[]).unwrap();
        assert_eq!(run.message, "A workflow is still running");
        assert_eq!(run.detail, "The workflow \"Review\" ends here as well: a run does not go on after a restart.");
    }
}
