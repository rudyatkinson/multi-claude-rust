//! Commands of later phases, registered so that the renderer gets a message it shows as a toast
//! instead of an unknown command. Each one is replaced by its package's real command.
//!
//! - Phase 3: the Library (`skills:*`, `agents:*`, `memory:*`, `instructions:*`).
//!
//! A stub takes no arguments: Tauri ignores the ones the renderer sends.

use crate::error::{CmdError, CmdResult};

pub const NOT_YET: &str = "Not available yet in this build.";

macro_rules! not_yet {
    ($($name:ident),* $(,)?) => {
        $(
            #[tauri::command]
            pub fn $name() -> CmdResult<()> {
                Err(CmdError(NOT_YET.into()))
            }
        )*
    };
}

not_yet!(
    skills_list,
    skills_read,
    skills_save,
    skills_create,
    skills_duplicate,
    skills_rename,
    skills_delete,
    skills_transfer,
    skills_export,
    skills_pick_import,
    skills_import,
    skills_reveal,
    skills_open_file,
    skills_assist,
    skills_add_folder,
    skills_remove_folder,
    agents_list,
    agents_read,
    agents_save,
    agents_create,
    agents_duplicate,
    agents_rename,
    agents_delete,
    agents_transfer,
    agents_reveal,
    agents_add_folder,
    agents_remove_folder,
    memory_list,
    memory_entries,
    memory_read,
    memory_save,
    memory_create,
    memory_delete,
    memory_search,
    memory_adopt,
    memory_reveal,
    instructions_scopes,
    instructions_files,
    instructions_read,
    instructions_save,
    instructions_create,
    instructions_delete,
    instructions_view,
    instructions_reveal,
    instructions_add_folder,
);
