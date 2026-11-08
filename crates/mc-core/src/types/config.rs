//! `src/main/configStore.ts`: the root of `config.json`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::serde_util::{array_or_default, object_or_default, one};
use super::{AppSettings, Extra, Profile};

/// The whole of `config.json`. Each field falls back to its default when the file has something
/// of the wrong kind there, as the config store did (`Array.isArray(raw.profiles) ? ... : []`,
/// settings merged over the defaults). A list or object that has the right kind but malformed
/// items is an error, so that a broken profile never silently drops every account.
///
/// Root keys this version does not know are kept in `extra`. (The Electron store read past them
/// but did not write them back.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// Always 1.
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default, deserialize_with = "array_or_default")]
    pub profiles: Vec<Profile>,
    #[serde(default, deserialize_with = "object_or_default")]
    pub settings: AppSettings,
    #[serde(default, deserialize_with = "array_or_default")]
    pub recent_folders: Vec<String>,
    /// Folders pinned to an account: folder key to profile id.
    #[serde(default, deserialize_with = "object_or_default")]
    pub folder_accounts: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            version: 1,
            profiles: Vec::new(),
            settings: AppSettings::default(),
            recent_folders: Vec::new(),
            folder_accounts: BTreeMap::new(),
            extra: Extra::new(),
        }
    }
}
