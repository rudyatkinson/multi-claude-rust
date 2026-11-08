//! `src/shared/instructions.ts`: the instruction files Claude Code reads when a session starts.

use serde::{Deserialize, Serialize};

use super::SaveOutcome;

pub struct InstructionLimits {
    /// Anthropic's advice for the length of one CLAUDE.md, in lines.
    pub lines: usize,
    /// Name length of a rule made here.
    pub rule_name: usize,
    /// How far `@path` imports are followed, as Claude Code follows them.
    pub import_hops: usize,
}

pub const INSTRUCTION_LIMITS: InstructionLimits = InstructionLimits { lines: 200, rule_name: 64, import_hops: 4 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstructionScopeKind {
    Account,
    Project,
}

/// Whose files: an account's (its config folder), or a project folder's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionScope {
    /// Stands for the folder; the window never names a path itself.
    pub id: String,
    pub kind: InstructionScopeKind,
    /// Account: whose config folder it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// The config folder, or the project folder; for showing.
    pub dir: String,
    /// Account: its CLAUDE.md is a copy that syncing from the main account overwrites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_claude_md: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstructionKind {
    Claude,
    Local,
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionFile {
    /// Within the scope's folder, with forward slashes: `CLAUDE.md`, `.claude/rules/tests.md`.
    pub file: String,
    pub kind: InstructionKind,
    /// False for a file Claude Code would read if it were there; it can be made.
    pub exists: bool,
    pub lines: u32,
    /// A rule's `paths`: it is read only when Claude works on a matching file.
    pub paths: Vec<String>,
    /// The `@path` imports in the file.
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionRef {
    pub scope_id: String,
    pub file: String,
}

/// `InstructionDetail extends InstructionRef`; the ref's fields are inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDetail {
    pub scope_id: String,
    pub file: String,
    pub text: String,
    pub hash: String,
    /// The file, for showing.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<String>,
}

pub type InstructionSaveResult = SaveOutcome<InstructionDetail>;

/// The second argument of `instructions:create`: `{ file } | { rule }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstructionTarget {
    /// One of the scope's fixed files that is not there yet.
    File(String),
    /// A rule of the given name.
    Rule(String),
}

/// `start`: read when the session starts. `demand`: a rule with `paths`, read when a matching
/// file comes up. `missing`: an import that leads nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadWhen {
    Start,
    Demand,
    Missing,
}

/// One file a session reads, in the order of reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadStep {
    /// The file on disk.
    pub path: String,
    /// What it is: `Account`, `Project`, `Rule`, `Local`, `Import`, `Memory`, `Managed policy`.
    pub role: String,
    pub when: LoadWhen,
    pub lines: u32,
    /// How deep it was imported: 0 for a file Claude Code finds by itself.
    pub depth: u32,
    /// Something worth knowing about this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
