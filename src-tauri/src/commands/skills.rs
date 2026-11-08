//! Commands of the `skills` package (`skills:*` of `src/main/ipc.ts` and the skill handlers of
//! `src/main/index.ts`). Each command validates with `mc_core::validate`, then works through
//! `AppState::skills` on a blocking thread (a listing reads every skill's SKILL.md, an archive may
//! be 50 MB); the service resolves every ref against the roots as they are right now.

use mc_core::types::{ImportPreview, SaveResult, SessionInfo, SkillDetail, SkillListing, SkillMoved, SkillRef, SKILL_LIMITS};
use mc_core::validate;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::app::launch::start_session;
use crate::app::remember_folder;
use crate::app::window::main_window;
use crate::error::{CmdError, CmdResult};
use crate::library::known_folders::{add_library_folder, remove_library_folder};
use crate::library::roots::RootSub;
use crate::skills::archive::ARCHIVE_LIMITS;
use crate::skills::{AssistRequest, FileAction};
use crate::state::AppState;

/// The longest description a new skill may be given.
const DESCRIPTION_MAX: usize = 2000;
/// The longest path of a file of a skill.
const FILE_MAX: usize = 400;
/// The longest token of a chosen archive.
const TOKEN_MAX: usize = 64;

/// Runs `work` with the app's state off the async workers.
async fn blocking<T: Send + 'static>(app: AppHandle, work: impl FnOnce(&AppState) -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = crate::app::state(&app).ok_or_else(|| CmdError("The app is still starting.".into()))?;
        work(&state)
    })
    .await
    .map_err(|e| CmdError(e.to_string()))?
}

fn skill_ref(value: Option<Value>) -> CmdResult<SkillRef> {
    Ok(validate::skill_ref(&value.unwrap_or(Value::Null))?)
}

fn name_of(value: &str) -> CmdResult<String> {
    Ok(validate::str(value, "name", SKILL_LIMITS.name)?.to_owned())
}

fn opener_failed(err: tauri_plugin_opener::Error) -> CmdError {
    CmdError(err.to_string())
}

/// `skills:list`: the roots and the skills of each.
#[tauri::command]
pub async fn skills_list(app: AppHandle) -> CmdResult<SkillListing> {
    blocking(app, |state| Ok(state.skills.list())).await
}

/// `skills:read`: a skill's SKILL.md and its other files.
#[tauri::command]
pub async fn skills_read(app: AppHandle, r#ref: Option<Value>) -> CmdResult<SkillDetail> {
    let r = skill_ref(r#ref)?;
    blocking(app, move |state| Ok(state.skills.read(&r)?)).await
}

/// `skills:save`: writes the SKILL.md over the state `baseHash` names (null overwrites).
#[tauri::command]
pub async fn skills_save(app: AppHandle, r#ref: Option<Value>, text: String, base_hash: Option<String>) -> CmdResult<SaveResult> {
    let r = skill_ref(r#ref)?;
    validate::text(&text, "text", SKILL_LIMITS.file_bytes)?;
    let base = validate::base_hash(base_hash.as_deref())?.map(str::to_owned);
    blocking(app, move |state| Ok(state.skills.save(&r, &text, base.as_deref())?)).await
}

/// `skills:create`: a new skill with a first text.
#[tauri::command]
pub async fn skills_create(app: AppHandle, root_id: String, name: String, description: String) -> CmdResult<SkillRef> {
    validate::skill_root(&root_id)?;
    let name = name_of(&name)?;
    validate::text(&description, "description", DESCRIPTION_MAX)?;
    blocking(app, move |state| Ok(state.skills.create(&root_id, &name, &description)?)).await
}

/// `skills:duplicate`: a copy in the same folder under another name.
#[tauri::command]
pub async fn skills_duplicate(app: AppHandle, r#ref: Option<Value>, name: String) -> CmdResult<SkillRef> {
    let r = skill_ref(r#ref)?;
    let name = name_of(&name)?;
    blocking(app, move |state| Ok(state.skills.duplicate(&r, &name)?)).await
}

/// `skills:rename`: another folder name, and the name in the file when it was the folder's.
#[tauri::command]
pub async fn skills_rename(app: AppHandle, r#ref: Option<Value>, name: String) -> CmdResult<SkillMoved> {
    let r = skill_ref(r#ref)?;
    let name = name_of(&name)?;
    blocking(app, move |state| Ok(state.skills.rename(&r, &name)?)).await
}

/// `skills:delete`: into the bin; a linked skill loses only its link.
#[tauri::command]
pub async fn skills_delete(app: AppHandle, r#ref: Option<Value>) -> CmdResult<()> {
    let r = skill_ref(r#ref)?;
    blocking(app, move |state| Ok(state.skills.remove(&r)?)).await
}

/// `skills:transfer`: a copy in another folder, or a move there.
#[tauri::command]
pub async fn skills_transfer(app: AppHandle, r#ref: Option<Value>, to_root_id: String, mode: String, name: String) -> CmdResult<SkillMoved> {
    let mode = validate::transfer_mode(&mode)?;
    let r = skill_ref(r#ref)?;
    validate::skill_root(&to_root_id)?;
    let name = name_of(&name)?;
    blocking(app, move |state| Ok(state.skills.transfer(&r, &to_root_id, mode, &name)?)).await
}

/// `skills:export`: packs the skill into a zip where the user says (Downloads by default); the
/// path written, or null when that was cancelled.
#[tauri::command]
pub async fn skills_export(app: AppHandle, state: State<'_, AppState>, r#ref: Option<Value>) -> CmdResult<Option<String>> {
    let r = skill_ref(r#ref)?;
    let Some(win) = main_window(&app) else { return Ok(None) };
    // the skill must be there before anything is asked
    state.skills.dir_of(&r)?;
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Export skill")
        .set_parent(&win)
        .set_file_name(format!("{}.zip", r.dir_name))
        .add_filter("Skill archive", &["zip", "skill"]);
    if let Some(downloads) = app.path().download_dir().ok().or_else(dirs::download_dir) {
        dialog = dialog.set_directory(downloads);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file()).await.map_err(|e| CmdError(e.to_string()))?;
    let Some(target) = picked.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    blocking(app, move |state| {
        std::fs::write(&target, state.skills.pack(&r)?)?;
        Ok(Some(target.to_string_lossy().into_owned()))
    })
    .await
}

/// `skills:pickImport`: asks for an archive (`.zip` or `.skill`, at most 50 MB), checks it and
/// keeps it under a token; what it holds, or null when that was cancelled.
#[tauri::command]
pub async fn skills_pick_import(app: AppHandle) -> CmdResult<Option<ImportPreview>> {
    let Some(win) = main_window(&app) else { return Ok(None) };
    let dialog = app.dialog().file().set_title("Import skill").set_parent(&win).add_filter("Skill archive", &["zip", "skill"]);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file()).await.map_err(|e| CmdError(e.to_string()))?;
    let Some(file) = picked.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    blocking(app, move |state| {
        if std::fs::metadata(&file)?.len() > ARCHIVE_LIMITS.archive_bytes {
            return Err(CmdError("The archive is larger than 50 MB.".into()));
        }
        let bytes = std::fs::read(&file)?;
        let file_name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Some(state.skills.preview_import(&bytes, &file_name)?))
    })
    .await
}

/// `skills:import`: brings the archive of `token` in as the skill `name` of a root.
#[tauri::command]
pub async fn skills_import(app: AppHandle, token: String, root_id: String, name: String) -> CmdResult<SkillRef> {
    validate::str(&token, "archive", TOKEN_MAX)?;
    validate::skill_root(&root_id)?;
    let name = name_of(&name)?;
    blocking(app, move |state| Ok(state.skills.import(&token, &root_id, &name)?)).await
}

/// `skills:reveal`: a file of the skill in the file manager, or the skill's folder opened.
#[tauri::command]
pub async fn skills_reveal(app: AppHandle, r#ref: Option<Value>, file: Option<String>) -> CmdResult<()> {
    let r = skill_ref(r#ref)?;
    validate::optional_str(file.as_deref(), "file", FILE_MAX)?;
    let opener = app.clone();
    blocking(app, move |state| match file {
        Some(file) => opener.opener().reveal_item_in_dir(state.skills.file_of(&r, &file)?).map_err(opener_failed),
        None => opener.opener().open_path(state.skills.dir_of(&r)?.to_string_lossy(), None::<&str>).map_err(opener_failed),
    })
    .await
}

/// `skills:openFile`: a text file of the skill opens with its program; anything else (a script
/// above all, which "opening" would run) is shown in the file manager.
#[tauri::command]
pub async fn skills_open_file(app: AppHandle, r#ref: Option<Value>, file: String) -> CmdResult<()> {
    let r = skill_ref(r#ref)?;
    validate::str(&file, "file", FILE_MAX)?;
    let opener = app.clone();
    blocking(app, move |state| match state.skills.file_action(&r, &file)? {
        FileAction::Open(target) => opener.opener().open_path(target.to_string_lossy(), None::<&str>).map_err(opener_failed),
        FileAction::Reveal(target) => opener.opener().reveal_item_in_dir(target).map_err(opener_failed),
    })
    .await
}

/// `skills:assist`: a session of the account about the skill, with the request in a file; the
/// project goes to the recent folders.
#[tauri::command]
pub async fn skills_assist(app: AppHandle, r#ref: Option<Value>, profile_id: String, request: String, place: Option<Value>) -> CmdResult<SessionInfo> {
    let r = skill_ref(r#ref)?;
    validate::id(&profile_id, "profile")?;
    validate::str(&request, "request", SKILL_LIMITS.request)?;
    let place = place.as_ref().and_then(validate::place);
    blocking(app, move |state| {
        let profile_name = state.profiles.get(&profile_id).map(|p| p.name);
        let claude_path = state.claude_path();
        let ask = AssistRequest {
            r: &r,
            profile_id: &profile_id,
            profile_name: profile_name.as_deref(),
            request: &request,
            place,
            claude_path: claude_path.as_deref(),
        };
        let asked = state.skills.assist(ask, |request, place| start_session(state, request, place))?;
        if let Some(folder) = &asked.project {
            remember_folder(state, folder);
        }
        Ok(asked.info)
    })
    .await
}

/// `skills:addFolder`: asks for a project folder and keeps it in the Library's list; the id of its
/// skills root, or null (cancelled, or the folder's skills are an account's own).
#[tauri::command]
pub async fn skills_add_folder(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<String>> {
    add_library_folder(&app, &state, RootSub::Skills).await
}

/// `skills:removeFolder`: takes a project folder out of the Library's list.
#[tauri::command]
pub async fn skills_remove_folder(app: AppHandle, root_id: String) -> CmdResult<()> {
    validate::skill_root(&root_id)?;
    blocking(app, move |state| Ok(remove_library_folder(state, &root_id, RootSub::Skills)?)).await
}
