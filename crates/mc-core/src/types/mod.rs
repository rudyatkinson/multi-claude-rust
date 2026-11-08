//! Domain types shared with the renderer, and the shapes of the files the app keeps.
//!
//! Field names mirror the TypeScript of the Electron version (`src/shared/*.ts` and a few files of
//! `src/main`) exactly on the wire: camelCase, optional fields left out when missing, string unions
//! as enums with the same spelling. The React renderer keeps its TypeScript types unchanged, so a
//! change here is a change of the protocol.
//!
//! Each submodule follows one source file; everything is re-exported here, so other code names
//! types as `mc_core::types::X`.

mod serde_util;

pub mod agents;
pub mod app;
pub mod config;
pub mod conversations;
pub mod hooks;
pub mod instructions;
pub mod ipc;
pub mod layout;
pub mod memory;
pub mod sessions;
pub mod skills;
pub mod turn;
pub mod usage;
pub mod workflows;

pub use agents::*;
pub use app::*;
pub use config::*;
pub use conversations::*;
pub use hooks::*;
pub use instructions::*;
pub use ipc::*;
pub use layout::*;
pub use memory::*;
pub use sessions::*;
pub use skills::*;
pub use turn::*;
pub use usage::*;
pub use workflows::*;

/// Unknown keys of an object that are kept as they were and written back.
pub type Extra = std::collections::BTreeMap<String, serde_json::Value>;

#[cfg(test)]
mod tests;
