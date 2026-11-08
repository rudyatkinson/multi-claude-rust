//! Commands of the `workflows` package (`workflows:*` of `src/main/ipc.ts`): the designed
//! workflows and their runs.

use mc_core::types::WorkflowRun;
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::error::{CmdError, CmdResult};
use crate::state::AppState;
use crate::workflows::{delete_workflow, run_workflow, save_workflow};

/// `workflows:save`: adds a workflow or replaces the one with its id. What cannot be read as a
/// workflow is `Invalid workflow`.
#[tauri::command]
pub fn workflows_save(state: State<'_, AppState>, workflow: Option<Value>) -> CmdResult<()> {
    Ok(save_workflow(&state, &workflow.unwrap_or(Value::Null))?)
}

/// `workflows:delete`.
#[tauri::command]
pub fn workflows_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let id = validate::id(&id, "workflow")?;
    Ok(delete_workflow(&state, id)?)
}

/// `workflows:run`: a run of a workflow in a folder that exists, with a task; `place` is where the
/// window wants the tab of its first step. The checks read files: off the window's thread.
#[tauri::command]
pub async fn workflows_run(app: AppHandle, input: Option<Value>, place: Option<Value>) -> CmdResult<WorkflowRun> {
    let input = validate::start_run_input(&input.unwrap_or(Value::Null))?;
    let place = place.as_ref().and_then(validate::place);
    super::blocking(&app, move |state| Ok(run_workflow(state, &input, place)?)).await
}

/// `workflows:approve`: the run passes the gate it waits at.
#[tauri::command]
pub fn workflows_approve(state: State<'_, AppState>, run_id: String) -> CmdResult<()> {
    state.runs.approve(validate::id(&run_id, "run")?);
    Ok(())
}

/// `workflows:retry`: the step that paused runs again.
#[tauri::command]
pub fn workflows_retry(state: State<'_, AppState>, run_id: String) -> CmdResult<()> {
    state.runs.retry(validate::id(&run_id, "run")?);
    Ok(())
}

/// `workflows:stop`.
#[tauri::command]
pub fn workflows_stop(state: State<'_, AppState>, run_id: String) -> CmdResult<()> {
    state.runs.stop(validate::id(&run_id, "run")?);
    Ok(())
}

/// `workflows:dismiss`: a run that is over leaves the list.
#[tauri::command]
pub fn workflows_dismiss(state: State<'_, AppState>, run_id: String) -> CmdResult<()> {
    state.runs.dismiss(validate::id(&run_id, "run")?);
    Ok(())
}

/// `workflows:takeAnswer`: what the current step's tab last said becomes its output; false when
/// there is nothing to take.
#[tauri::command]
pub async fn workflows_take_answer(state: State<'_, AppState>, run_id: String) -> CmdResult<bool> {
    let run_id = validate::id(&run_id, "run")?;
    Ok(state.runs.take_answer(run_id).await)
}

/// `workflows:output`: what a step of a run gave, while the run is on the list (it is the text
/// that was also written to the step's output file); null when it gave nothing yet.
#[tauri::command]
pub fn workflows_output(state: State<'_, AppState>, run_id: String, index: Option<Value>) -> CmdResult<Option<String>> {
    let index = index.as_ref().and_then(Value::as_f64).ok_or_else(|| CmdError("Invalid step".into()))?;
    let index = validate::step_index(index)?;
    Ok(state.runs.output(validate::id(&run_id, "run")?, index))
}
