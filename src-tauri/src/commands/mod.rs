//! The command surface. Naming rule: Electron channel `a:b` becomes Tauri command `a_b` in
//! snake_case (`profiles:create` → `profiles_create`, `app:getState` → `app_get_state`,
//! `skills:pickImport` → `skills_pick_import`); `src/api.ts` in the renderer does the same mapping.
//! Argument names are the camelCase keys listed in `INVOKE_PARAMS` of `src/api.ts`; Tauri maps them
//! onto snake_case Rust parameters (`baseHash` → `base_hash`, `ref` → `r#ref`). Send channels take a
//! single `payload` argument. Every command validates with `mc_core::validate` before touching a
//! service and returns `CmdResult<T>`; the error text is what the user reads.

pub mod app;
pub mod attention;
pub mod layout;
pub mod mcp;
pub mod profiles;
pub mod sessions;
pub mod stubs;
pub mod usage;
pub mod agents;
pub mod instructions;
pub mod memory;
pub mod skills;

/// Defines [`handler`] and the list of the names it registers from one list of commands.
macro_rules! registry {
    ($($module:ident :: $name:ident),* $(,)?) => {
        /// All commands registered with the Tauri builder: every channel of `INVOKE_CHANNELS` and
        /// `SEND_CHANNELS` of the renderer, plus `app_get_version`. Commands of later phases are
        /// stubs (`stubs`) until their packages replace them. `usage_refresh` is not registered:
        /// the renderer has no usage channel of its own.
        pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$($module::$name),*]
        }

        /// The names [`handler`] registers.
        #[cfg(test)]
        pub(crate) const REGISTERED: &[&str] = &[$(stringify!($name)),*];
    };
}

registry![
    // app
    app::app_get_version,
    app::app_get_state,
    app::app_ready,
    app::settings_update,
    app::shell_open_external,
    app::clipboard_read,
    app::clipboard_write_text,
    app::dialog_pick_folder,
    app::fs_folder_for,
    app::explorer_menu_status,
    app::explorer_menu_set,
    app::ui_unsaved,
    // profiles
    profiles::profiles_create,
    profiles::profiles_update,
    profiles::profiles_delete,
    profiles::profiles_login,
    profiles::profiles_logout,
    profiles::profiles_set_token,
    profiles::profiles_refresh_status,
    profiles::profiles_sync_from_main,
    profiles::profiles_open_config_dir,
    profiles::folders_set_account,
    // sessions
    sessions::sessions_create,
    sessions::sessions_attach,
    sessions::sessions_close,
    sessions::sessions_close_many,
    sessions::sessions_rename,
    sessions::sessions_restart,
    sessions::sessions_reveal_folder,
    sessions::pty_write,
    sessions::pty_resize,
    layout::ui_layout,
    attention::session_attention,
    // mcp
    mcp::mcp_list,
    mcp::mcp_copy,
    // Phase 2
    stubs::sessions_transfer,
    stubs::sessions_open_external,
    stubs::conversations_list,
    stubs::conversations_open,
    stubs::workflows_save,
    stubs::workflows_delete,
    stubs::workflows_run,
    stubs::workflows_approve,
    stubs::workflows_retry,
    stubs::workflows_stop,
    stubs::workflows_dismiss,
    stubs::workflows_take_answer,
    stubs::workflows_output,
    // Phase 3
    stubs::skills_list,
    stubs::skills_read,
    stubs::skills_save,
    stubs::skills_create,
    stubs::skills_duplicate,
    stubs::skills_rename,
    stubs::skills_delete,
    stubs::skills_transfer,
    stubs::skills_export,
    stubs::skills_pick_import,
    stubs::skills_import,
    stubs::skills_reveal,
    stubs::skills_open_file,
    stubs::skills_assist,
    stubs::skills_add_folder,
    stubs::skills_remove_folder,
    stubs::agents_list,
    stubs::agents_read,
    stubs::agents_save,
    stubs::agents_create,
    stubs::agents_duplicate,
    stubs::agents_rename,
    stubs::agents_delete,
    stubs::agents_transfer,
    stubs::agents_reveal,
    stubs::agents_add_folder,
    stubs::agents_remove_folder,
    stubs::memory_list,
    stubs::memory_entries,
    stubs::memory_read,
    stubs::memory_save,
    stubs::memory_create,
    stubs::memory_delete,
    stubs::memory_search,
    stubs::memory_adopt,
    stubs::memory_reveal,
    stubs::instructions_scopes,
    stubs::instructions_files,
    stubs::instructions_read,
    stubs::instructions_save,
    stubs::instructions_create,
    stubs::instructions_delete,
    stubs::instructions_view,
    stubs::instructions_reveal,
    stubs::instructions_add_folder,
];

#[cfg(test)]
mod tests {
    use mc_core::types::{INVOKE_CHANNELS, SEND_CHANNELS};

    /// `commandName` of `src/api.ts`: `:` becomes `_`, a capital becomes `_` and its lower case.
    fn command_name(channel: &str) -> String {
        let mut out = String::new();
        for c in channel.chars() {
            match c {
                ':' => out.push('_'),
                c if c.is_ascii_uppercase() => {
                    out.push('_');
                    out.push(c.to_ascii_lowercase());
                }
                c => out.push(c),
            }
        }
        out
    }

    #[test]
    fn every_channel_of_the_renderer_has_a_command() {
        let mut expected: Vec<String> = INVOKE_CHANNELS.iter().chain(SEND_CHANNELS.iter()).map(|c| command_name(c)).collect();
        expected.push("app_get_version".into());
        expected.sort();
        let mut registered: Vec<String> = super::REGISTERED.iter().map(|s| (*s).to_owned()).collect();
        registered.sort();
        assert_eq!(registered, expected);
    }
}
