//! `src/shared/layout.ts` (the types and the saved form) and `src/main/tabStore.ts` (`tabs.json`).

use serde::{Deserialize, Serialize};

use super::serde_util::{minus_one, ok_or_none};

/// `row`: the first child is left of the second; `column`: it is above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Dir {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// An area of the window with its own tab strip. It shows its active tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    /// Session ids, in tab order.
    pub tabs: Vec<String>,
    /// `null` on the wire for an empty group.
    pub active: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Split {
    pub id: String,
    pub dir: Dir,
    /// Share of the first child, 0..1.
    pub ratio: f64,
    pub first: Box<Node>,
    pub second: Box<Node>,
}

/// `Group | Split`, told apart by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Node {
    Group(Group),
    Split(Split),
}

/// How the sessions are arranged in the window: a tree of areas, one of which has the keyboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub root: Node,
    /// Id of the group that has the keyboard.
    pub focused: String,
    /// Last number used for a node id.
    pub seq: u64,
}

impl Default for Layout {
    /// `emptyLayout()`: one empty group.
    fn default() -> Self {
        Layout {
            root: Node::Group(Group { id: "g1".into(), tabs: Vec::new(), active: None }),
            focused: "g1".into(),
            seq: 1,
        }
    }
}

/// Where a new session's tab should appear.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// Right after this session's tab, in its group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub id: String,
    /// The session this one took the place of (a restart, a move to another account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
}

pub const RATIO_MIN: f64 = 0.05;
pub const RATIO_MAX: f64 = 0.95;
/// Deeper trees than this are refused when they come from outside.
pub const LAYOUT_MAX_DEPTH: usize = 32;

// ---------- the saved form ----------

/// A layout as it is saved: tabs are positions in the saved tab list instead of session ids.
/// `{ tabs, active } | { dir, ratio, first, second }`, told apart by their keys (`'tabs' in node`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SavedNode {
    Group {
        tabs: Vec<i64>,
        /// -1 for none.
        active: i64,
    },
    Split {
        dir: Dir,
        ratio: f64,
        first: Box<SavedNode>,
        second: Box<SavedNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLayout {
    pub root: SavedNode,
    /// Position of the group with the keyboard, counting the groups in reading order.
    pub focused: u32,
}

// ---------- tabs.json ----------

/// An open tab as remembered between runs of the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTab {
    pub profile_id: String,
    pub cwd: String,
    /// Claude Code's id of the tab's conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    /// Where Claude Code keeps that conversation, when it has told us.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    /// The user's own name for the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The whole of `tabs.json`. Typed reading is strict about the tabs; `loadTabs` in the Electron
/// version dropped single malformed entries, which is the loader's job. A layout that does not
/// have the right shape is dropped here, as `parseSavedLayout` dropped it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTabs {
    pub tabs: Vec<SavedTab>,
    /// Index of the tab that was in front; -1 for none.
    #[serde(default = "minus_one")]
    pub active: i64,
    /// How the tabs were arranged in areas; files of older versions have none.
    #[serde(default, deserialize_with = "ok_or_none", skip_serializing_if = "Option::is_none")]
    pub layout: Option<SavedLayout>,
}

impl Default for SavedTabs {
    /// `NONE`: no tabs, none in front.
    fn default() -> Self {
        SavedTabs { tabs: Vec::new(), active: -1, layout: None }
    }
}

/// `{ id }`: the one conversation to resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeConversation {
    pub id: String,
}

/// A session to start for a saved tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    pub profile_id: String,
    pub cwd: String,
    /// Missing for a tab that never got a conversation: it starts fresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeConversation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePlan {
    pub requests: Vec<RestoreRequest>,
    /// Index into `requests` of the tab that was in front; -1 for none.
    pub active: i64,
    /// The saved arrangement, in terms of `requests`. Missing for files without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<SavedLayout>,
}
