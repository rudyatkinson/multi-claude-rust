//! `src/shared/memory.ts`: Claude Code's auto memory as the Library sees it.

use serde::{Deserialize, Serialize};

use super::SaveOutcome;

pub const MEMORY_INDEX: &str = "MEMORY.md";

pub struct MemoryLimits {
    /// What a session reads of MEMORY.md: this many lines ...
    pub index_lines: usize,
    /// ... or this many bytes, whichever comes first.
    pub index_bytes: usize,
    /// The name of a memory made here, which names its file.
    pub name: usize,
    pub title: usize,
    pub description: usize,
    /// Hits a search hands back.
    pub hits: usize,
}

pub const MEMORY_LIMITS: MemoryLimits = MemoryLimits { index_lines: 200, index_bytes: 25_000, name: 64, title: 120, description: 300, hits: 60 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

pub const MEMORY_TYPES: [MemoryType; 4] = [MemoryType::User, MemoryType::Feedback, MemoryType::Project, MemoryType::Reference];

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }
}

/// A memory folder: what one account (or several that share it) remembers of one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFolder {
    /// Stands for the folder; the window never names a path itself.
    pub id: String,
    /// The accounts whose own folder this is.
    pub profile_ids: Vec<String>,
    /// The accounts that use it through sharing instead of their own.
    pub shared_with: Vec<String>,
    /// The memory folder itself, for showing.
    pub dir: String,
    /// Memories in it, MEMORY.md not counted.
    pub count: u32,
    /// When a file in it last changed, epoch ms.
    pub updated_at: i64,
}

/// A project that has a memory in at least one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProject {
    /// Claude Code's name for the project: its path with a hyphen for every other character.
    pub key: String,
    /// The project's folder, when a conversation or a known folder tells it (`null` otherwise).
    pub cwd: Option<String>,
    pub folders: Vec<MemoryFolder>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListing {
    pub projects: Vec<MemoryProject>,
    /// Why no account can use the main account's memory right now, when that is so (`null` otherwise).
    pub share_problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRef {
    pub folder_id: String,
    /// The file within the memory folder.
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub file: String,
    /// What the index calls it, else the name in the file, else the file's name.
    pub title: String,
    /// The line of the index about it, else the description in the file.
    pub description: String,
    /// `user`, `feedback`, `project` or `reference`, when the file says; free text otherwise.
    pub r#type: String,
    pub updated_at: i64,
    pub size: u64,
    /// MEMORY.md has a line for it.
    pub indexed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryIssueLevel {
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIssue {
    pub level: MemoryIssueLevel,
    pub message: String,
}

/// One memory folder, opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryView {
    pub folder_id: String,
    pub dir: String,
    /// MEMORY.md is there.
    pub has_index: bool,
    pub index_lines: u32,
    pub index_bytes: u64,
    pub entries: Vec<MemoryEntry>,
    pub issues: Vec<MemoryIssue>,
}

/// `MemoryDetail extends MemoryRef`; the ref's fields are inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDetail {
    pub folder_id: String,
    pub file: String,
    pub text: String,
    pub hash: String,
    /// The file, for showing.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<String>,
}

pub type MemorySaveResult = SaveOutcome<MemoryDetail>;

/// `MemoryHit extends MemoryRef`; the ref's fields are inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHit {
    pub folder_id: String,
    pub file: String,
    /// The project's key, for showing where the hit is.
    pub key: String,
    pub cwd: Option<String>,
    pub profile_ids: Vec<String>,
    pub title: String,
    /// The text around the first word that was found.
    pub snippet: String,
}

/// The second argument of `memory:create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCreateInput {
    pub name: String,
    pub title: String,
    pub description: String,
    pub r#type: String,
}

/// A line of MEMORY.md that links a memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexLine {
    /// The line in MEMORY.md, counted from 0.
    pub line: u32,
    pub title: String,
    pub file: String,
    /// What stands after the link: when the memory is worth reading.
    pub hook: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptFile {
    pub name: String,
    /// Stands for the file's content.
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptCopy {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptPlan {
    /// Files that go over, under the name they get there.
    pub copy: Vec<AdoptCopy>,
    /// Files the target has already, with the same content.
    pub same: Vec<String>,
}

/// A plan, and whether it was carried out. `AdoptResult extends AdoptPlan`; inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptResult {
    pub copy: Vec<AdoptCopy>,
    pub same: Vec<String>,
    /// The folder the files go to, for showing.
    pub target: String,
    /// False for a look at what would happen.
    pub applied: bool,
}
