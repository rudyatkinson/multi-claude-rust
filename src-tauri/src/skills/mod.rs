//! The skill manager (`src/main/skills/*`): the skills of every account and of the projects in
//! sight, read and saved with conflict detection, made, copied, renamed, moved, deleted into the
//! bin, packed into and brought in from archives, and handed to Claude in a session ("Ask Claude").
//!
//! [`scan`] finds the skills of a folder, [`files`] does the file-system work, [`store`] is the
//! service, [`archive`] packs and unpacks zips, [`assist`] opens the session. Where the skills
//! folders are is [`crate::library::roots`].

pub mod archive;
pub mod assist;
pub mod files;
pub mod scan;
pub mod store;
#[cfg(test)]
mod tests;

pub use store::{AssistRequest, Assisted, FileAction, SkillDeps, SkillService};
