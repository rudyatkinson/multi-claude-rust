//! Commands of the `agents` package (`agents:*` of `src/main/ipc.ts`). Each command validates what
//! the window sent with `mc_core::validate`, in the order the Electron version did, and runs the
//! work on disk off the async runtime with the roots as the app has them right now.
//!
//! Until `commands::stubs` gives up its `agents_*`, the attribute is held back as
//! `#[cfg_attr(any(), tauri::command)]`: `#[tauri::command]` exports a macro per command name at
//! the crate root, and two of one name do not compile. Swapping the registry means turning each one
//! into `#[tauri::command]`.

use mc_core::error::AppResult;
use mc_core::types::{AgentDetail, AgentListing, AgentMoved, AgentRef, AgentSaveResult, AGENT_LIMITS, SKILL_LIMITS};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::agents::{root_inputs, RootInputs};
use crate::error::{CmdError, CmdResult};
use crate::state::AppState;

/// What `agents:create` takes as a description.
const DESCRIPTION_MAX: usize = 2000;

/// Runs `work` on a blocking thread with the app's state and the roots' inputs as they are now.
async fn on_disk<T: Send + 'static>(app: &AppHandle, work: impl FnOnce(&AppState, &RootInputs) -> AppResult<T> + Send + 'static) -> CmdResult<T> {
    let app = app.clone();
    let done = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let inputs = root_inputs(&state);
        work(&state, &inputs)
    })
    .await
    .map_err(|e| CmdError(e.to_string()))?;
    Ok(done?)
}

fn agent_of(value: Option<Value>) -> AppResult<AgentRef> {
    validate::agent_ref(&value.unwrap_or(Value::Null))
}

/// `agents:list`: the agents folders of the accounts and the projects in sight, and their agents.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_list(app: AppHandle) -> CmdResult<AgentListing> {
    on_disk(&app, |state, inputs| Ok(state.agents.list(inputs))).await
}

/// `agents:read`: an agent's whole file.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_read(app: AppHandle, r#ref: Option<Value>) -> CmdResult<AgentDetail> {
    let agent = agent_of(r#ref)?;
    on_disk(&app, move |state, inputs| state.agents.read(inputs, &agent)).await
}

/// `agents:save`: writes an agent's file over the state the editor read (`baseHash`), or over
/// whatever is there when that is null.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_save(app: AppHandle, r#ref: Option<Value>, text: String, base_hash: Option<String>) -> CmdResult<AgentSaveResult> {
    let agent = agent_of(r#ref)?;
    validate::text(&text, "text", SKILL_LIMITS.file_bytes)?;
    let base = validate::base_hash(base_hash.as_deref())?.map(str::to_owned);
    on_disk(&app, move |state, inputs| state.agents.save(inputs, &agent, &text, base.as_deref())).await
}

/// `agents:create`: a new agent at the top of a root.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_create(app: AppHandle, root_id: String, name: String, description: String) -> CmdResult<AgentRef> {
    let root_id = validate::skill_root(&root_id)?.to_owned();
    validate::str(&name, "name", AGENT_LIMITS.new_name)?;
    validate::text(&description, "description", DESCRIPTION_MAX)?;
    on_disk(&app, move |state, inputs| state.agents.create(inputs, &root_id, &name, &description)).await
}

/// `agents:duplicate`: a copy next to the original, under another name.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_duplicate(app: AppHandle, r#ref: Option<Value>, name: String) -> CmdResult<AgentRef> {
    let agent = agent_of(r#ref)?;
    validate::str(&name, "name", AGENT_LIMITS.new_name)?;
    on_disk(&app, move |state, inputs| state.agents.duplicate(inputs, &agent, &name)).await
}

/// `agents:rename`: another name in the file, and for the file when it was named after the agent.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_rename(app: AppHandle, r#ref: Option<Value>, name: String) -> CmdResult<AgentMoved> {
    let agent = agent_of(r#ref)?;
    validate::str(&name, "name", AGENT_LIMITS.new_name)?;
    on_disk(&app, move |state, inputs| state.agents.rename(inputs, &agent, &name)).await
}

/// `agents:delete`: the agent's file into the bin.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_delete(app: AppHandle, r#ref: Option<Value>) -> CmdResult<()> {
    let agent = agent_of(r#ref)?;
    on_disk(&app, move |state, inputs| state.agents.remove(inputs, &agent)).await
}

/// `agents:transfer`: a copy in another root, or a move there; an empty name keeps the agent's own.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_transfer(app: AppHandle, r#ref: Option<Value>, to_root_id: String, mode: String, name: String) -> CmdResult<AgentMoved> {
    let mode = validate::transfer_mode(&mode)?;
    let agent = agent_of(r#ref)?;
    let to_root_id = validate::skill_root(&to_root_id)?.to_owned();
    validate::text(&name, "name", AGENT_LIMITS.name)?;
    on_disk(&app, move |state, inputs| state.agents.transfer(inputs, &agent, &to_root_id, mode, &name)).await
}

/// `agents:reveal`: the agent's file in the file manager.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_reveal(app: AppHandle, r#ref: Option<Value>) -> CmdResult<()> {
    let agent = agent_of(r#ref)?;
    on_disk(&app, move |state, inputs| state.agents.reveal(inputs, &agent)).await
}

/// `agents:addFolder`: asks for a project folder and adds it to the Library's list; the id of its
/// agents root, or null.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_add_folder(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    let state = &*state;
    Ok(state.agents.add_folder(state.profiles.config(), || root_inputs(state)).await?)
}

/// `agents:removeFolder`: takes a project's folder out of the Library's list.
#[cfg_attr(any(), tauri::command)]
pub async fn agents_remove_folder(app: AppHandle, root_id: String) -> CmdResult<()> {
    let root_id = validate::skill_root(&root_id)?.to_owned();
    on_disk(&app, move |state, inputs| state.agents.remove_folder(state.profiles.config(), inputs, &root_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tauri runs async commands on its runtime, which needs their futures to be `Send`. Compiled,
    /// never run.
    #[allow(dead_code)]
    fn futures_are_send(app: AppHandle, state: State<'_, AppState>) {
        fn send<T: Send>(_: T) {}
        send(agents_list(app.clone()));
        send(agents_read(app.clone(), None));
        send(agents_save(app.clone(), None, String::new(), None));
        send(agents_create(app.clone(), String::new(), String::new(), String::new()));
        send(agents_duplicate(app.clone(), None, String::new()));
        send(agents_rename(app.clone(), None, String::new()));
        send(agents_delete(app.clone(), None));
        send(agents_transfer(app.clone(), None, String::new(), String::new(), String::new()));
        send(agents_reveal(app.clone(), None));
        send(agents_remove_folder(app, String::new()));
        send(agents_add_folder(state));
    }
}
