//! Commands of the `mcp` package: `mcp:list` and `mcp:copy`, the main account's user-level MCP
//! servers compared with and copied to another account.

use std::sync::PoisonError;

use mc_core::error::{AppError, AppResult};
use mc_core::types::{McpCopyResult, McpServerRow, Profile};
use mc_core::validate;
use tauri::State;

use crate::error::CmdResult;
use crate::mcp_servers::{copy_mcp_servers, diff_mcp_servers, read_user_servers_file, ClaudeMcpRunner, McpServers};
use crate::profiles::ProfileService;
use crate::state::AppState;

/// `mcpTarget`: the account the servers go to, never Main.
fn target(profiles: &ProfileService, id: &str) -> AppResult<Profile> {
    let profile = profiles.get(id).ok_or_else(|| AppError::refused("Profile not found"))?;
    if profile.is_default {
        return Err(AppError::refused("The servers are copied from this account, not to it."));
    }
    Ok(profile)
}

/// Main's servers and the target's.
fn servers_of(profiles: &ProfileService, target: &Profile) -> (McpServers, McpServers) {
    let paths = profiles.paths();
    (read_user_servers_file(&paths.claude_json_of(None)), read_user_servers_file(&paths.claude_json_of(target.config_dir.as_deref())))
}

/// `mcpList`.
pub fn list_servers(profiles: &ProfileService, id: &str) -> AppResult<Vec<McpServerRow>> {
    let target = target(profiles, id)?;
    let (main, theirs) = servers_of(profiles, &target);
    Ok(diff_mcp_servers(&main, &theirs))
}

/// `mcpCopy`: through `claude mcp` of the native install, with the target account's environment,
/// in the empty probe folder.
pub async fn copy_servers(profiles: &ProfileService, id: &str, names: &[String], claude_path: Option<String>) -> AppResult<Vec<McpCopyResult>> {
    let target = target(profiles, id)?;
    let claude = claude_path.filter(|p| !p.is_empty()).ok_or_else(|| AppError::refused("claude was not found. Set its path in Settings."))?;
    let probe = profiles.paths().probe_dir();
    std::fs::create_dir_all(&probe)?;
    let runner = ClaudeMcpRunner::new(&claude, profiles.env_for(&target), probe)?;
    let (main, theirs) = servers_of(profiles, &target);
    Ok(copy_mcp_servers(names, &main, &theirs, &runner).await)
}

/// `mcp:list`: what copying Main's servers would do to the account, by name and transport only.
#[tauri::command]
pub async fn mcp_list(state: State<'_, AppState>, profile_id: String) -> CmdResult<Vec<McpServerRow>> {
    let id = validate::id(&profile_id, "profile")?;
    Ok(list_servers(&state.profiles, id)?)
}

/// `mcp:copy`: copies the named servers of Main to the account.
#[tauri::command]
pub async fn mcp_copy(state: State<'_, AppState>, profile_id: String, names: Vec<String>) -> CmdResult<Vec<McpCopyResult>> {
    let names: Vec<String> = validate::mcp_names(&names)?.into_iter().map(str::to_owned).collect();
    let id = validate::id(&profile_id, "profile")?;
    let claude = state.claude.read().unwrap_or_else(PoisonError::into_inner).path.clone();
    Ok(copy_servers(&state.profiles, id, &names, claude).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_store::ConfigStore;
    use crate::paths::Paths;
    use crate::token_vault::MemoryTokenStore;
    use mc_core::types::{CreateProfileInput, McpServerStatus};
    use std::sync::Arc;

    #[test]
    fn lists_and_refuses_as_the_electron_version_did() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".claude.json"), r#"{"mcpServers":{"unity":{"command":"node"}}}"#).unwrap();
        let root = tmp.path().join("mc");
        let profiles =
            ProfileService::with_parts(None, Paths::with_home(root.clone(), home), Arc::new(ConfigStore::new(root)), Arc::new(MemoryTokenStore::new()));
        let input = CreateProfileInput {
            name: "Work".into(),
            color: "#5b9bd5".into(),
            auth: None,
            token: None,
            email: None,
            share_dirs: vec![],
            share_files: vec![],
            keep_model_separate: false,
        };
        let (work, _) = profiles.create(&input).unwrap();

        let rows = list_servers(&profiles, &work.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, McpServerStatus::Missing);
        assert_eq!(list_servers(&profiles, "main").unwrap_err().to_string(), "The servers are copied from this account, not to it.");
        assert_eq!(list_servers(&profiles, "gone").unwrap_err().to_string(), "Profile not found");

        let block_on = tauri::async_runtime::block_on;
        let names = vec!["unity".to_owned()];
        assert_eq!(
            block_on(copy_servers(&profiles, &work.id, &names, None)).unwrap_err().to_string(),
            "claude was not found. Set its path in Settings."
        );
        let shim = block_on(copy_servers(&profiles, &work.id, &names, Some(r"C:\npm\claude.cmd".into())));
        assert!(shim.unwrap_err().to_string().contains("native Claude Code install"));
    }
}
