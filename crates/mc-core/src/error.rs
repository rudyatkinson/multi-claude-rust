use thiserror::Error;

/// The one error type the backend hands to the renderer. Its `Display` text is what the user reads
/// in a toast, so every variant carries a complete sentence.
#[derive(Debug, Error)]
pub enum AppError {
    /// The caller sent something that does not pass validation.
    #[error("{0}")]
    Invalid(String),
    /// A precondition is not met (no claude binary, account not found, ...).
    #[error("{0}")]
    Refused(String),
    /// A filesystem or process failure, already worded for the user.
    #[error("{0}")]
    Io(String),
}

impl AppError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        AppError::Invalid(msg.into())
    }
    pub fn refused(msg: impl Into<String>) -> Self {
        AppError::Refused(msg.into())
    }
    pub fn io(msg: impl Into<String>) -> Self {
        AppError::Io(msg.into())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
