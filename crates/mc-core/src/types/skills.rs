//! `src/shared/skills.ts` (the skill manager's types and limits) and the form field types of
//! `src/shared/frontmatter.ts`. Also [`SaveOutcome`], the `{ saved } | { conflict }` shape every
//! editor of the Library shares.

use serde::{Deserialize, Serialize};

use super::serde_util::is_false;

pub struct SkillLimits {
    /// Name length the Agent Skills format allows.
    pub name: usize,
    /// Description length claude.ai accepts on upload.
    pub description: usize,
    /// What Claude Code keeps of description plus when_to_use in its listing.
    pub listing: usize,
    pub compatibility: usize,
    /// Anthropic's advice for the length of SKILL.md, in lines.
    pub lines: usize,
    /// A SKILL.md larger than this is not opened in the editor.
    pub file_bytes: usize,
    /// Length of a request to Claude about a skill.
    pub request: usize,
}

pub const SKILL_LIMITS: SkillLimits = SkillLimits {
    name: 64,
    description: 1024,
    listing: 1536,
    compatibility: 500,
    lines: 500,
    file_bytes: 1_000_000,
    request: 20_000,
};

/// The frontmatter keys of the two switches: who may start the skill.
pub struct SkillSwitches {
    /// `true` keeps Claude from using the skill by itself.
    pub hide_from_model: &'static str,
    /// `false` takes the skill out of the / menu.
    pub user_invocable: &'static str,
}

pub const SKILL_SWITCHES: SkillSwitches = SkillSwitches { hide_from_model: "disable-model-invocation", user_invocable: "user-invocable" };

/// The keys claude.ai accepts in an uploaded skill; any other key makes it refuse the upload.
pub const UPLOAD_KEYS: [&str; 6] = ["name", "description", "license", "compatibility", "metadata", "allowed-tools"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillRootKind {
    Personal,
    Project,
}

/// A folder that holds skills (and, as `AgentRoot`, subagents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRoot {
    pub id: String,
    pub kind: SkillRootKind,
    /// The skills folder itself, for showing.
    pub dir: String,
    /// Personal: the accounts whose skills folder this is (several when they share it).
    pub profile_ids: Vec<String>,
    /// Project: the folder whose `.claude/skills` this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The skills folder is there. A project the user added may not have one yet.
    pub exists: bool,
    /// Project: the user added the folder to the list by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRef {
    pub root_id: String,
    /// The name of the skill's folder.
    pub dir_name: String,
}

/// `'error' | 'warning' | 'info'`, for skills and agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueLevel {
    Error,
    Warning,
    Info,
}

/// `claude`: it matters to Claude Code. `upload`: only when the skill is uploaded to claude.ai.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueScope {
    Claude,
    Upload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillIssue {
    pub level: IssueLevel,
    pub scope: IssueScope,
    pub message: String,
    /// It is about another skill of the same name, which only the whole list can tell (`clash?: true`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub clash: bool,
}

/// `SkillSummary extends SkillRef`; the ref's fields are inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub root_id: String,
    pub dir_name: String,
    /// The name in the file, or the folder's name when the file gives none.
    pub name: String,
    pub description: String,
    /// Claude may pick the skill up by itself.
    pub model_invocable: bool,
    /// The skill shows in the / menu and runs as /name.
    pub user_invocable: bool,
    /// The folder is a link to a folder somewhere else.
    pub linked: bool,
    /// What Claude Code would stumble over. Upload rules are not counted here.
    pub issues: Vec<SkillIssue>,
}

impl SkillSummary {
    pub fn skill_ref(&self) -> SkillRef {
        SkillRef { root_id: self.root_id.clone(), dir_name: self.dir_name.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFile {
    /// Relative to the skill's folder, with forward slashes.
    pub path: String,
    pub size: u64,
}

/// `SkillDetail extends SkillRef`; the ref's fields are inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub root_id: String,
    pub dir_name: String,
    /// The whole SKILL.md, with plain line feeds. Empty when it cannot be edited here.
    pub text: String,
    /// Stands for the bytes on disk; a save names the one it started from.
    pub hash: String,
    pub files: Vec<SkillFile>,
    /// There are more files than are listed.
    pub more_files: bool,
    /// The skill's folder, for showing.
    pub path: String,
    pub linked: bool,
    /// Why the file cannot be edited in the app, when it cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillListing {
    pub roots: Vec<SkillRoot>,
    pub skills: Vec<SkillSummary>,
}

/// What an archive holds, before it is brought in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    /// Names this archive for `skills:import`; the window never sees its path.
    pub token: String,
    /// The name the archive suggests.
    pub name: String,
    pub description: String,
    pub files: Vec<SkillFile>,
    pub more_files: bool,
    pub issues: Vec<SkillIssue>,
}

/// `{ saved: T } | { conflict: T }`: a save went through, or the file changed under it and this
/// is what is on disk now. Serde's external tagging is exactly this shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveOutcome<T> {
    Saved(T),
    Conflict(T),
}

pub type SaveResult = SaveOutcome<SkillDetail>;

/// Where a skill is after it was renamed, copied or moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMoved {
    pub r#ref: SkillRef,
    /// What did not go fully as asked, though the skill is where it should be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

// ---------- form fields (src/shared/frontmatter.ts) ----------

/// How a form shows one field. `text` is one line and `long` several; `list` is names separated
/// by commas, kept as one text; `names` is the same to the eye and always written as a YAML list;
/// `choice` is one of a few; `number` a whole number; `switch` a yes or no.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormFieldKind {
    Text,
    Long,
    List,
    Names,
    Choice,
    Number,
    Switch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormChoice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub key: String,
    pub label: String,
    pub kind: FormFieldKind,
    pub hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// For `choice`; the empty value stands for "not set".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<FormChoice>>,
    /// For `choice`: values Claude Code takes as well, though the form does not offer them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts: Option<Vec<String>>,
}

pub type SkillField = FormField;
