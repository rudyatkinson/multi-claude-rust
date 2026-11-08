//! `default_profile_id`, `choose_open_target` (`src/shared/openTarget.ts`).

use serde::Serialize;

use crate::types::{Profile, ProfileView};

/// What these rules read of an account: `{ id, isDefault?, account?: { loggedIn } }`.
pub trait AccountRef {
    fn id(&self) -> &str;
    fn is_default(&self) -> bool;
    /// None while the account's status is not known yet.
    fn logged_in(&self) -> Option<bool>;
}

impl AccountRef for Profile {
    fn id(&self) -> &str {
        &self.id
    }
    fn is_default(&self) -> bool {
        self.is_default
    }
    fn logged_in(&self) -> Option<bool> {
        self.account.as_ref().map(|a| a.logged_in)
    }
}

impl AccountRef for ProfileView {
    fn id(&self) -> &str {
        &self.id
    }
    fn is_default(&self) -> bool {
        self.is_default
    }
    fn logged_in(&self) -> Option<bool> {
        self.account.as_ref().map(|a| a.logged_in)
    }
}

/// The account a new session starts with unless the user picks another: Main when it is logged
/// in, else the first logged-in one.
pub fn default_profile_id<P: AccountRef>(profiles: &[P]) -> Option<String> {
    let mut logged_in = profiles.iter().filter(|p| p.logged_in() == Some(true));
    let first = logged_in.clone().next();
    logged_in.find(|p| p.is_default()).or(first).map(|p| p.id().to_owned())
}

/// `{ kind: 'direct', profileId } | { kind: 'pick' } | { kind: 'none' }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenTarget {
    Direct {
        #[serde(rename = "profileId")]
        profile_id: String,
    },
    Pick,
    /// No account is logged in.
    #[serde(rename = "none")]
    NoAccount,
}

/// Which account opens a folder that arrives from outside the app: the one the folder is pinned
/// to, or the only logged-in one, directly; otherwise the user picks. Accounts whose status is not
/// known yet may still turn out to be logged in, so they also lead to the picker.
pub fn choose_open_target<P: AccountRef>(profiles: &[P], pinned_id: Option<&str>) -> OpenTarget {
    let logged_in: Vec<&P> = profiles.iter().filter(|p| p.logged_in() == Some(true)).collect();
    if let Some(pinned) = pinned_id.filter(|id| !id.is_empty()) {
        if logged_in.iter().any(|p| p.id() == pinned) {
            return OpenTarget::Direct { profile_id: pinned.to_owned() };
        }
    }
    let unknown = profiles.iter().filter(|p| p.logged_in().is_none()).count();
    match (logged_in.as_slice(), unknown) {
        ([only], 0) => OpenTarget::Direct { profile_id: only.id().to_owned() },
        ([], 0) => OpenTarget::NoAccount,
        _ => OpenTarget::Pick,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An account as the TypeScript tests spell it.
    pub(crate) struct Acc {
        pub id: &'static str,
        pub is_default: bool,
        pub logged_in: Option<bool>,
    }

    impl AccountRef for Acc {
        fn id(&self) -> &str {
            self.id
        }
        fn is_default(&self) -> bool {
            self.is_default
        }
        fn logged_in(&self) -> Option<bool> {
            self.logged_in
        }
    }

    pub(crate) fn acc(id: &'static str, logged_in: bool) -> Acc {
        Acc { id, is_default: false, logged_in: Some(logged_in) }
    }

    fn unknown(id: &'static str) -> Acc {
        Acc { id, is_default: false, logged_in: None }
    }

    fn main(logged_in: bool) -> Acc {
        Acc { id: "main", is_default: true, logged_in: Some(logged_in) }
    }

    fn direct(id: &str) -> OpenTarget {
        OpenTarget::Direct { profile_id: id.to_owned() }
    }

    // ---- probe.test.ts: defaultProfileId ----

    #[test]
    fn default_prefers_the_main_account_when_it_is_logged_in() {
        assert_eq!(default_profile_id(&[acc("work", true), main(true)]).as_deref(), Some("main"));
    }

    #[test]
    fn default_falls_back_to_the_first_logged_in_account() {
        assert_eq!(default_profile_id(&[main(false), acc("work", true), acc("home", true)]).as_deref(), Some("work"));
    }

    #[test]
    fn default_is_none_when_nobody_is_logged_in() {
        assert_eq!(default_profile_id(&[main(false), unknown("new")]), None);
    }

    // ---- openFolder.test.ts: chooseOpenTarget ----

    #[test]
    fn opens_directly_with_the_only_logged_in_account() {
        assert_eq!(choose_open_target(&[acc("main", true)], None), direct("main"));
        assert_eq!(choose_open_target(&[acc("main", false), acc("work", true)], None), direct("work"));
    }

    #[test]
    fn asks_when_several_accounts_are_logged_in() {
        assert_eq!(choose_open_target(&[acc("main", true), acc("work", true)], None), OpenTarget::Pick);
    }

    #[test]
    fn asks_while_an_account_has_not_been_checked_yet() {
        assert_eq!(choose_open_target(&[acc("main", true), unknown("work")], None), OpenTarget::Pick);
        assert_eq!(choose_open_target(&[unknown("main")], None), OpenTarget::Pick);
    }

    #[test]
    fn reports_when_nobody_is_logged_in() {
        assert_eq!(choose_open_target(&[acc("main", false)], None), OpenTarget::NoAccount);
        assert_eq!(choose_open_target::<Acc>(&[], None), OpenTarget::NoAccount);
    }

    // ---- folders.test.ts: chooseOpenTarget with a pinned folder ----

    #[test]
    fn opens_with_the_pinned_account_without_asking() {
        let both = [acc("main", true), acc("work", true)];
        assert_eq!(choose_open_target(&both, Some("work")), direct("work"));
        assert_eq!(choose_open_target(&both, None), OpenTarget::Pick);
    }

    #[test]
    fn asks_when_the_pinned_account_is_logged_out_or_gone() {
        let logged_out = [acc("main", true), acc("work", false), acc("third", true)];
        assert_eq!(choose_open_target(&logged_out, Some("work")), OpenTarget::Pick);
        let both = [acc("main", true), acc("work", true)];
        assert_eq!(choose_open_target(&both, Some("deleted")), OpenTarget::Pick);
    }

    #[test]
    fn still_opens_directly_when_only_one_account_is_logged_in() {
        let one = [acc("main", true), acc("work", false)];
        assert_eq!(choose_open_target(&one, Some("work")), direct("main"));
    }

    #[test]
    fn serializes_as_the_typescript_union() {
        assert_eq!(serde_json::to_value(direct("w")).unwrap(), serde_json::json!({ "kind": "direct", "profileId": "w" }));
        assert_eq!(serde_json::to_value(OpenTarget::Pick).unwrap(), serde_json::json!({ "kind": "pick" }));
        assert_eq!(serde_json::to_value(OpenTarget::NoAccount).unwrap(), serde_json::json!({ "kind": "none" }));
    }
}
