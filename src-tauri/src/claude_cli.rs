//! `claude auth status --json` and `claude auth logout` (`src/main/claudeCli.ts`). The login
//! itself runs in a session tab (`SessionKind::Login`, see `sessions`).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mc_core::auth_status::parse_auth_status;
use mc_core::types::AccountInfo;

use crate::claude_binary::{is_dev, process_env, run_claude};

/// `auth status` and `auth logout` get this long.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(20);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default()
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// The environment for an account's claude processes (`envFor`): the app's environment
/// scrubbed, `CLAUDE_CONFIG_DIR` for any account but Main (`config_dir` None), and the token of a
/// token account (already decrypted).
pub fn profile_env(config_dir: Option<&Path>, token: Option<&str>) -> BTreeMap<String, String> {
    mc_core::env::build_claude_env(&process_env(), config_dir, token, is_dev())
}

/// `authStatus`: the account as `claude auth status --json` reports it. The command exits 1 when
/// logged out but still prints JSON, so its output is read whatever the exit code; the error of
/// the run only replaces the parse error when nothing could be read.
pub async fn auth_status(claude: &str, env: &BTreeMap<String, String>) -> AccountInfo {
    let out = run_claude(claude, &strings(&["auth", "status", "--json"]), Some(env), None, AUTH_TIMEOUT).await;
    let mut info = parse_auth_status(&out.stdout, now_ms());
    if let (Some(run_error), Some(_)) = (out.error, info.error.as_ref()) {
        info.error = Some(run_error);
    }
    info
}

/// `authLogout`: signs the account out. Whatever happens, it is over when this returns.
pub async fn auth_logout(claude: &str, env: &BTreeMap<String, String>) {
    let out = run_claude(claude, &strings(&["auth", "logout"]), Some(env), None, AUTH_TIMEOUT).await;
    if let Some(e) = out.error {
        log::info!("claude auth logout: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_environment_is_scrubbed() {
        let env = profile_env(Some(Path::new("/profiles/work")), Some("tok"));
        assert_eq!(env["CLAUDE_CONFIG_DIR"], "/profiles/work");
        assert_eq!(env["CLAUDE_CODE_OAUTH_TOKEN"], "tok");
        assert_eq!(env["TERM"], "xterm-256color");
        assert!(!profile_env(None, None).contains_key("CLAUDE_CONFIG_DIR"));
    }

    #[test]
    fn a_status_that_cannot_run_is_a_logged_out_account_with_the_reason() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let info = rt.block_on(auth_status("surely-not-claude-mc", &profile_env(None, None)));
        assert!(!info.logged_in);
        assert!(info.error.as_deref().unwrap().starts_with("spawn surely-not-claude-mc"));
        assert!(info.checked_at > 0);
    }
}
