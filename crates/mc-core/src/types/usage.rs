//! `src/shared/usage.ts` and `src/shared/autoSwitch.ts` (types and `SWITCH_RULES`), and the shape
//! of `usage-cache.json` (`src/main/usage/index.ts`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{AuthKind, Turn, UsageState};

/// `usage-cache.json`: profile id to the last known usage of that account.
pub type UsageCache = BTreeMap<String, UsageState>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageTone {
    Ok,
    Amber,
    Red,
}

/// How a usage window is shown at a moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageView {
    /// Whole percent to show; 0 once the window has reset.
    pub pct: i64,
    pub tone: UsageTone,
    /// The reset time has passed, so this is a fresh window whatever was last reported.
    pub reset: bool,
}

/// A limit that looks used up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedUp {
    /// "5-hour limit", "weekly limit", "Fable weekly limit".
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub profile_id: String,
    /// Its 5-hour use in whole percent, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour_pct: Option<i64>,
}

// ---------- moving a conversation by itself ----------

/// The limits within which a conversation is moved to another account without the user.
pub struct SwitchRules {
    /// A failed turn older than this is not acted on any more.
    pub fresh_ms: i64,
    /// How far back the moves of a conversation count: the length of a 5-hour limit.
    pub memory_ms: i64,
    /// How often a conversation is moved within that time.
    pub max_moves: usize,
    /// How long after a move the same conversation is not moved again.
    pub gap_ms: i64,
    /// How long an account that just took a conversation takes no other.
    pub arrival_gap_ms: i64,
    /// How full the account to move to may be, in percent.
    pub five_hour_pct: i64,
    pub week_pct: i64,
}

pub const SWITCH_RULES: SwitchRules = SwitchRules {
    fresh_ms: 2 * 60_000,
    memory_ms: 5 * 3_600_000,
    max_moves: 2,
    gap_ms: 10 * 60_000,
    arrival_gap_ms: 2 * 60_000,
    five_hour_pct: 80,
    week_pct: 94,
};

/// A move the app made by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchMove {
    pub conversation_id: String,
    pub from: String,
    pub to: String,
    pub at: i64,
}

/// `{ loggedIn }`: as much of an account's status as a decision needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginState {
    pub logged_in: bool,
}

/// `SwitchInput['session']`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchSession {
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moving: Option<bool>,
    /// A step of a workflow run: the run decides what becomes of it.
    pub workflow: bool,
}

/// An entry of `SwitchInput['profiles']`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchProfile {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<LoginState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchInput {
    /// The setting is on, and Claude can be handed a first message.
    pub enabled: bool,
    pub now: i64,
    pub session: SwitchSession,
    /// When the failed turn that asked for this decision got there.
    pub failed_at: i64,
    /// Claude was at work when the turn failed.
    pub was_busy: bool,
    /// Something was typed into the tab since.
    pub typed: bool,
    pub profiles: Vec<SwitchProfile>,
    pub usage: BTreeMap<String, UsageState>,
    /// The accounts whose usage was asked for anew for this decision, and whether numbers came.
    pub probed: BTreeMap<String, bool>,
    pub moves: Vec<SwitchMove>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StayWhy {
    Moved,
    Full,
}

/// What to do about a turn that a usage limit seems to have cut off. Told apart by `action`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum SwitchDecision {
    Move {
        to: String,
        limit: String,
    },
    /// Ask for this account's usage, then decide again.
    Probe {
        #[serde(rename = "profileId")]
        profile_id: String,
    },
    /// The limit is real, but the conversation stays: say so.
    Stay { why: StayWhy, limit: String },
    /// `{ action: 'none', why }`; not named `None` so that it never shadows `Option::None`.
    #[serde(rename = "none")]
    Nothing { why: String },
}
