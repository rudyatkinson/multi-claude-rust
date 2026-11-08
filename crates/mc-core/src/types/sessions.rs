//! The request types of `src/main/sessions.ts`: what a session is asked to be. They stay in the
//! backend, but restarts, moves, workflow steps and restored tabs all build them.

use serde::{Deserialize, Serialize};

use super::{ResumeConversation, ResumeMode, SessionKind};

/// Whether a session reports its turns to the app: not at all, when it can, or it must and does
/// not start otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookNeed {
    /// `'none'`; not named `None` so that it never shadows `Option::None`.
    #[serde(rename = "none")]
    Off,
    Wanted,
    Required,
}

/// How a session's Claude is started beyond the defaults. It stays with the tab through a restart
/// or a move to another account. The values are checked where the arguments are built.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    /// An alias or a full model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Folders outside the working folder that Claude may read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_dirs: Option<Vec<String>>,
    /// The session must report its turns to the app, or not start (a workflow step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<bool>,
}

/// `ResumeMode | { id: string }`: a mode, or the one conversation to resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SessionResume {
    Mode(ResumeMode),
    Conversation(ResumeConversation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRequest {
    pub profile_id: String,
    pub cwd: String,
    pub kind: SessionKind,
    /// Missing for a new conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<SessionResume>,
    /// The user's own name for the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchOptions>,
    /// Sent as the session's first message. It is not kept: a restarted or restored tab does not say it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}
