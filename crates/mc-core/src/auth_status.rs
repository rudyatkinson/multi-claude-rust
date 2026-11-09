//! `parse_auth_status` (`src/main/claudeCli.ts`).

use serde_json::Value;

use crate::types::AccountInfo;

const UNREADABLE: &str = "Could not read the status";

/// `str`: a text with something in it, else nothing.
fn text(json: &Value, key: &str) -> Option<String> {
    json.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// `parseAuthStatus`: reads `claude auth status --json`, which exits 1 when logged out but still
/// prints JSON. The JSON is the text between the first `{` and the last `}`, so that warnings
/// around it do not matter. Anything unreadable is a logged-out account with an error.
pub fn parse_auth_status(stdout: &str, now: i64) -> AccountInfo {
    let unreadable = || AccountInfo {
        logged_in: false,
        email: None,
        org_id: None,
        org_name: None,
        subscription_type: None,
        plan_tier: None,
        auth_method: None,
        checked_at: now,
        error: Some(UNREADABLE.to_owned()),
    };
    let (Some(start), Some(end)) = (stdout.find('{'), stdout.rfind('}')) else {
        return unreadable();
    };
    if end <= start {
        return unreadable();
    }
    let Ok(json) = serde_json::from_str::<Value>(&stdout[start..=end]) else {
        return unreadable();
    };
    AccountInfo {
        logged_in: json.get("loggedIn") == Some(&Value::Bool(true)),
        email: text(&json, "email"),
        org_id: text(&json, "orgId"),
        org_name: text(&json, "orgName"),
        subscription_type: text(&json, "subscriptionType"),
        plan_tier: text(&json, "planTier"),
        auth_method: text(&json, "authMethod"),
        checked_at: now,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logged_out(now: i64) -> AccountInfo {
        AccountInfo {
            logged_in: false,
            email: None,
            org_id: None,
            org_name: None,
            subscription_type: None,
        plan_tier: None,
            auth_method: None,
            checked_at: now,
            error: None,
        }
    }

    // ---------- test/misc.test.ts: parseAuthStatus ----------

    #[test]
    fn reads_a_logged_in_account() {
        let out = r#"{"loggedIn":true,"authMethod":"claude.ai","email":"a@b.co","orgId":"o1","orgName":"Org","subscriptionType":"max","planTier":"max_20x"}"#;
        assert_eq!(
            parse_auth_status(out, 5),
            AccountInfo {
                logged_in: true,
                auth_method: Some("claude.ai".into()),
                email: Some("a@b.co".into()),
                org_id: Some("o1".into()),
                org_name: Some("Org".into()),
                subscription_type: Some("max".into()),
                plan_tier: Some("max_20x".into()),
                checked_at: 5,
                error: None,
            }
        );
    }

    #[test]
    fn reads_the_logged_out_shape_and_ignores_noise_around_the_json() {
        let info = parse_auth_status("warning: something\n{\n  \"loggedIn\": false,\n  \"authMethod\": \"none\"\n}\n", 5);
        assert!(!info.logged_in);
        assert_eq!(info.auth_method.as_deref(), Some("none"));
        assert_eq!(info.error, None);
    }

    #[test]
    fn treats_wrong_field_types_and_garbage_as_logged_out() {
        assert_eq!(parse_auth_status(r#"{"loggedIn":"yes","email":42}"#, 5), logged_out(5));
        assert!(parse_auth_status("command not found", 5).error.is_some());
    }

    #[test]
    fn braces_in_the_wrong_order_or_broken_json_are_unreadable() {
        assert_eq!(parse_auth_status("} {", 1).error.as_deref(), Some(UNREADABLE));
        assert_eq!(parse_auth_status("{ not json }", 1).error.as_deref(), Some(UNREADABLE));
        assert_eq!(parse_auth_status("", 1).error.as_deref(), Some(UNREADABLE));
        // an empty text is no email
        assert_eq!(parse_auth_status(r#"{"loggedIn":true,"email":""}"#, 1).email, None);
    }
}
