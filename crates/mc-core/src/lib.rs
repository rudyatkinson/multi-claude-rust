//! Pure logic shared by the Multi Claude backend: domain types, validation and the algorithms
//! that were in `src/shared` and the testable parts of `src/main` of the Electron version.
//! Nothing in this crate touches a window, a pty, a process or the network.

pub mod error;
pub mod jsstr;
pub mod types;
pub mod validate;
