//! `src/shared/agents.ts`: Claude Code subagents as the Library sees them.

use serde::{Deserialize, Serialize};

use super::serde_util::is_false;
use super::{IssueLevel, SaveOutcome, SkillRoot};

pub struct AgentLimits {
    /// Name length Claude Code allows.
    pub name: usize,
    /// Name length for an agent made or renamed here, where the name names the file as well.
    pub new_name: usize,
    /// Lines of a system prompt after which the list says so.
    pub lines: usize,
}

pub const AGENT_LIMITS: AgentLimits = AgentLimits { name: 256, new_name: 64, lines: 500 };

/// A folder that holds subagents: the same thing as a folder that holds skills.
pub type AgentRoot = SkillRoot;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    pub root_id: String,
    /// The file within the agents folder, with forward slashes: `reviewer.md`, `team/qa.md`.
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIssue {
    pub level: IssueLevel,
    pub message: String,
    /// It is about another agent of the same name (`clash?: true`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub clash: bool,
}

/// `AgentSummary extends AgentRef`; the ref's fields are inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub root_id: String,
    pub file: String,
    /// The name in the file; empty when the file gives none.
    pub name: String,
    pub description: String,
    pub model: String,
    pub effort: String,
    /// Claude Code can load the file as an agent.
    pub loads: bool,
    pub issues: Vec<AgentIssue>,
}

impl AgentSummary {
    pub fn agent_ref(&self) -> AgentRef {
        AgentRef { root_id: self.root_id.clone(), file: self.file.clone() }
    }
}

/// `AgentDetail extends AgentRef`; the ref's fields are inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDetail {
    pub root_id: String,
    pub file: String,
    /// The whole file, with plain line feeds. Empty when it cannot be edited here.
    pub text: String,
    /// Stands for the bytes on disk; a save names the one it started from.
    pub hash: String,
    /// The file, for showing.
    pub path: String,
    /// Why the file cannot be edited in the app, when it cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentListing {
    pub roots: Vec<AgentRoot>,
    pub agents: Vec<AgentSummary>,
}

pub type AgentSaveResult = SaveOutcome<AgentDetail>;

/// Where an agent is after it was renamed, copied or moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMoved {
    pub r#ref: AgentRef,
    /// What did not go fully as asked, though the agent is where it should be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}
