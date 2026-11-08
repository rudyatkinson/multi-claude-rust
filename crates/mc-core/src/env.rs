//! Environment of a Claude Code session: the variables removed and set (`src/main/env.ts`).
//!
//! Names are matched as the TypeScript matched them: the account variables by their upper-case
//! form (`toUpperCase`, Unicode), the noise by ASCII-case-insensitive patterns (`/.../i`). The
//! result is an ordinary case-sensitive map, as the TypeScript result was a plain object.

use std::collections::BTreeMap;
use std::path::Path;

use crate::types::{AuthKind, Profile};

/// Variables that select or override the account; removed from every spawned process.
pub const AUTH_SCRUB: [&str; 8] = [
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

/// One `/^...$/i` or `/^.../i` pattern: a whole name or a prefix, ASCII letters in any case.
enum Pattern {
    Exact(&'static str),
    Prefix(&'static str),
}

impl Pattern {
    fn test(&self, name: &str) -> bool {
        // the patterns are ASCII; a non-ASCII character never folds onto an ASCII one in a
        // JavaScript regular expression without the `u` flag, and its bytes never equal ASCII
        match *self {
            Pattern::Exact(p) => name.eq_ignore_ascii_case(p),
            Pattern::Prefix(p) => name.len() >= p.len() && name.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes()),
        }
    }
}

/// Leaks from a parent Claude Code session (when the app is started from inside one) and from the dev tooling.
const NOISE_SCRUB: [Pattern; 12] = [
    Pattern::Exact("CLAUDECODE"),
    Pattern::Exact("CLAUDE_PID"),
    Pattern::Exact("CLAUDE_EFFORT"),
    // /^CLAUDE_CODE_(SESSION|MESSAGING|CHILD|ENTRYPOINT|EXECPATH)/i
    Pattern::Prefix("CLAUDE_CODE_SESSION"),
    Pattern::Prefix("CLAUDE_CODE_MESSAGING"),
    Pattern::Prefix("CLAUDE_CODE_CHILD"),
    Pattern::Prefix("CLAUDE_CODE_ENTRYPOINT"),
    Pattern::Prefix("CLAUDE_CODE_EXECPATH"),
    Pattern::Prefix("ELECTRON_"),
    Pattern::Prefix("VITE_"),
    Pattern::Exact("NODE_ENV_ELECTRON_VITE"),
    Pattern::Exact("MULTI_CLAUDE_HOME"),
];

/// The dev run's tooling sets these; they must not reach the user's builds inside a session.
const DEV_SCRUB: [Pattern; 2] = [Pattern::Exact("NODE_ENV"), Pattern::Exact("NODE_OPTIONS")];

/// `isScrubbed`: whether a variable is kept out of the processes that run claude.
pub fn is_scrubbed(name: &str, dev: bool) -> bool {
    let upper = name.to_uppercase();
    if AUTH_SCRUB.contains(&upper.as_str()) {
        return true;
    }
    if NOISE_SCRUB.iter().any(|p| p.test(name)) {
        return true;
    }
    dev && DEV_SCRUB.iter().any(|p| p.test(name))
}

/// `buildClaudeEnv`: the environment for any process that runs claude for a profile.
/// `profile_config_dir` is None for Main, which must run without `CLAUDE_CONFIG_DIR`. A token
/// (of a token account) is added after the scrub, so a token that happens to be in the app's own
/// environment never reaches another account. An empty directory or token counts as none.
pub fn build_claude_env(
    base: &BTreeMap<String, String>,
    profile_config_dir: Option<&Path>,
    token: Option<&str>,
    dev: bool,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> =
        base.iter().filter(|(key, _)| !is_scrubbed(key, dev)).map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Some(dir) = profile_config_dir.filter(|d| !d.as_os_str().is_empty()) {
        env.insert("CLAUDE_CONFIG_DIR".into(), dir.to_string_lossy().into_owned());
    }
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    if let Some(token) = token.filter(|t| !t.is_empty()) {
        env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), token.into());
    }
    env
}

/// `buildProfileEnv`: the environment for an account's processes. Only a token account with a
/// stored token that decrypts gets a token.
pub fn build_profile_env(
    base: &BTreeMap<String, String>,
    profile: &Profile,
    decrypt: impl FnOnce(&str) -> Option<String>,
    dev: bool,
) -> BTreeMap<String, String> {
    let token = match (profile.auth, profile.token_enc.as_deref()) {
        (Some(AuthKind::Token), Some(enc)) if !enc.is_empty() => decrypt(enc),
        _ => None,
    };
    build_claude_env(base, profile.config_dir.as_deref().map(Path::new), token.as_deref(), dev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Extra, ShareConfig};

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn base() -> BTreeMap<String, String> {
        map(&[
            ("PATH", "/usr/bin"),
            ("Claude_Code_OAuth_Token", "secret"),
            ("ANTHROPIC_API_KEY", "key"),
            ("anthropic_auth_token", "token"),
            ("CLAUDE_CONFIG_DIR", "/somewhere/else"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_SESSION_ID", "abc"),
            ("CLAUDE_CODE_MESSAGING_TOKEN", "abc"),
            ("CLAUDE_CODE_GIT_BASH_PATH", "C:/Git/bin/bash.exe"),
            ("ELECTRON_RUN_AS_NODE", "1"),
            ("NODE_ENV", "development"),
        ])
    }

    // ---------- test/env.test.ts: buildClaudeEnv ----------

    #[test]
    fn removes_account_overrides_regardless_of_case() {
        let env = build_claude_env(&base(), None, None, false);
        assert!(!env.contains_key("Claude_Code_OAuth_Token"));
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
        assert!(!env.contains_key("anthropic_auth_token"));
    }

    #[test]
    fn runs_main_without_config_dir_and_other_profiles_with_their_own() {
        assert!(!build_claude_env(&base(), None, None, false).contains_key("CLAUDE_CONFIG_DIR"));
        let env = build_claude_env(&base(), Some(Path::new("/profiles/work")), None, false);
        assert_eq!(env["CLAUDE_CONFIG_DIR"], "/profiles/work");
    }

    #[test]
    fn drops_variables_leaked_by_a_parent_session_but_keeps_user_configuration() {
        let env = build_claude_env(&base(), None, None, false);
        assert!(!env.contains_key("CLAUDECODE"));
        assert!(!env.contains_key("CLAUDE_CODE_SESSION_ID"));
        assert!(!env.contains_key("CLAUDE_CODE_MESSAGING_TOKEN"));
        assert!(!env.contains_key("ELECTRON_RUN_AS_NODE"));
        assert_eq!(env["CLAUDE_CODE_GIT_BASH_PATH"], "C:/Git/bin/bash.exe");
        assert_eq!(env["PATH"], "/usr/bin");
    }

    #[test]
    fn removes_the_dev_server_node_env_only_in_dev() {
        assert_eq!(build_claude_env(&base(), None, None, false)["NODE_ENV"], "development");
        assert!(!build_claude_env(&base(), None, None, true).contains_key("NODE_ENV"));
    }

    #[test]
    fn sets_terminal_capabilities() {
        // (`UNDEFINED_VALUE: undefined` of the TypeScript test has no counterpart in a map of texts)
        let env = build_claude_env(&base(), None, None, false);
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["COLORTERM"], "truecolor");
    }

    #[test]
    fn noise_patterns_match_as_the_regular_expressions_did() {
        for name in [
            "claudecode",
            "Claude_Pid",
            "CLAUDE_EFFORT",
            "claude_code_entrypoint",
            "CLAUDE_CODE_EXECPATH_X",
            "CLAUDE_CODE_CHILD",
            "electron_x",
            "Vite_PORT",
            "node_env_electron_vite",
            "multi_claude_home",
        ] {
            assert!(is_scrubbed(name, false), "{name}");
        }
        for name in ["CLAUDECODE_X", "CLAUDE_PID_2", "CLAUDE_CODE_GIT_BASH_PATH", "MY_ELECTRON_", "NODE_ENV", "NODE_OPTIONS", "MULTI_CLAUDE_HOMEX"] {
            assert!(!is_scrubbed(name, false), "{name}");
        }
        assert!(is_scrubbed("node_options", true));
        // toUpperCase is Unicode-aware for the account variables: a dotless i becomes I
        assert!(is_scrubbed("anthrop\u{131}c_api_key", false));
        // the regular expressions are not: a long s does not fold onto S
        assert!(!is_scrubbed("CLAUDE_CODE_\u{17f}ESSION", false));
    }

    #[test]
    fn empty_directory_or_token_counts_as_none() {
        let env = build_claude_env(&BTreeMap::new(), Some(Path::new("")), Some(""), false);
        assert!(!env.contains_key("CLAUDE_CONFIG_DIR"));
        assert!(!env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
    }

    // ---------- test/token.test.ts: buildProfileEnv ----------

    const TOKEN: &str = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz";

    fn decrypt(encrypted: &str) -> Option<String> {
        encrypted.strip_prefix("enc:").map(str::to_owned)
    }

    fn profile(config_dir: &str, auth: Option<AuthKind>, token_enc: Option<&str>) -> Profile {
        Profile {
            id: "p".into(),
            name: "P".into(),
            slug: "p".into(),
            color: "#000000".into(),
            is_default: false,
            config_dir: Some(config_dir.into()),
            share: ShareConfig::default(),
            email_hint: None,
            auth,
            token_enc: token_enc.map(str::to_owned),
            last_sync: None,
            account: None,
            share_memory: None,
            extra: Extra::new(),
        }
    }

    #[test]
    fn gives_a_token_account_its_own_token_replacing_one_from_the_environment() {
        let enc = format!("enc:{TOKEN}");
        let env = build_profile_env(
            &map(&[("PATH", "/bin"), ("CLAUDE_CODE_OAUTH_TOKEN", "someone-elses-token")]),
            &profile("/profiles/work", Some(AuthKind::Token), Some(&enc)),
            decrypt,
            false,
        );
        assert_eq!(env["CLAUDE_CODE_OAUTH_TOKEN"], TOKEN);
        assert_eq!(env["CLAUDE_CONFIG_DIR"], "/profiles/work");
    }

    #[test]
    fn passes_no_token_to_a_login_account_even_when_one_is_in_the_environment() {
        let env = build_profile_env(
            &map(&[("CLAUDE_CODE_OAUTH_TOKEN", "someone-elses-token")]),
            &profile("/profiles/home", None, None),
            decrypt,
            false,
        );
        assert!(!env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
    }

    #[test]
    fn passes_no_token_when_the_stored_one_cannot_be_decrypted() {
        let env = build_profile_env(&BTreeMap::new(), &profile("/profiles/work", Some(AuthKind::Token), Some("garbage")), decrypt, false);
        assert!(!env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
    }
}
