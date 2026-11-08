//! Pure logic shared by the Multi Claude backend: domain types, validation and the algorithms
//! that were in `src/shared` and the testable parts of `src/main` of the Electron version.
//! Nothing in this crate touches a window, a pty, a process or the network.

pub mod error;
pub mod jsstr;
pub mod types;
pub mod validate;

// Phase 1 packages (each module is owned by one work package; see the plan)
pub mod attention;
pub mod auth_status;
pub mod claude_args;
pub mod duration;
pub mod env;
pub mod folders;
pub mod hooks;
pub mod layout;
pub mod open_folder_args;
pub mod open_target;
pub mod settings_merge;
pub mod slug;
pub mod tab_order;
pub mod terminal_input;
pub mod terminal_title;
pub mod tray_model;
pub mod turn;
pub mod usage;

// Phase 2 and 3 packages
pub mod auto_switch;
pub mod conversations;
pub mod frontmatter;
pub mod paths_input;
pub mod text_file;
pub mod transfer;
pub mod workflow;
