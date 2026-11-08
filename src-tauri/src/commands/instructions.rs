//! Commands of the `instructions` package: `instructions:*`, the CLAUDE.md files and rules of the
//! accounts and the projects, and what a session reads.
//!
//! The commands run on the async runtime, and the work itself on a blocking thread: it reads and
//! writes files, walks rules folders and, for `addFolder`, waits for the folder dialog.

use mc_core::error::AppResult;
use mc_core::types::{InstructionDetail, InstructionFile, InstructionRef, InstructionSaveResult, InstructionScope, LoadStep, SKILL_LIMITS};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::error::{CmdError, CmdResult};
use crate::instructions::InstructionService;
use crate::state::AppState;

/// Runs `work` on the instruction service, off the async runtime.
async fn on_service<T: Send + 'static>(app: AppHandle, work: impl FnOnce(&InstructionService) -> AppResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.try_state::<AppState>().ok_or_else(|| CmdError("The app is still starting.".into()))?;
        work(&state.instructions).map_err(CmdError::from)
    })
    .await
    .map_err(|e| CmdError(e.to_string()))?
}

/// `instructions:scopes`: the accounts, then the project folders in sight.
#[tauri::command]
pub async fn instructions_scopes(app: AppHandle) -> CmdResult<Vec<InstructionScope>> {
    on_service(app, |s| Ok(s.scopes())).await
}

/// `instructions:files`: a scope's fixed files, there or not, then its rules.
#[tauri::command]
pub async fn instructions_files(app: AppHandle, scope_id: String) -> CmdResult<Vec<InstructionFile>> {
    let scope_id = validate::instruction_scope(&scope_id)?.to_owned();
    on_service(app, move |s| s.files(&scope_id)).await
}

/// `instructions:read`.
#[tauri::command]
pub async fn instructions_read(app: AppHandle, r#ref: Option<Value>) -> CmdResult<InstructionDetail> {
    let r = validate::instruction_ref(&r#ref.unwrap_or(Value::Null))?;
    on_service(app, move |s| s.read(&r)).await
}

/// `instructions:save`: against the state the editor started from (`baseHash`), or over it (null).
#[tauri::command]
pub async fn instructions_save(app: AppHandle, r#ref: Option<Value>, text: String, base_hash: Option<String>) -> CmdResult<InstructionSaveResult> {
    let r = validate::instruction_ref(&r#ref.unwrap_or(Value::Null))?;
    validate::text(&text, "text", SKILL_LIMITS.file_bytes)?;
    let base_hash = validate::base_hash(base_hash.as_deref())?.map(str::to_owned);
    on_service(app, move |s| s.save(&r, &text, base_hash.as_deref())).await
}

/// `instructions:create`: `{ file }` for a fixed file not there yet, `{ rule }` for a new rule.
#[tauri::command]
pub async fn instructions_create(app: AppHandle, scope_id: String, target: Option<Value>) -> CmdResult<InstructionRef> {
    let scope_id = validate::instruction_scope(&scope_id)?.to_owned();
    let target = validate::instruction_target(&target.unwrap_or(Value::Null))?;
    on_service(app, move |s| s.create(&scope_id, &target)).await
}

/// `instructions:delete`: into the bin.
#[tauri::command]
pub async fn instructions_delete(app: AppHandle, r#ref: Option<Value>) -> CmdResult<()> {
    let r = validate::instruction_ref(&r#ref.unwrap_or(Value::Null))?;
    on_service(app, move |s| s.delete(&r)).await
}

/// `instructions:view`: what a session of the account reads in the project folder, in order.
#[tauri::command]
pub async fn instructions_view(app: AppHandle, profile_id: String, scope_id: String) -> CmdResult<Vec<LoadStep>> {
    let profile_id = validate::id(&profile_id, "profile")?.to_owned();
    let scope_id = validate::instruction_scope(&scope_id)?.to_owned();
    on_service(app, move |s| s.view(&profile_id, &scope_id)).await
}

/// `instructions:reveal`: a file of the scope in the file manager, or the scope's folder opened.
#[tauri::command]
pub async fn instructions_reveal(app: AppHandle, scope_id: String, file: Option<String>) -> CmdResult<()> {
    let scope_id = validate::instruction_scope(&scope_id)?.to_owned();
    let file = validate::optional_str(file.as_deref(), "file", 400)?.map(str::to_owned);
    on_service(app, move |s| s.reveal(&scope_id, file.as_deref())).await
}

/// `instructions:addFolder`: asks for a project folder and keeps it in the Library's list; the id
/// of its instruction files, null when cancelled.
#[tauri::command]
pub async fn instructions_add_folder(app: AppHandle) -> CmdResult<Option<String>> {
    on_service(app, InstructionService::add_folder).await
}
