//! Multi Claude backend. This crate owns everything the Electron main process did: pty sessions,
//! the hook server, profiles, usage, workflows, the Library services, tray and OS integration.
//! Pure logic lives in `mc-core`; this crate is the I/O around it plus the Tauri command surface.

pub mod commands;
pub mod error;
pub mod events;
pub mod state;

use tauri::Manager;

/// Builds and runs the Tauri application. Called from `main.rs`.
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            // A second instance passed its arguments to us; Phase 1 integration turns them into
            // --open-folder / --quit handling. For now: bring the window to the front.
            log::info!("second instance: argv={argv:?} cwd={cwd}");
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .setup(|app| {
            let state = state::AppState::new(app.handle().clone())?;
            app.manage(state);
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
            }
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("error while running Multi Claude");
}
