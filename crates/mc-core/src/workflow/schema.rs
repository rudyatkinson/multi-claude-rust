//! `src/shared/workflows.ts`: reading workflows from a file or from the window, the variables of
//! a prompt, editing a workflow, and what stands in the way of running one.
//!
//! Lengths are JavaScript `length`s, counted in UTF-16 units (see [`crate::jsstr`]).

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::{Map, Value};

use crate::jsstr::cut16;
use crate::tab_order::reorder;
use crate::turn::js_trim;
use crate::types::{
    Extra, PermissionMode, Profile, ProfileView, PromptRef, UsageState, Workflow, WorkflowIssue, WorkflowIssueLevel,
    WorkflowRun, WorkflowStep, EFFORT_LEVELS, PERMISSION_MODES, WORKFLOW_LIMITS,
};
use crate::usage::view::{used_up, AccountRef};

// ---------- reading what comes from a file or from the window ----------

static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{1,64}$").unwrap());
/// A model alias or id as `--model` takes it. It cannot start with a dash, which would make it a flag.
static MODEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:\[\]-]{0,79}$").unwrap());
static STEP_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}][\p{L}\p{N} _-]*$").unwrap());

/// The keys `parseStep` sets itself; every other key of the raw step is kept as it was.
const STEP_KEYS: [&str; 9] = ["id", "name", "profileId", "model", "effort", "permissionMode", "continues", "prompt", "gate"];
const WORKFLOW_KEYS: [&str; 3] = ["id", "name", "steps"];
/// `text(raw.profileId, 64)`.
const PROFILE_ID_MAX: usize = 64;

/// `typeof value === 'string' ? value : ''`.
fn str_of(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

/// The value when it is a string that passes `ID`.
fn id_of(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| ID.is_match(s))
}

/// `{ ...raw }` without the keys that are set anew.
fn rest(raw: &Map<String, Value>, known: &[&str]) -> Extra {
    raw.iter().filter(|(key, _)| !known.contains(&key.as_str())).map(|(key, value)| (key.clone(), value.clone())).collect()
}

/// `isModel`: a model alias or id that `--model` can take.
pub fn is_model(value: &str) -> bool {
    MODEL.is_match(value)
}

/// A step with every field of the right type; fields this version does not know are kept in
/// `extra`. None when it has no usable id. A `model`, `effort`, `permissionMode` or `continues`
/// that is not one of the choices is dropped (a missing key), not an error. What the fields say
/// is judged by [`validate_workflow`], not here.
pub fn parse_step(raw: &Value) -> Option<WorkflowStep> {
    let obj = raw.as_object()?;
    let id = id_of(obj.get("id"))?;
    let choice = |key: &str| obj.get(key).and_then(Value::as_str);
    Some(WorkflowStep {
        id: id.to_string(),
        name: js_trim(cut16(str_of(obj.get("name")), WORKFLOW_LIMITS.step_name)).to_string(),
        profile_id: cut16(str_of(obj.get("profileId")), PROFILE_ID_MAX).to_string(),
        model: choice("model").filter(|m| is_model(m)).map(str::to_string),
        effort: choice("effort").and_then(|e| EFFORT_LEVELS.into_iter().find(|level| level.as_str() == e)),
        permission_mode: choice("permissionMode").and_then(|m| PERMISSION_MODES.into_iter().find(|mode| mode.as_str() == m)),
        continues: id_of(obj.get("continues")).map(str::to_string),
        prompt: cut16(str_of(obj.get("prompt")), WORKFLOW_LIMITS.prompt).to_string(),
        gate: obj.get("gate") != Some(&Value::Bool(false)),
        extra: rest(obj, &STEP_KEYS),
    })
}

/// A workflow with its readable steps (at most [`WORKFLOW_LIMITS`]`.steps`, the first of each
/// id); unknown fields are kept in `extra`. None when it has no usable id.
pub fn parse_workflow(raw: &Value) -> Option<Workflow> {
    let obj = raw.as_object()?;
    let id = id_of(obj.get("id"))?;
    let items = obj.get("steps").and_then(Value::as_array).map_or(&[][..], |list| &list[..list.len().min(WORKFLOW_LIMITS.steps)]);
    let mut seen = HashSet::new();
    let mut steps = Vec::new();
    for item in items {
        let Some(step) = parse_step(item) else { continue };
        if !seen.insert(step.id.clone()) {
            continue;
        }
        steps.push(step);
    }
    Some(Workflow {
        id: id.to_string(),
        name: js_trim(cut16(str_of(obj.get("name")), WORKFLOW_LIMITS.name)).to_string(),
        steps,
        extra: rest(obj, &WORKFLOW_KEYS),
    })
}

/// The workflows of a saved file (`{ workflows: [...] }`); entries that cannot be read, and
/// later entries with an id seen before, are left out. Anything else gives an empty list.
pub fn parse_workflows(raw: &Value) -> Vec<Workflow> {
    let list = raw.as_object().and_then(|obj| obj.get("workflows")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let mut seen = HashSet::new();
    let mut workflows = Vec::new();
    for item in &list[..list.len().min(WORKFLOW_LIMITS.workflows)] {
        let Some(workflow) = parse_workflow(item) else { continue };
        if !seen.insert(workflow.id.clone()) {
            continue;
        }
        workflows.push(workflow);
    }
    workflows
}

// ---------- prompt variables ----------

/// `{{ ... }}` on one line, without braces inside.
static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{([^{}\r\n]*)\}\}").unwrap());
/// `/^steps\.(.+)\.output$/i`. The case is spelled out: JavaScript's `/i` without `/u` matches only
/// the ASCII cases of these letters, where `(?i)` would also take `ſ` for `s`. `.` stops at the
/// line terminators, as in JavaScript.
static STEP_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[sS][tT][eE][pP][sS]\.([^\n\r\u{2028}\u{2029}]+)\.[oO][uU][tT][pP][uU][tT]$").unwrap()
});

/// What the inside of a `{{ ... }}` refers to. Spaces around it and the case do not matter.
pub fn ref_of(inner: &str) -> PromptRef {
    let key = js_trim(inner);
    match key.to_lowercase().as_str() {
        "task" => return PromptRef::Task,
        "folder" => return PromptRef::Folder,
        "previous.output" => return PromptRef::Previous,
        _ => {}
    }
    if let Some(caps) = STEP_REF.captures(key) {
        let name = js_trim(&caps[1]);
        if !name.is_empty() {
            return PromptRef::Step { name: name.to_string() };
        }
    }
    PromptRef::Unknown { text: key.to_string() }
}

/// Two step names that cannot be told apart in a prompt.
pub(super) fn same_name(a: &str, b: &str) -> bool {
    js_trim(a).to_lowercase() == js_trim(b).to_lowercase()
}

/// Everything in a prompt that is written like a variable, in order.
pub fn prompt_refs(prompt: &str) -> Vec<PromptRef> {
    TOKEN.captures_iter(prompt).map(|caps| ref_of(&caps[1])).collect()
}

/// The output of an earlier step by its name, if it has one.
pub type StepOutput<'a> = Box<dyn Fn(&str) -> Option<&'a str> + 'a>;

/// What the variables of a prompt stand for.
pub struct PromptValues<'a> {
    pub task: &'a str,
    pub folder: &'a str,
    /// The output of the step before this one.
    pub previous: Option<&'a str>,
    /// The output of an earlier step, by its name.
    pub step: StepOutput<'a>,
}

/// The prompt with its variables filled in. It is one pass: text that arrives through a variable
/// is never read for variables itself. A variable without a value stays as it is written.
pub fn render_prompt(prompt: &str, values: &PromptValues<'_>) -> String {
    TOKEN
        .replace_all(prompt, |caps: &Captures| {
            let whole = &caps[0];
            match ref_of(&caps[1]) {
                PromptRef::Task => values.task.to_string(),
                PromptRef::Folder => values.folder.to_string(),
                PromptRef::Previous => values.previous.unwrap_or(whole).to_string(),
                PromptRef::Step { name } => (values.step)(&name).unwrap_or(whole).to_string(),
                PromptRef::Unknown { .. } => whole.to_string(),
            }
        })
        .into_owned()
}

/// The prompt with `{{steps.<from>.output}}` pointing at the step's new name.
pub fn rename_step_refs(prompt: &str, from: &str, to: &str) -> String {
    TOKEN
        .replace_all(prompt, |caps: &Captures| match ref_of(&caps[1]) {
            PromptRef::Step { name } if same_name(&name, from) => format!("{{{{steps.{to}.output}}}}"),
            _ => caps[0].to_string(),
        })
        .into_owned()
}

// ---------- editing ----------

/// "Step 3", or the first such name that is free.
pub fn free_step_name(steps: &[WorkflowStep]) -> String {
    free_step_name_from(steps, "Step")
}

/// `freeStepName(steps, base)`: `"<base> <n>"`, counting on from the number of steps plus one.
pub fn free_step_name_from(steps: &[WorkflowStep], base: &str) -> String {
    let mut n = steps.len() + 1;
    loop {
        let name = format!("{base} {n}");
        if !steps.iter().any(|s| same_name(&s.name, &name)) {
            return name;
        }
        n += 1;
    }
}

/// The workflow with other steps and the same id, name and unknown fields.
fn with_steps(workflow: &Workflow, steps: Vec<WorkflowStep>) -> Workflow {
    Workflow { id: workflow.id.clone(), name: workflow.name.clone(), steps, extra: workflow.extra.clone() }
}

/// Renames a step and every `{{steps.<name>.output}}` that names it. The same workflow
/// (`Cow::Borrowed`) when there is no such step or the name does not change.
pub fn rename_step<'a>(workflow: &'a Workflow, step_id: &str, name: &str) -> Cow<'a, Workflow> {
    let Some(old) = workflow.steps.iter().find(|s| s.id == step_id) else { return Cow::Borrowed(workflow) };
    if old.name == name {
        return Cow::Borrowed(workflow);
    }
    // a name that is shared with another step cannot be told apart in the prompts
    let rewrite = !js_trim(&old.name).is_empty()
        && !js_trim(name).is_empty()
        && !workflow.steps.iter().any(|s| s.id != step_id && same_name(&s.name, &old.name));
    let steps = workflow
        .steps
        .iter()
        .map(|s| {
            let mut s = s.clone();
            if s.id == step_id {
                s.name = name.to_string();
            }
            if rewrite {
                s.prompt = rename_step_refs(&s.prompt, &old.name, js_trim(name));
            }
            s
        })
        .collect();
    Cow::Owned(with_steps(workflow, steps))
}

/// Removes a step; steps that went on with its conversation start their own instead.
pub fn remove_step(workflow: &Workflow, step_id: &str) -> Workflow {
    let steps = workflow
        .steps
        .iter()
        .filter(|s| s.id != step_id)
        .map(|s| {
            let mut s = s.clone();
            if s.continues.as_deref() == Some(step_id) {
                s.continues = None;
            }
            s
        })
        .collect();
    with_steps(workflow, steps)
}

/// Moves a step in front of another one, or to the end with `before_id` None. The same workflow
/// (`Cow::Borrowed`) when nothing moves.
pub fn move_step<'a>(workflow: &'a Workflow, step_id: &str, before_id: Option<&str>) -> Cow<'a, Workflow> {
    let ids: Vec<String> = workflow.steps.iter().map(|s| s.id.clone()).collect();
    let order = reorder(&ids, step_id, before_id);
    // an id stands for the first step that has it, as `find` takes it
    let from: Vec<usize> = order.iter().filter_map(|id| workflow.steps.iter().position(|s| &s.id == id)).collect();
    if from.len() == workflow.steps.len() && from.iter().enumerate().all(|(i, &at)| i == at) {
        return Cow::Borrowed(workflow);
    }
    Cow::Owned(with_steps(workflow, from.iter().map(|&at| workflow.steps[at].clone()).collect()))
}

/// The index of the step whose conversation the step at `index` belongs to: itself when it
/// starts a new one, else the start of the chain of `continues`. A link that does not point at
/// an earlier step counts as none.
pub fn thread_of(steps: &[WorkflowStep], index: usize) -> usize {
    let mut at = index;
    loop {
        let before = steps
            .get(at)
            .and_then(|s| s.continues.as_deref())
            .and_then(|target| steps[..at].iter().position(|s| s.id == target));
        match before {
            Some(before) => at = before,
            None => return at,
        }
    }
}

/// A workflow to start from: plan on one account, implement on another, review where the plan
/// was made. `new_id` is called for the plan step, the workflow, the implement step and the
/// review step, in that order.
pub fn sample_workflow(mut new_id: impl FnMut() -> String, plan_profile_id: &str, implement_profile_id: &str) -> Workflow {
    let plan = new_id();
    let id = new_id();
    let step = |id: String, name: &str, profile_id: &str, model: &str, prompt: &str| WorkflowStep {
        id,
        name: name.to_string(),
        profile_id: profile_id.to_string(),
        model: Some(model.to_string()),
        effort: None,
        permission_mode: None,
        continues: None,
        prompt: prompt.to_string(),
        gate: true,
        extra: Extra::new(),
    };
    let mut first = step(
        plan.clone(),
        "Plan",
        plan_profile_id,
        "fable",
        "Plan this task. Do not change any files.\n\nTask:\n{{task}}\n\nExplore the code as far as you need, then present a concrete, step-by-step implementation plan: the files to change, what changes in each, and how to verify the result.",
    );
    first.permission_mode = Some(PermissionMode::Plan);
    let second = step(
        new_id(),
        "Implement",
        implement_profile_id,
        "opus",
        "Implement the following plan in this folder.\n\nTask:\n{{task}}\n\nPlan:\n{{previous.output}}\n\nWhen you are done, summarize what you changed and how you verified it.",
    );
    let mut third = step(
        new_id(),
        "Review",
        plan_profile_id,
        "fable",
        "Another session has implemented your plan. Review the result against the plan. Do not change any files.\n\nIts summary:\n{{previous.output}}\n\nLook at the actual changes in the folder (for example with git diff). Report what is wrong or missing, most important first, and say clearly whether the work is ready.",
    );
    // out of the plan mode the conversation was left in; edits would still ask first
    third.permission_mode = Some(PermissionMode::Manual);
    third.continues = Some(plan);
    Workflow { id, name: "Plan, implement, review".to_string(), steps: vec![first, second, third], extra: Extra::new() }
}

// ---------- checks ----------

fn issue(level: WorkflowIssueLevel, message: impl Into<String>, step_id: Option<&str>) -> WorkflowIssue {
    WorkflowIssue { step_id: step_id.map(str::to_string), level, message: message.into() }
}

/// What is wrong with a workflow as it is designed. `profile_ids` are the accounts that exist.
pub fn validate_workflow(workflow: &Workflow, profile_ids: &HashSet<String>) -> Vec<WorkflowIssue> {
    let mut issues = Vec::new();
    let error = |issues: &mut Vec<WorkflowIssue>, message: String, step_id: Option<&str>| {
        issues.push(issue(WorkflowIssueLevel::Error, message, step_id));
    };
    let warning = |issues: &mut Vec<WorkflowIssue>, message: String, step_id: Option<&str>| {
        issues.push(issue(WorkflowIssueLevel::Warning, message, step_id));
    };

    if js_trim(&workflow.name).is_empty() {
        error(&mut issues, "The workflow needs a name.".into(), None);
    }
    if workflow.steps.is_empty() {
        error(&mut issues, "The workflow has no steps.".into(), None);
    }

    let steps = &workflow.steps;
    for (index, step) in steps.iter().enumerate() {
        let id = Some(step.id.as_str());
        let name = js_trim(&step.name);
        if name.is_empty() {
            error(&mut issues, "The step needs a name.".into(), id);
        } else if !STEP_NAME.is_match(name) {
            error(
                &mut issues,
                "A step name can have letters, digits, spaces, \"-\" and \"_\", and starts with a letter or digit.".into(),
                id,
            );
        } else if steps[..index].iter().any(|other| same_name(&other.name, name)) {
            error(&mut issues, format!("Another step is already named \"{name}\"."), id);
        }

        if step.profile_id.is_empty() {
            error(&mut issues, "Choose an account for this step.".into(), id);
        } else if !profile_ids.contains(&step.profile_id) {
            error(&mut issues, "The account of this step no longer exists. Choose another one.".into(), id);
        }

        if js_trim(&step.prompt).is_empty() {
            error(&mut issues, "The step has no prompt.".into(), id);
        }

        if let Some(continues) = &step.continues {
            match steps.iter().position(|s| &s.id == continues) {
                None => error(&mut issues, "The step whose conversation this one continues no longer exists.".into(), id),
                Some(target) if target >= index => error(
                    &mut issues,
                    format!(
                        "A step can only continue the conversation of a step before it, and \"{}\" comes later.",
                        steps[target].name
                    ),
                    id,
                ),
                Some(_) => {
                    // Claude Code resumes a conversation in the mode it was left in
                    let thread = thread_of(steps, index);
                    let before = (0..index).rev().find(|&i| thread_of(steps, i) == thread).map(|i| &steps[i]);
                    if let Some(before) = before {
                        if before.permission_mode == Some(PermissionMode::Plan) && step.permission_mode.is_none() {
                            warning(
                                &mut issues,
                                format!(
                                    "\"{}\" leaves the conversation in plan mode, and this step stays in it. Choose a permission mode if it should not.",
                                    before.name
                                ),
                                id,
                            );
                        }
                    }
                }
            }
        }

        for found in prompt_refs(&step.prompt) {
            match found {
                PromptRef::Previous if index == 0 => error(
                    &mut issues,
                    "The first step has no previous step: {{previous.output}} cannot be used in it.".into(),
                    id,
                ),
                PromptRef::Step { name } => match steps.iter().position(|s| same_name(&s.name, &name)) {
                    None => error(&mut issues, format!("The prompt uses the output of \"{name}\", and no step has that name."), id),
                    Some(target) if target >= index => error(
                        &mut issues,
                        format!("The prompt uses the output of \"{name}\", which has not run yet at this point."),
                        id,
                    ),
                    Some(_) => {}
                },
                PromptRef::Unknown { text } => {
                    warning(&mut issues, format!("{{{{{text}}}}} is not a variable; it is sent as it is written."), id)
                }
                _ => {}
            }
        }
    }
    issues
}

/// As much of an account as [`preflight`] needs: `{ id, name, account?: { loggedIn } }`.
pub trait NamedAccount: AccountRef {
    fn account_name(&self) -> &str;
}

impl NamedAccount for Profile {
    fn account_name(&self) -> &str {
        &self.name
    }
}

impl NamedAccount for ProfileView {
    fn account_name(&self) -> &str {
        &self.name
    }
}

impl<T: NamedAccount + ?Sized> NamedAccount for &T {
    fn account_name(&self) -> &str {
        (**self).account_name()
    }
}

/// What stands in the way of running a workflow right now: an account that is not signed in is
/// an error (told once per account), an account that looks out of its limit is a warning (it may
/// have extra usage enabled; told once per account and limit). Steps whose account is gone are
/// left to [`validate_workflow`].
pub fn preflight<P: NamedAccount>(
    workflow: &Workflow,
    profiles: &[P],
    usage: &BTreeMap<String, UsageState>,
    now: i64,
) -> Vec<WorkflowIssue> {
    let mut issues = Vec::new();
    let mut told: HashSet<String> = HashSet::new();
    for step in &workflow.steps {
        let Some(profile) = profiles.iter().find(|p| p.account_id() == step.profile_id) else { continue };
        let id = Some(step.id.as_str());
        if !profile.logged_in() {
            if told.insert(profile.account_id().to_string()) {
                issues.push(issue(WorkflowIssueLevel::Error, format!("\"{}\" is not signed in.", profile.account_name()), id));
            }
            continue;
        }
        let Some(limit) = used_up(usage.get(profile.account_id()), now, step.model.as_deref()) else { continue };
        if told.insert(format!("{} {}", profile.account_id(), limit.label)) {
            issues.push(issue(
                WorkflowIssueLevel::Warning,
                format!("\"{}\" looks out of its {}; step \"{}\" may stop there.", profile.account_name(), limit.label, step.name),
                id,
            ));
        }
    }
    issues
}

// ---------- a run, as the window sees it ----------

/// `isActiveRun`: a run that still has something to do or to wait for.
pub fn is_active_run(run: &WorkflowRun) -> bool {
    run.state.is_active()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::{ModelUsage, UsageWindow};

    fn step(id: &str) -> WorkflowStep {
        WorkflowStep {
            id: id.into(),
            name: id.into(),
            profile_id: "main".into(),
            model: None,
            effort: None,
            permission_mode: None,
            continues: None,
            prompt: "Do {{task}}".into(),
            gate: true,
            extra: Extra::new(),
        }
    }
    fn with(mut s: WorkflowStep, patch: impl FnOnce(&mut WorkflowStep)) -> WorkflowStep {
        patch(&mut s);
        s
    }
    fn named(id: &str, name: &str) -> WorkflowStep {
        with(step(id), |s| s.name = name.into())
    }
    fn prompted(id: &str, prompt: &str) -> WorkflowStep {
        with(step(id), |s| s.prompt = prompt.into())
    }
    fn continuing(id: &str, target: &str) -> WorkflowStep {
        with(step(id), |s| s.continues = Some(target.into()))
    }
    fn flow(steps: Vec<WorkflowStep>) -> Workflow {
        Workflow { id: "w1".into(), name: "Flow".into(), steps, extra: Extra::new() }
    }
    fn accounts() -> HashSet<String> {
        ["main", "work"].into_iter().map(String::from).collect()
    }
    fn messages_at(workflow: &Workflow, level: Option<WorkflowIssueLevel>) -> Vec<String> {
        validate_workflow(workflow, &accounts())
            .into_iter()
            .filter(|i| level.is_none_or(|l| i.level == l))
            .map(|i| i.message)
            .collect()
    }
    fn messages(workflow: &Workflow) -> Vec<String> {
        messages_at(workflow, None)
    }
    fn ids(steps: &[WorkflowStep]) -> Vec<&str> {
        steps.iter().map(|s| s.id.as_str()).collect()
    }
    #[allow(clippy::ptr_arg)] // the variant is what is asked: a plain reference cannot tell
    fn borrowed(result: &Cow<'_, Workflow>, original: &Workflow) -> bool {
        matches!(result, Cow::Borrowed(w) if std::ptr::eq(*w, original))
    }

    mod parse_step {
        use super::*;

        #[test]
        fn keeps_what_is_valid_and_drops_what_is_not() {
            let parsed = parse_step(&json!({ "id": "s1", "name": "  Plan  ", "profileId": "main", "model": "opus", "effort": "ultra", "permissionMode": "plan", "prompt": "x", "gate": false }))
                .unwrap();
            let expected = WorkflowStep {
                id: "s1".into(),
                name: "Plan".into(),
                profile_id: "main".into(),
                model: Some("opus".into()),
                effort: None,
                permission_mode: Some(PermissionMode::Plan),
                continues: None,
                prompt: "x".into(),
                gate: false,
                extra: Extra::new(),
            };
            assert_eq!(parsed, expected);
            assert!(serde_json::to_value(&parsed).unwrap().get("effort").is_none());
        }

        #[test]
        fn never_lets_a_value_through_that_would_read_as_a_flag() {
            assert_eq!(parse_step(&json!({ "id": "s1", "model": "--dangerously-skip-permissions" })).unwrap().model, None);
            assert_eq!(parse_step(&json!({ "id": "s1", "model": "-p" })).unwrap().model, None);
            assert_eq!(parse_step(&json!({ "id": "s1", "permissionMode": "bypassPermissions" })).unwrap().permission_mode, None);
            assert_eq!(parse_step(&json!({ "id": "s1", "effort": "--max" })).unwrap().effort, None);
            assert!(is_model("claude-opus-5-5"));
            assert!(is_model("opus[1m]"));
            assert!(!is_model("opus plan"));
            assert!(!is_model(""));
        }

        #[test]
        fn has_a_gate_unless_it_is_switched_off_and_refuses_a_step_without_an_id() {
            let bare = WorkflowStep {
                id: "s1".into(),
                name: String::new(),
                profile_id: String::new(),
                model: None,
                effort: None,
                permission_mode: None,
                continues: None,
                prompt: String::new(),
                gate: true,
                extra: Extra::new(),
            };
            assert_eq!(parse_step(&json!({ "id": "s1" })), Some(bare));
            assert_eq!(parse_step(&json!({ "name": "no id" })), None);
            assert_eq!(parse_step(&json!({ "id": "bad id" })), None);
            assert_eq!(parse_step(&json!("text")), None);
        }

        #[test]
        fn keeps_fields_a_later_version_may_have_added() {
            let parsed = parse_step(&json!({ "id": "s1", "timeoutMinutes": 5 })).unwrap();
            assert_eq!(parsed.id, "s1");
            assert_eq!(parsed.extra.get("timeoutMinutes"), Some(&json!(5)));
            let workflow = parse_workflow(&json!({ "id": "w", "name": "N", "steps": [], "color": "red" })).unwrap();
            assert_eq!(workflow.extra.get("color"), Some(&json!("red")));
        }
    }

    mod parse_workflows {
        use super::*;

        #[test]
        fn reads_a_file_and_leaves_out_what_it_cannot_use() {
            let list = parse_workflows(&json!({
                "version": 1,
                "workflows": [
                    { "id": "a", "name": "A", "steps": [{ "id": "s1" }, { "id": "s1" }, { "nope": true }, { "id": "s2" }] },
                    { "id": "a", "name": "twice" },
                    "junk",
                    { "name": "no id" }
                ]
            }));
            assert_eq!(list.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["a"]);
            assert_eq!(ids(&list[0].steps), ["s1", "s2"]);
        }

        #[test]
        fn gives_an_empty_list_for_anything_else() {
            assert!(parse_workflows(&Value::Null).is_empty());
            assert!(parse_workflows(&json!([])).is_empty());
            assert!(parse_workflows(&json!({ "workflows": "x" })).is_empty());
        }
    }

    mod prompt_variables {
        use super::*;

        fn values<'a>(task: &'a str, previous: Option<&'a str>) -> PromptValues<'a> {
            PromptValues {
                task,
                folder: "C:\\code\\app",
                previous,
                step: Box::new(|name: &str| -> Option<&'a str> { (name.to_lowercase() == "plan").then_some("PLAN TEXT") }),
            }
        }
        fn plain() -> PromptValues<'static> {
            values("fix the login", Some("the plan"))
        }

        #[test]
        fn fills_in_the_task_the_folder_the_previous_output_and_a_named_step() {
            assert_eq!(
                render_prompt("{{task}} in {{folder}}: {{previous.output}} / {{steps.Plan.output}}", &plain()),
                "fix the login in C:\\code\\app: the plan / PLAN TEXT"
            );
        }

        #[test]
        fn allows_spaces_inside_the_braces_and_any_case() {
            assert_eq!(render_prompt("{{ Task }} {{STEPS.plan.OUTPUT}} {{ previous.output }}", &plain()), "fix the login PLAN TEXT the plan");
        }

        #[test]
        fn reads_text_that_came_in_through_a_variable_only_as_text() {
            let tricky = values("{{previous.output}} and {{steps.Plan.output}}", Some("{{task}}"));
            assert_eq!(render_prompt("{{task}} | {{previous.output}}", &tricky), "{{previous.output}} and {{steps.Plan.output}} | {{task}}");
        }

        #[test]
        fn leaves_alone_what_is_not_a_variable_or_has_no_value() {
            let text = "Vue {{ user.name }}, Jinja {{ item|upper }}, {{steps.Nope.output}}, {{}} and { {task} }";
            assert_eq!(render_prompt(text, &plain()), text);
            assert_eq!(render_prompt("{{previous.output}}", &values("fix the login", None)), "{{previous.output}}");
            // a replacement pattern in the value is not a pattern
            assert_eq!(render_prompt("{{task}}", &values("$& $1 $'", Some("the plan"))), "$& $1 $'");
        }

        #[test]
        fn lists_what_a_prompt_refers_to() {
            assert_eq!(
                prompt_refs("{{task}} {{folder}} {{previous.output}} {{steps.My Plan.output}} {{nope}} text"),
                [
                    PromptRef::Task,
                    PromptRef::Folder,
                    PromptRef::Previous,
                    PromptRef::Step { name: "My Plan".into() },
                    PromptRef::Unknown { text: "nope".into() },
                ]
            );
            assert!(prompt_refs("no variables, {single} braces").is_empty());
        }

        #[test]
        fn renames_references_to_a_step() {
            assert_eq!(
                rename_step_refs("a {{steps.plan.output}} b {{ steps.Plan.output }} c {{steps.Other.output}}", "Plan", "Design"),
                "a {{steps.Design.output}} b {{steps.Design.output}} c {{steps.Other.output}}"
            );
        }
    }

    mod editing {
        use super::*;

        fn three() -> Workflow {
            flow(vec![
                named("a", "Plan"),
                with(step("b"), |s| {
                    s.name = "Build".into();
                    s.prompt = "Use {{steps.Plan.output}}".into();
                }),
                with(step("c"), |s| {
                    s.name = "Review".into();
                    s.continues = Some("a".into());
                }),
            ])
        }

        #[test]
        fn renames_a_step_together_with_the_prompts_that_name_it() {
            let three = three();
            let renamed = rename_step(&three, "a", "Design");
            assert_eq!(renamed.steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Design", "Build", "Review"]);
            assert_eq!(renamed.steps[1].prompt, "Use {{steps.Design.output}}");
            assert!(borrowed(&rename_step(&three, "a", "Plan"), &three));
            assert!(borrowed(&rename_step(&three, "zz", "X"), &three));
        }

        #[test]
        fn does_not_touch_the_prompts_while_the_name_is_being_cleared_or_is_shared() {
            assert_eq!(rename_step(&three(), "a", "").steps[1].prompt, "Use {{steps.Plan.output}}");
            let twins = flow(vec![
                named("a", "Plan"),
                named("b", "Plan"),
                with(step("c"), |s| {
                    s.name = "C".into();
                    s.prompt = "{{steps.Plan.output}}".into();
                }),
            ]);
            assert_eq!(rename_step(&twins, "b", "Other").steps[2].prompt, "{{steps.Plan.output}}");
        }

        #[test]
        fn removes_a_step_and_the_links_to_its_conversation() {
            let left = remove_step(&three(), "a");
            assert_eq!(ids(&left.steps), ["b", "c"]);
            assert_eq!(left.steps[1].continues, None);
            assert!(serde_json::to_value(&left.steps[1]).unwrap().get("continues").is_none());
        }

        #[test]
        fn moves_a_step() {
            let three = three();
            assert_eq!(ids(&move_step(&three, "c", Some("a")).steps), ["c", "a", "b"]);
            assert_eq!(ids(&move_step(&three, "a", None).steps), ["b", "c", "a"]);
            assert!(borrowed(&move_step(&three, "a", Some("b")), &three));
        }

        #[test]
        fn finds_a_free_step_name() {
            assert_eq!(free_step_name(&three().steps), "Step 4");
            assert_eq!(free_step_name(&[named("a", "step 2")]), "Step 3");
            assert_eq!(free_step_name(&[named("a", "Step 2"), named("b", "Step 3")]), "Step 4");
        }

        #[test]
        fn follows_a_conversation_back_to_the_step_that_started_it() {
            let chain = [step("a"), step("b"), continuing("c", "a"), continuing("d", "c"), continuing("e", "zz")];
            assert_eq!((0..chain.len()).map(|i| thread_of(&chain, i)).collect::<Vec<_>>(), [0, 1, 0, 0, 4]);
            // a link that points forward or at itself is no link
            let odd = [continuing("a", "b"), continuing("b", "b")];
            assert_eq!((0..odd.len()).map(|i| thread_of(&odd, i)).collect::<Vec<_>>(), [0, 1]);
        }

        #[test]
        fn builds_the_sample_plan_and_review_on_one_account_implement_on_the_other() {
            let mut n = 0;
            let sample = sample_workflow(
                || {
                    n += 1;
                    format!("id{n}")
                },
                "main",
                "work",
            );
            assert_eq!(
                sample.steps.iter().map(|s| (s.name.as_str(), s.profile_id.as_str(), s.model.as_deref())).collect::<Vec<_>>(),
                [("Plan", "main", Some("fable")), ("Implement", "work", Some("opus")), ("Review", "main", Some("fable"))]
            );
            assert_eq!(sample.steps[2].continues.as_deref(), Some(sample.steps[0].id.as_str()));
            // the review goes on with the plan's conversation, and leaves its plan mode
            assert_eq!(
                sample.steps.iter().map(|s| s.permission_mode).collect::<Vec<_>>(),
                [Some(PermissionMode::Plan), None, Some(PermissionMode::Manual)]
            );
            assert_eq!(thread_of(&sample.steps, 2), 0);
            assert!(validate_workflow(&sample, &accounts()).is_empty());
            assert_eq!(parse_workflow(&serde_json::to_value(&sample).unwrap()), Some(sample));
        }
    }

    mod validate_workflow {
        use super::*;

        #[test]
        fn accepts_a_plain_chain() {
            assert_eq!(messages(&flow(vec![step("a"), prompted("b", "{{previous.output}}")])), Vec::<String>::new());
        }

        #[test]
        fn wants_a_name_steps_and_for_every_step_a_name_an_account_and_a_prompt() {
            let empty = Workflow { id: "w".into(), name: " ".into(), steps: vec![], extra: Extra::new() };
            assert_eq!(messages(&empty), ["The workflow needs a name.", "The workflow has no steps."]);
            let bare = flow(vec![with(step("a"), |s| {
                s.name = String::new();
                s.profile_id = String::new();
                s.prompt = "  ".into();
            })]);
            let issues = validate_workflow(&bare, &accounts());
            assert_eq!(
                issues.iter().map(|i| (i.step_id.as_deref(), i.level)).collect::<Vec<_>>(),
                [(Some("a"), WorkflowIssueLevel::Error); 3]
            );
        }

        #[test]
        fn refuses_names_that_cannot_be_told_apart_or_cannot_be_written_in_a_variable() {
            assert_eq!(messages(&flow(vec![named("a", "Plan"), named("b", "plan ")])), ["Another step is already named \"plan\"."]);
            assert!(messages(&flow(vec![named("a", "Plan.v2")]))[0].contains("A step name can have"));
            assert_eq!(messages(&flow(vec![named("a", "Tasarım 2_son-hali")])), Vec::<String>::new());
        }

        #[test]
        fn reports_an_account_that_is_gone() {
            let gone = flow(vec![with(step("a"), |s| s.profile_id = "deleted".into())]);
            assert_eq!(messages(&gone), ["The account of this step no longer exists. Choose another one."]);
        }

        #[test]
        fn lets_a_step_continue_only_a_step_before_it() {
            assert_eq!(messages(&flow(vec![step("a"), continuing("b", "a")])), Vec::<String>::new());
            assert!(messages(&flow(vec![continuing("a", "b"), step("b")]))[0].contains("comes later"));
            assert!(messages(&flow(vec![continuing("a", "a")]))[0].contains("comes later"));
            assert!(messages(&flow(vec![continuing("a", "gone")]))[0].contains("no longer exists"));
        }

        #[test]
        fn warns_when_a_step_would_stay_in_the_plan_mode_of_the_conversation_it_continues() {
            let planned = || with(step("a"), |s| s.permission_mode = Some(PermissionMode::Plan));
            let mode = |s: WorkflowStep, mode: PermissionMode| with(s, |s| s.permission_mode = Some(mode));
            let warnings = |w: &Workflow| messages_at(w, Some(WorkflowIssueLevel::Warning));
            let errors = |w: &Workflow| messages_at(w, Some(WorkflowIssueLevel::Error));
            assert!(warnings(&flow(vec![planned(), continuing("b", "a")]))[0].contains("leaves the conversation in plan mode"));
            assert_eq!(errors(&flow(vec![planned(), continuing("b", "a")])), Vec::<String>::new());
            assert_eq!(messages(&flow(vec![planned(), mode(continuing("b", "a"), PermissionMode::Auto)])), Vec::<String>::new());
            assert_eq!(messages(&flow(vec![planned(), mode(continuing("b", "a"), PermissionMode::Plan)])), Vec::<String>::new());
            // a conversation of its own starts in the account's mode
            assert_eq!(messages(&flow(vec![planned(), step("b")])), Vec::<String>::new());
            // what counts is the step that had the conversation last
            assert_eq!(
                messages(&flow(vec![planned(), mode(continuing("b", "a"), PermissionMode::Auto), continuing("c", "a")])),
                Vec::<String>::new()
            );
            assert_eq!(
                warnings(&flow(vec![step("a"), mode(continuing("b", "a"), PermissionMode::Plan), continuing("c", "a")])).len(),
                1
            );
        }

        #[test]
        fn checks_what_the_prompts_refer_to() {
            assert!(messages(&flow(vec![prompted("a", "{{previous.output}}")]))[0].contains("The first step has no previous step"));
            assert!(messages(&flow(vec![prompted("a", "{{steps.b.output}}"), step("b")]))[0].contains("has not run yet"));
            assert!(messages(&flow(vec![prompted("a", "{{steps.a.output}}")]))[0].contains("has not run yet"));
            assert!(messages(&flow(vec![prompted("a", "{{steps.Nobody.output}}")]))[0].contains("no step has that name"));
            assert_eq!(messages(&flow(vec![step("a"), prompted("b", "{{steps.A.output}} {{task}} {{folder}}")])), Vec::<String>::new());
        }

        #[test]
        fn only_warns_about_braces_that_are_not_variables() {
            let issues = validate_workflow(&flow(vec![prompted("a", "Render {{ user.name }}")]), &accounts());
            assert_eq!(
                issues,
                [WorkflowIssue {
                    step_id: Some("a".into()),
                    level: WorkflowIssueLevel::Warning,
                    message: "{{user.name}} is not a variable; it is sent as it is written.".into(),
                }]
            );
        }
    }

    mod preflight {
        use super::*;

        const NOW: i64 = 1_000_000;

        struct Account {
            id: &'static str,
            name: &'static str,
            logged_in: bool,
        }
        impl AccountRef for Account {
            fn account_id(&self) -> &str {
                self.id
            }
            fn logged_in(&self) -> bool {
                self.logged_in
            }
        }
        impl NamedAccount for Account {
            fn account_name(&self) -> &str {
                self.name
            }
        }

        fn profiles() -> [Account; 2] {
            [Account { id: "main", name: "Main", logged_in: true }, Account { id: "work", name: "Work", logged_in: false }]
        }
        fn on(id: &str, profile_id: &str) -> WorkflowStep {
            with(step(id), |s| s.profile_id = profile_id.into())
        }
        fn usage_of(state: UsageState) -> BTreeMap<String, UsageState> {
            BTreeMap::from([("main".to_string(), state)])
        }

        #[test]
        fn stops_for_an_account_that_is_not_signed_in_once() {
            let both = flow(vec![step("a"), on("b", "work"), on("c", "work"), step("d")]);
            assert_eq!(
                preflight(&both, &profiles(), &BTreeMap::new(), NOW),
                [WorkflowIssue { step_id: Some("b".into()), level: WorkflowIssueLevel::Error, message: "\"Work\" is not signed in.".into() }]
            );
        }

        #[test]
        fn warns_about_a_limit_that_looks_used_up_once_per_limit() {
            let usage = usage_of(UsageState {
                five_hour: Some(UsageWindow { used_pct: 100.0, resets_at: Some(NOW + 60_000) }),
                ..UsageState::default()
            });
            let issues = preflight(&flow(vec![named("a", "Plan"), step("d")]), &profiles(), &usage, NOW);
            assert_eq!(
                issues,
                [WorkflowIssue {
                    step_id: Some("a".into()),
                    level: WorkflowIssueLevel::Warning,
                    message: "\"Main\" looks out of its 5-hour limit; step \"Plan\" may stop there.".into(),
                }]
            );
            // the limit has started over
            assert!(preflight(&flow(vec![step("a")]), &profiles(), &usage, NOW + 120_000).is_empty());
        }

        #[test]
        fn looks_at_the_weekly_limit_of_the_steps_own_model() {
            let usage = usage_of(UsageState {
                models: Some(vec![ModelUsage { name: "Fable".into(), used_pct: 100.0, resets_at: Some(NOW + 60_000) }]),
                ..UsageState::default()
            });
            let model = |m: &str| flow(vec![with(step("a"), |s| s.model = Some(m.into()))]);
            assert!(preflight(&model("opus"), &profiles(), &usage, NOW).is_empty());
            assert!(preflight(&model("fable"), &profiles(), &usage, NOW)[0].message.contains("Fable weekly limit"));
        }
    }
}
