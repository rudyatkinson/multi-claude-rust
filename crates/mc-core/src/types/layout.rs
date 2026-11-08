//! `src/shared/layout.ts` (the types and the saved form) and `src/main/tabStore.ts` (`tabs.json`).

use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::serde_util::{js_number, minus_one, ok_or_none};

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
    #[serde(serialize_with = "js_number")]
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
        #[serde(serialize_with = "js_number")]
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

/// A panel of the app that sits in a tab like a session: its tab id is `panel:<name>`
/// ([`PANEL_PREFIX`](crate::layout::PANEL_PREFIX)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelKind {
    /// The workflow designer.
    Workflows,
    /// Skills, agents, memory and instructions.
    Library,
}

impl PanelKind {
    pub const ALL: [PanelKind; 2] = [PanelKind::Workflows, PanelKind::Library];

    /// What follows `panel:` in its tab id, and what `tabs.json` calls it.
    pub fn name(self) -> &'static str {
        match self {
            PanelKind::Workflows => "workflows",
            PanelKind::Library => "library",
        }
    }

    /// The id of its tab in the layout.
    pub fn tab_id(self) -> String {
        format!("{}{}", crate::layout::PANEL_PREFIX, self.name())
    }

    /// The panel a tab id stands for; None for a session, and for a panel this version does not know.
    pub fn of_tab(id: &str) -> Option<PanelKind> {
        let name = id.strip_prefix(crate::layout::PANEL_PREFIX)?;
        PanelKind::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// An open session's tab as remembered between runs of the app (see [`SavedEntry`]).
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

/// An entry of `tabs.json`:
/// `{ kind?: 'session', profileId, cwd, … } | { kind: 'panel', panel: 'workflows' | 'library' }`.
///
/// An entry without `kind` is a session, as every entry of the files of older versions is; a
/// session is written without it, so that those versions still read it. They drop a panel's entry
/// (it has no account or folder), and the layout follows the entries that stay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedEntry {
    Session(SavedTab),
    Panel(PanelKind),
}

impl SavedEntry {
    pub fn session(&self) -> Option<&SavedTab> {
        match self {
            SavedEntry::Session(tab) => Some(tab),
            SavedEntry::Panel(_) => None,
        }
    }

    pub fn panel(&self) -> Option<PanelKind> {
        match self {
            SavedEntry::Session(_) => None,
            SavedEntry::Panel(kind) => Some(*kind),
        }
    }
}

impl From<SavedTab> for SavedEntry {
    fn from(tab: SavedTab) -> Self {
        SavedEntry::Session(tab)
    }
}

impl Serialize for SavedEntry {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            SavedEntry::Session(tab) => tab.serialize(s),
            SavedEntry::Panel(kind) => {
                let mut map = s.serialize_map(Some(2))?;
                map.serialize_entry("kind", "panel")?;
                map.serialize_entry("panel", kind)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for SavedEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        let session = |value: Value| SavedTab::deserialize(value).map(SavedEntry::Session).map_err(D::Error::custom);
        match value.get("kind") {
            None => session(value),
            Some(Value::String(kind)) if kind == "session" => session(value),
            Some(Value::String(kind)) if kind == "panel" => {
                let panel = value.get("panel").ok_or_else(|| D::Error::missing_field("panel"))?;
                PanelKind::deserialize(panel).map(SavedEntry::Panel).map_err(D::Error::custom)
            }
            Some(other) => Err(D::Error::custom(format!("unknown kind of tab: {other}"))),
        }
    }
}

/// The whole of `tabs.json`. Typed reading is strict about the tabs; `loadTabs` in the Electron
/// version dropped single malformed entries, which is the loader's job. A layout that does not
/// have the right shape is dropped here, as `parseSavedLayout` dropped it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTabs {
    /// Sessions and panels, in the order the layout shows them.
    pub tabs: Vec<SavedEntry>,
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
    /// The sessions to start.
    pub requests: Vec<RestoreRequest>,
    /// Index of the tab that was in front, counted as in `layout`; -1 for none.
    pub active: i64,
    /// The saved arrangement. Tab `i` is `requests[i]`; tab `requests.len() + j` is `panels[j]`.
    /// Missing for files without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<SavedLayout>,
    /// The panels among the saved tabs, each once; nothing has to be started for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<PanelKind>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_saved_entry_is_a_session_unless_it_says_it_is_a_panel() {
        let session: SavedEntry = serde_json::from_value(json!({ "profileId": "main", "cwd": "C:\\a" })).unwrap();
        assert_eq!(session.session().map(|t| t.cwd.as_str()), Some("C:\\a"));
        assert_eq!(session.panel(), None);
        // written as older versions wrote it: without a kind
        assert_eq!(serde_json::to_value(&session).unwrap(), json!({ "profileId": "main", "cwd": "C:\\a" }));
        let tagged: SavedEntry = serde_json::from_value(json!({ "kind": "session", "profileId": "main", "cwd": "C:\\a" })).unwrap();
        assert_eq!(tagged, session);

        let panel: SavedEntry = serde_json::from_value(json!({ "kind": "panel", "panel": "library" })).unwrap();
        assert_eq!(panel, SavedEntry::Panel(PanelKind::Library));
        assert_eq!(serde_json::to_value(&panel).unwrap(), json!({ "kind": "panel", "panel": "library" }));

        // a panel this version does not know, a panel without its name, a kind of tab that does not exist
        assert!(serde_json::from_value::<SavedEntry>(json!({ "kind": "panel", "panel": "usage" })).is_err());
        assert!(serde_json::from_value::<SavedEntry>(json!({ "kind": "panel" })).is_err());
        assert!(serde_json::from_value::<SavedEntry>(json!({ "kind": "terminal", "profileId": "main", "cwd": "C:\\a" })).is_err());
    }

    #[test]
    fn a_panel_knows_its_tab() {
        assert_eq!(PanelKind::Workflows.tab_id(), "panel:workflows");
        assert_eq!(PanelKind::Library.tab_id(), "panel:library");
        assert_eq!(PanelKind::of_tab("panel:library"), Some(PanelKind::Library));
        assert_eq!(PanelKind::of_tab("panel:usage"), None);
        assert_eq!(PanelKind::of_tab("library"), None);
    }
}
