//! `src/shared/turn.ts`: what a session's turn has come to. Types only; the state machine
//! (`nextTurn`, `tabMark`, ...) is ported separately.

use serde::{Deserialize, Serialize};

use super::serde_util::is_false;

/// The API errors a turn can end with, as Claude Code names them in its `StopFailure` hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopError {
    RateLimit,
    Overloaded,
    AuthenticationFailed,
    OauthOrgNotAllowed,
    AccountOnHold,
    BillingError,
    InvalidRequest,
    ModelNotFound,
    ServerError,
    MaxOutputTokens,
    CloudCredentialError,
    Unknown,
}

/// `STOP_ERRORS`, in the TypeScript order.
pub const STOP_ERRORS: [StopError; 12] = [
    StopError::RateLimit,
    StopError::Overloaded,
    StopError::AuthenticationFailed,
    StopError::OauthOrgNotAllowed,
    StopError::AccountOnHold,
    StopError::BillingError,
    StopError::InvalidRequest,
    StopError::ModelNotFound,
    StopError::ServerError,
    StopError::MaxOutputTokens,
    StopError::CloudCredentialError,
    StopError::Unknown,
];

impl StopError {
    /// The name as Claude Code sends it.
    pub fn as_str(self) -> &'static str {
        match self {
            StopError::RateLimit => "rate_limit",
            StopError::Overloaded => "overloaded",
            StopError::AuthenticationFailed => "authentication_failed",
            StopError::OauthOrgNotAllowed => "oauth_org_not_allowed",
            StopError::AccountOnHold => "account_on_hold",
            StopError::BillingError => "billing_error",
            StopError::InvalidRequest => "invalid_request",
            StopError::ModelNotFound => "model_not_found",
            StopError::ServerError => "server_error",
            StopError::MaxOutputTokens => "max_output_tokens",
            StopError::CloudCredentialError => "cloud_credential_error",
            StopError::Unknown => "unknown",
        }
    }

    /// `isStopError`: the error of that name, if it is one of [`STOP_ERRORS`].
    pub fn from_name(name: &str) -> Option<StopError> {
        STOP_ERRORS.into_iter().find(|e| e.as_str() == name)
    }
}

/// How Claude Code's own wait for a usage limit ended, when it did not simply go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnWait {
    NeedsEnter,
    Ended,
}

/// What a session's turn has come to, as its hooks report it. `at` is when it got there.
/// `{ at: number } & (...)` in the TypeScript: the union is flattened next to `at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub at: i64,
    #[serde(flatten)]
    pub kind: TurnKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TurnKind {
    Working,
    /// Claude finished its answer. `background`: work it started still runs and will wake it.
    Done {
        #[serde(default, skip_serializing_if = "is_false")]
        background: bool,
    },
    /// Claude waits for an answer in the tab.
    Permission,
    Question,
    Plan,
    /// The turn ended with an API error. `limit`: a usage limit of the account cut it off.
    /// `wait`: how Claude Code's own wait for that limit ended, when it did not simply go on.
    Failed {
        error: StopError,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        limit: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<TurnWait>,
    },
}

/// What Claude waits for: a permission or an answer to a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AskWhat {
    Permission,
    Question,
}

/// How Claude Code's wait for a usage limit to reset is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitWaitStage {
    Resumed,
    NeedsEnter,
    Ended,
}

/// What can change a turn: the signals of the session's hooks, and the user typing into its tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TurnEvent {
    Prompt,
    Stop { background: bool },
    Plan,
    NeedsYou { what: AskWhat },
    Failure { error: StopError, text: String },
    LimitWait { stage: LimitWaitStage },
    Typed,
}

/// The mark of a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabMark {
    Busy,
    Idle,
    Ask,
    Failed,
    Limit,
    Ended,
    /// The tab sleeps: no process, woken by a click or a key.
    Asleep,
}
