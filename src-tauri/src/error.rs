//! Tauri commands return `Result<T, CmdError>`. `CmdError` serializes as the plain message string,
//! which the renderer shows in a toast (the Electron version stripped an "Error invoking remote
//! method" prefix; here there is nothing to strip).

use mc_core::error::AppError;
use serde::Serialize;

#[derive(Debug)]
pub struct CmdError(pub String);

impl Serialize for CmdError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl From<AppError> for CmdError {
    fn from(e: AppError) -> Self {
        CmdError(e.to_string())
    }
}

impl From<std::io::Error> for CmdError {
    fn from(e: std::io::Error) -> Self {
        CmdError(e.to_string())
    }
}

impl From<String> for CmdError {
    fn from(s: String) -> Self {
        CmdError(s)
    }
}

impl From<&str> for CmdError {
    fn from(s: &str) -> Self {
        CmdError(s.to_string())
    }
}

impl From<tauri::Error> for CmdError {
    fn from(e: tauri::Error) -> Self {
        CmdError(e.to_string())
    }
}

pub type CmdResult<T> = Result<T, CmdError>;
