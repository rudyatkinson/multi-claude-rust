//! The command surface. Naming rule: Electron channel `a:b` becomes Tauri command `a_b` in
//! snake_case (`profiles:create` → `profiles_create`, `app:getState` → `app_get_state`,
//! `skills:pickImport` → `skills_pick_import`); `src/api.ts` in the renderer does the same mapping.
//! Argument names are the camelCase keys listed in `INVOKE_PARAMS` of `src/api.ts`; Tauri maps them
//! onto snake_case Rust parameters (`baseHash` → `base_hash`, `ref` → `r#ref`). Send channels take a
//! single `payload` argument. Every command validates with `mc_core::validate` before touching a
//! service and returns `CmdResult<T>`; the error text is what the user reads.

pub mod app;
pub mod attention;
pub mod conversations;
pub mod explorer_menu;
pub mod layout;
pub mod mcp;
pub mod profiles;
pub mod sessions;
pub mod transfer;
pub mod usage;
pub mod workflows;
pub mod agents;
pub mod instructions;
pub mod memory;
pub mod skills;
pub mod updater;

/// Defines [`handler`] and the list of the names it registers from one list of commands.
macro_rules! registry {
    ($($module:ident :: $name:ident),* $(,)?) => {
        /// All commands registered with the Tauri builder: every channel of `INVOKE_CHANNELS` and
        /// `SEND_CHANNELS` of the renderer, plus `app_get_version`. `usage_refresh` is not
        /// registered: the renderer has no usage channel of its own.
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
    explorer_menu::explorer_menu_status,
    explorer_menu::explorer_menu_set,
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
    transfer::sessions_transfer,
    sessions::sessions_open_external,
    // conversations
    conversations::conversations_list,
    conversations::conversations_open,
    // workflows
    workflows::workflows_save,
    workflows::workflows_delete,
    workflows::workflows_run,
    workflows::workflows_approve,
    workflows::workflows_retry,
    workflows::workflows_stop,
    workflows::workflows_dismiss,
    workflows::workflows_take_answer,
    workflows::workflows_output,
    // Phase 3
    skills::skills_list,
    skills::skills_read,
    skills::skills_save,
    skills::skills_create,
    skills::skills_duplicate,
    skills::skills_rename,
    skills::skills_delete,
    skills::skills_transfer,
    skills::skills_export,
    skills::skills_pick_import,
    skills::skills_import,
    skills::skills_reveal,
    skills::skills_open_file,
    skills::skills_assist,
    skills::skills_add_folder,
    skills::skills_remove_folder,
    agents::agents_list,
    agents::agents_read,
    agents::agents_save,
    agents::agents_create,
    agents::agents_duplicate,
    agents::agents_rename,
    agents::agents_delete,
    agents::agents_transfer,
    agents::agents_reveal,
    agents::agents_add_folder,
    agents::agents_remove_folder,
    memory::memory_list,
    memory::memory_entries,
    memory::memory_read,
    memory::memory_save,
    memory::memory_create,
    memory::memory_delete,
    memory::memory_search,
    memory::memory_adopt,
    memory::memory_reveal,
    instructions::instructions_scopes,
    instructions::instructions_files,
    instructions::instructions_read,
    instructions::instructions_save,
    instructions::instructions_create,
    instructions::instructions_delete,
    instructions::instructions_view,
    instructions::instructions_reveal,
    instructions::instructions_add_folder,
    // Phase 4
    updater::updater_check,
    updater::updater_install,
    updater::updater_dismiss,
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

    #[test]
    fn the_updater_channels_are_the_renderers() {
        let ipc = include_str!("../../../src/shared/ipc.ts");
        let list = ipc.split("export const INVOKE_CHANNELS = [").nth(1).and_then(|rest| rest.split(']').next()).expect("INVOKE_CHANNELS in ipc.ts");
        for channel in crate::updater::INVOKE_CHANNELS {
            let quoted = format!("'{channel}'");
            assert!(ipc.contains(&format!("{quoted}: (")), "{channel} is not a channel of IpcInvoke");
            assert!(list.contains(&quoted), "{channel} is not in INVOKE_CHANNELS");
        }
    }
}
