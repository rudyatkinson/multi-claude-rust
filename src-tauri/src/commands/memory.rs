//! Commands of the `memory` package: `memory:*` of `src/main/ipc.ts`, the Library's memory browser.
//! Each command validates with `mc_core::validate`, then hands the service the current accounts
//! and known folders ([`MemoryInputs::of`]) on a blocking thread (a listing or a search reads
//! every account's memory folders) and returns `CmdResult<T>`. A memory is named by its folder's
//! id and its file's name; the service resolves both against a fresh listing.

use mc_core::error::AppResult;
use mc_core::types::{AdoptResult, MemoryDetail, MemoryHit, MemoryListing, MemoryRef, MemorySaveResult, MemoryView, SKILL_LIMITS};
use mc_core::validate;
use serde_json::Value;
use tauri::AppHandle;

use crate::app::state as app_state;
use crate::error::{CmdError, CmdResult};
use crate::memory::{MemoryInputs, MemoryService};

/// Runs `work` with the memory service and what it needs to know, off the async workers.
async fn with_memory<T: Send + 'static>(app: AppHandle, work: impl FnOnce(&MemoryService, &MemoryInputs) -> AppResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_state(&app).ok_or_else(|| CmdError("The app is still starting.".into()))?;
        let inputs = MemoryInputs::of(&state);
        work(&state.memory, &inputs).map_err(CmdError::from)
    })
    .await
    .map_err(|e| CmdError(e.to_string()))?
}

/// `memory:list`: the projects with a memory in at least one account.
#[tauri::command]
pub async fn memory_list(app: AppHandle) -> CmdResult<MemoryListing> {
    with_memory(app, |memory, inputs| Ok(memory.list(inputs))).await
}

/// `memory:entries`: one memory folder, opened.
#[tauri::command]
pub async fn memory_entries(app: AppHandle, folder_id: String) -> CmdResult<MemoryView> {
    validate::memory_folder(&folder_id)?;
    with_memory(app, move |memory, inputs| memory.entries(inputs, &folder_id)).await
}

/// `memory:read`: a memory file for the editor.
#[tauri::command]
pub async fn memory_read(app: AppHandle, r#ref: Option<Value>) -> CmdResult<MemoryDetail> {
    let r = validate::memory_ref(&r#ref.unwrap_or(Value::Null))?;
    with_memory(app, move |memory, inputs| memory.read(inputs, &r)).await
}

/// `memory:save`: writes a memory file against the state the editor started from (`baseHash`, or
/// null to overwrite).
#[tauri::command]
pub async fn memory_save(app: AppHandle, r#ref: Option<Value>, text: String, base_hash: Option<String>) -> CmdResult<MemorySaveResult> {
    let r = validate::memory_ref(&r#ref.unwrap_or(Value::Null))?;
    validate::text(&text, "text", SKILL_LIMITS.file_bytes)?;
    validate::base_hash(base_hash.as_deref())?;
    with_memory(app, move |memory, inputs| memory.save(inputs, &r, &text, base_hash.as_deref())).await
}

/// `memory:create`: a new memory file with its line in MEMORY.md.
#[tauri::command]
pub async fn memory_create(app: AppHandle, folder_id: String, input: Option<Value>) -> CmdResult<MemoryRef> {
    validate::memory_folder(&folder_id)?;
    let input = validate::memory_create_input(&input.unwrap_or(Value::Null))?;
    with_memory(app, move |memory, inputs| memory.create(inputs, &folder_id, &input)).await
}

/// `memory:delete`: the memory goes to the bin and its line out of MEMORY.md.
#[tauri::command]
pub async fn memory_delete(app: AppHandle, r#ref: Option<Value>) -> CmdResult<()> {
    let r = validate::memory_ref(&r#ref.unwrap_or(Value::Null))?;
    with_memory(app, move |memory, inputs| memory.delete(inputs, &r)).await
}

/// `memory:search`: the memories of every account and project with every word of the query.
#[tauri::command]
pub async fn memory_search(app: AppHandle, query: String) -> CmdResult<Vec<MemoryHit>> {
    validate::memory_query(&query)?;
    with_memory(app, move |memory, inputs| Ok(memory.search(inputs, &query))).await
}

/// `memory:adopt`: brings an account's own memories into the main account's folder of the
/// project; only a real `true` for `apply` writes anything.
#[tauri::command]
pub async fn memory_adopt(app: AppHandle, folder_id: String, apply: Option<Value>) -> CmdResult<AdoptResult> {
    validate::memory_folder(&folder_id)?;
    let apply = apply.as_ref().is_some_and(validate::flag);
    with_memory(app, move |memory, inputs| memory.adopt(inputs, &folder_id, apply)).await
}

/// `memory:reveal`: a memory in the file manager, or its folder when no file is named.
#[tauri::command]
pub async fn memory_reveal(app: AppHandle, folder_id: String, file: Option<String>) -> CmdResult<()> {
    validate::memory_folder(&folder_id)?;
    validate::optional_str(file.as_deref(), "file", 255)?;
    with_memory(app, move |memory, inputs| memory.reveal(inputs, &folder_id, file.as_deref())).await
}
