//! The hook types of `src/main/workflows/hooks.ts` and the entries of `limit-events.json`
//! (`src/main/limitLog.ts`). Types only; parsing a hook body is ported separately.

use serde::{Deserialize, Serialize};

use super::{AskWhat, LimitWaitStage, StopError};

/// The hook events a session posts to the app, as Claude Code names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    UserPromptSubmit,
    Stop,
    StopFailure,
    PreToolUse,
    PermissionRequest,
    Notification,
}

/// `HOOK_EVENTS`, in the TypeScript order.
pub const HOOK_EVENTS: [HookEvent; 6] = [
    HookEvent::UserPromptSubmit,
    HookEvent::Stop,
    HookEvent::StopFailure,
    HookEvent::PreToolUse,
    HookEvent::PermissionRequest,
    HookEvent::Notification,
];

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::Stop => "Stop",
            HookEvent::StopFailure => "StopFailure",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PermissionRequest => "PermissionRequest",
            HookEvent::Notification => "Notification",
        }
    }

    /// `isHookEvent`: the event of that name, if it is one of [`HOOK_EVENTS`].
    pub fn from_name(name: &str) -> Option<HookEvent> {
        HOOK_EVENTS.into_iter().find(|e| e.as_str() == name)
    }
}

/// Longer outputs are cut: they travel on as part of the next step's prompt.
pub const OUTPUT_LIMIT: usize = 1_000_000;

/// What a posted hook event says about its session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HookSignal {
    /// A message was sent: a turn begins.
    Prompt,
    /// Claude finished its answer. `background`: work it started is still running and will wake it again.
    Stop { text: String, background: bool },
    /// Claude presents a plan and waits for it to be approved.
    Plan { text: String },
    /// The turn ended with an API error. `text` is the beginning of what Claude Code showed for it.
    Failure { reason: String, error: StopError, text: String },
    /// Claude waits for the user in the tab.
    NeedsYou { what: AskWhat },
    /// Claude Code's own wait for a usage limit to reset is over.
    LimitWait { stage: LimitWaitStage },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookReport {
    /// Claude Code's id of the conversation the event belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub signal: HookSignal,
}

/// How many events `limit-events.json` keeps.
pub const LIMIT_LOG_KEEP: usize = 20;
/// How much of each text an entry keeps, in UTF-16 units.
pub const LIMIT_LOG_TEXT: usize = 300;

/// An entry of `limit-events.json`: a failed turn, or the end of Claude Code's wait for a usage
/// limit, as Claude Code reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitEvent {
    /// ISO 8601, as `Date.toISOString` writes it.
    pub at: String,
    pub account: String,
    pub event: HookEvent,
    /// The error's name as it was sent, also one this version does not know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// The kind of notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// `limit-events.json`: the last [`LIMIT_LOG_KEEP`] events, oldest first.
pub type LimitLog = Vec<LimitEvent>;
