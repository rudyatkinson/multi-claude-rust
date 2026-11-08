//! `src/shared/conversations.ts`: the past conversations of the accounts.

use serde::{Deserialize, Serialize};

/// A past conversation of an account, as much of it as a list needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationEntry {
    pub profile_id: String,
    /// Claude Code's id of the conversation. The same id can exist in two accounts after a move.
    pub id: String,
    /// The folder it took place in.
    pub cwd: String,
    /// Claude Code's title for it, else the start of its first message.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt: Option<String>,
    /// When it last changed, epoch ms (whole ms; the Electron version passed a fractional `mtimeMs`).
    pub updated_at: i64,
}
