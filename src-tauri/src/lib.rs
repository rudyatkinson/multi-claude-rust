//! Multi Claude backend. This crate owns everything the Electron main process did: pty sessions,
//! the hook server, profiles, usage, workflows, the Library services, tray and OS integration.
//! Pure logic lives in `mc-core`; this crate is the I/O around it plus the Tauri command surface.

pub mod app;
pub mod commands;
pub mod error;
pub mod events;
pub mod state;

// Phase 1 packages
pub mod claude_binary;
pub mod claude_cli;
pub mod config_store;
pub mod hook_server;
pub mod launchers;
pub mod limit_log;
pub mod mcp_servers;
pub mod paths;
pub mod profiles;
pub mod session_alerts;
pub mod sessions;
pub mod sharing;
pub mod tab_store;
pub mod token_vault;
pub mod usage;

// Phase 2 packages
pub mod auto_switch;
pub mod conversations;
pub mod explorer_menu;
pub mod external_terminal;
pub mod transfer;
pub mod workflows;

// Phase 3 packages
pub mod agents;
pub mod instructions;
pub mod library;
pub mod memory;
pub mod skills;

// Phase 4 packages
pub mod tray;
pub mod updater;

/// Builds and runs the Tauri application. Called from `main.rs`.
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let (root, isolated) = match state::app_root() {
        Ok(found) => found,
        Err(e) => {
            log::error!("Multi Claude cannot start: {e}");
            std::process::exit(1);
        }
    };
    let startup = app::boot::startup_args();
    let mut context = tauri::generate_context!();
    if isolated {
        app::boot::isolate(&mut context, &root);
    }

    let app = tauri::Builder::default()
        // first, so that a second start hands over its arguments before anything else happens
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| app::boot::second_instance(app, argv)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(move |app| app::boot::setup(app, root, startup))
        .on_page_load(app::boot::on_page_load)
        .on_window_event(app::window::on_window_event)
        .invoke_handler(commands::handler())
        .build(context);
    match app {
        Ok(app) => app.run(app::boot::on_run_event),
        Err(e) => {
            log::error!("Multi Claude cannot start: {e}");
            std::process::exit(1);
        }
    }
}
