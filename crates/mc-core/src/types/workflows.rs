//! `src/shared/workflows.ts` (workflows, runs, their limits and choices) and the file of
//! `src/main/workflows/store.ts` (`workflows.json`).

use serde::{Deserialize, Serialize};

use super::serde_util::{array_or_default, one, yes};
use super::Extra;

/// The model aliases offered for a step. A full model id may be typed instead.
pub const MODEL_ALIASES: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

pub const EFFORT_LEVELS: [Effort; 5] = [Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh, Effort::Max];

impl Effort {
    /// The name as `--effort` takes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

/// The permission modes a step may start in. `bypassPermissions` is left out on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PermissionMode {
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "dontAsk")]
    DontAsk,
}

pub const PERMISSION_MODES: [PermissionMode; 5] =
    [PermissionMode::Plan, PermissionMode::Manual, PermissionMode::AcceptEdits, PermissionMode::Auto, PermissionMode::DontAsk];

impl PermissionMode {
    /// The name as `--permission-mode` takes it.
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Plan => "plan",
            PermissionMode::Manual => "manual",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Auto => "auto",
            PermissionMode::DontAsk => "dontAsk",
        }
    }
}

pub struct WorkflowLimits {
    pub name: usize,
    pub step_name: usize,
    pub steps: usize,
    pub prompt: usize,
    pub task: usize,
    pub workflows: usize,
}

pub const WORKFLOW_LIMITS: WorkflowLimits = WorkflowLimits { name: 60, step_name: 40, steps: 12, prompt: 100_000, task: 100_000, workflows: 100 };

/// One session of a workflow: an account, how Claude runs there, and what it is told. Fields
/// this version does not know are kept in `extra`, as `parseStep` kept them.
///
/// Typed reading is stricter than `parseStep`: an `effort` or `permissionMode` outside the
/// choices fails here, where `parseStep` dropped the field. Reading untrusted input goes through
/// the parse port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub id: String,
    /// Shown on the step's card and tab, and used in `{{steps.<name>.output}}`.
    pub name: String,
    pub profile_id: String,
    /// An alias or a full model id. Missing: whatever the account uses by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// The id of an earlier step whose conversation this step goes on with, instead of starting a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continues: Option<String>,
    /// What Claude is told, with `{{...}}` variables.
    pub prompt: String,
    /// Wait for the user's approval before the next step starts. On unless `false` (`raw.gate !== false`).
    #[serde(default = "yes")]
    pub gate: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A designed workflow. Unknown fields are kept in `extra`, as `parseWorkflow` kept them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub steps: Vec<WorkflowStep>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The whole of `workflows.json`. What the file holds beyond what this version knows is kept in
/// `extra` and written back as it was. A `workflows` that is not a list reads as none; single
/// malformed workflows are for the parse port to drop (`parseWorkflows`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsFile {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default, deserialize_with = "array_or_default")]
    pub workflows: Vec<Workflow>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for WorkflowsFile {
    fn default() -> Self {
        WorkflowsFile { version: 1, workflows: Vec::new(), extra: Extra::new() }
    }
}

/// Something in a prompt that is written like a variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PromptRef {
    Task,
    Folder,
    Previous,
    Step { name: String },
    /// Looks like a variable and is none; it stays in the prompt as written.
    Unknown { text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowIssueLevel {
    /// Keeps the workflow from running.
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowIssue {
    /// The step it is about; missing for the workflow as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    pub level: WorkflowIssueLevel,
    pub message: String,
}

// ---------- a run, as the window sees it ----------

/// `starting`: its session is being opened. `running`: Claude works on it, or waits in its tab.
/// `gate`: it is finished and the run waits for approval. `paused`: it cannot go on by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepState {
    Pending,
    Starting,
    Running,
    Gate,
    Done,
    Paused,
}

/// `waiting`: at a gate. `paused`: the current step needs a retry or a hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    Running,
    Waiting,
    Paused,
    Done,
    Stopped,
}

impl RunState {
    /// `isActiveRun`: a run that still has something to do or to wait for.
    pub fn is_active(self) -> bool {
        matches!(self, RunState::Running | RunState::Waiting | RunState::Paused)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStep {
    pub id: String,
    pub name: String,
    pub profile_id: String,
    pub state: StepState,
    /// The tab the step runs in, while that tab exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Claude waits for the user in that tab: a permission, a question, a dialog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_you: Option<bool>,
    /// Claude gave an answer, and work it started in the background is still running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    pub has_output: bool,
    /// Why the step paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    /// The workflow's name when the run began.
    pub name: String,
    pub cwd: String,
    pub state: RunState,
    /// Index of the step the run is at.
    pub current: u32,
    pub steps: Vec<RunStep>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunInput {
    pub workflow_id: String,
    pub cwd: String,
    pub task: String,
}
