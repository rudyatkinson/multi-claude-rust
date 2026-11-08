//! `src/shared/types.ts`: accounts, sessions, settings and the state the window starts from.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::serde_util::{is_false, ms, opt_ms, px};
use super::{Extra, Layout, Place, Turn, Workflow, WorkflowRun};

// ---------- accounts ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub logged_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    /// As `claude auth status` names it, e.g. `oauth_token`. Free text, not a closed set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(deserialize_with = "ms")]
    pub checked_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareConfig {
    /// Directories linked from the Main profile (junction on Windows, symlink elsewhere).
    pub dirs: Vec<String>,
    /// Files copied from the Main profile on sync.
    pub files: Vec<String>,
    /// settings.json keys the profile keeps for itself across syncs.
    pub local_settings_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSnapshot {
    #[serde(deserialize_with = "ms")]
    pub at: i64,
    pub main_hashes: BTreeMap<String, String>,
    pub local_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthKind {
    Login,
    Token,
}

/// An account as `config.json` keeps it. Keys this version does not know stay in `extra`, as the
/// Electron version kept the profile objects of the file verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub color: String,
    pub is_default: bool,
    /// None for Main. Otherwise passed verbatim as CLAUDE_CONFIG_DIR and never rewritten.
    pub config_dir: Option<String>,
    pub share: ShareConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_hint: Option<String>,
    /// How the account authenticates; a missing value means a browser login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthKind>,
    /// The encrypted OAuth token of a token account. It never leaves the backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_enc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<SyncSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountInfo>,
    /// The account's sessions use the main account's project memory in place of their own. Never set on Main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_memory: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub main_changed: Vec<String>,
    pub local_changed: Vec<String>,
}

/// A profile as the renderer sees it: `Omit<Profile, 'tokenEnc'> & { sync?: SyncStatus }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub color: String,
    pub is_default: bool,
    pub config_dir: Option<String>,
    pub share: ShareConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<SyncSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_memory: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncStatus>,
    /// The profile's unknown keys, which the Electron version spread into the view as well.
    #[serde(flatten)]
    pub extra: Extra,
}

impl From<Profile> for ProfileView {
    /// The view without a sync status; the token is dropped.
    fn from(p: Profile) -> Self {
        ProfileView {
            id: p.id,
            name: p.name,
            slug: p.slug,
            color: p.color,
            is_default: p.is_default,
            config_dir: p.config_dir,
            share: p.share,
            email_hint: p.email_hint,
            auth: p.auth,
            last_sync: p.last_sync,
            account: p.account,
            share_memory: p.share_memory,
            sync: None,
            extra: p.extra,
        }
    }
}

pub const PROFILE_COLORS: [&str; 8] = ["#d97757", "#5b9bd5", "#6bbf6b", "#c678dd", "#e5c07b", "#56b6c2", "#e06c75", "#a0a8b8"];

// ---------- usage ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// 0-100, with a fraction as Claude Code reports it.
    pub used_pct: f64,
    /// Epoch ms; missing when Claude Code reported no reset time.
    #[serde(default, deserialize_with = "opt_ms", skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

/// A weekly limit that applies to one model, named as Claude Code names it (e.g. "Fable").
/// `ModelUsage extends UsageWindow` in the TypeScript; the fields are inlined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub name: String,
    pub used_pct: f64,
    #[serde(default, deserialize_with = "opt_ms", skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

impl ModelUsage {
    pub fn window(&self) -> UsageWindow {
        UsageWindow { used_pct: self.used_pct, resets_at: self.resets_at }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelUsage>>,
    /// Epoch ms of the last sample that confirmed these numbers.
    #[serde(default, deserialize_with = "opt_ms", skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<i64>,
}

// ---------- sessions ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionKind {
    Claude,
    Login,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Running,
    Exited,
}

/// How a conversation came to the account of its tab, when a usage limit made it move there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arrival {
    /// The profile it came from.
    pub from: String,
    pub at: i64,
    /// The limit that was reached there, e.g. "5-hour limit"; "usage limit" when the numbers did not say which.
    pub limit: String,
    /// Claude was asked to pick the task up.
    pub picked_up: bool,
    /// The app moved it by itself. (`auto?: true`: written only when true.)
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub profile_id: String,
    pub kind: SessionKind,
    pub cwd: String,
    pub title: String,
    pub status: SessionStatus,
    /// `exitCode?: number | null`: a missing key and `null` both read as None, and None is written
    /// as a missing key (the renderer only tests it for truthiness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub started_at: i64,
    /// The user's own name for the tab. It wins over the conversation's topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Set in the one list that introduces this session: the session whose tab it took over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    /// Set in the one list that introduces this session: where its tab was asked to appear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    /// Claude Code's own id of the conversation in this tab, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    /// Display name of the model in use, e.g. "Opus".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whole percent of the context window in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_pct: Option<u32>,
    /// What the session's turn has come to, as its hooks report it. Missing for a session without hooks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
    /// On its way to another account (`moving?: true`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub moving: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrived: Option<Arrival>,
}

/// `last` continues the folder's most recent conversation; `pick` opens Claude Code's conversation list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResumeMode {
    Last,
    Pick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionInput {
    pub profile_id: String,
    pub cwd: String,
    /// Missing for a new conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeMode>,
    /// Where the tab should appear; without it, in the area that has the keyboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachResult {
    pub info: SessionInfo,
    pub replay: String,
    /// UTF-16 units of output the session has produced so far.
    pub seq: u64,
}

// ---------- settings ----------

/// Limits of the terminal font size, in px.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontSizeLimits {
    pub min: u32,
    pub max: u32,
    pub default: u32,
}

pub const FONT_SIZE: FontSizeLimits = FontSizeLimits { min: 8, max: 28, default: 13 };

fn default_font_size() -> u32 {
    FONT_SIZE.default
}

/// The app's settings. Keys this version does not know stay in `extra`, as the Electron version
/// merged the stored object over its defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Explicit path to the claude binary; auto-detected when empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_path: Option<String>,
    #[serde(default = "default_font_size", deserialize_with = "px")]
    pub font_size: u32,
    /// Keep the File Explorer context menu entry registered (Windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explorer_menu: Option<bool>,
    /// Reopen the tabs of the last run on startup. On unless set to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_tabs: Option<bool>,
    /// Alert when a session in the background needs input. On unless set to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<bool>,
    /// Alert when an account nears a usage limit, and when such a limit starts over. On unless set to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_alerts: Option<bool>,
    /// Move a conversation to another account without asking when a usage limit cuts it off. Off unless set to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_switch: Option<bool>,
    /// Show a tray icon and keep running in it when the window is closed. On unless set to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray: Option<bool>,
    /// Start with the operating system (the installed app only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_at_login: Option<bool>,
    /// The note about still running in the tray has been shown once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_hint_shown: Option<bool>,
    /// Project folders the user added to the skill manager's list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_folders: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for AppSettings {
    /// `DEFAULT_SETTINGS` of the config store: `{ fontSize: 13 }`.
    fn default() -> Self {
        AppSettings {
            claude_path: None,
            font_size: FONT_SIZE.default,
            explorer_menu: None,
            restore_tabs: None,
            notifications: None,
            limit_alerts: None,
            auto_switch: None,
            tray: None,
            open_at_login: None,
            tray_hint_shown: None,
            skill_folders: None,
            extra: Extra::new(),
        }
    }
}

/// What `settings:update` may change, after `validate::settings_patch` has checked the
/// renderer's `Partial<AppSettings>`. `claude_path` has three states: untouched (None), cleared
/// (`Some(None)`) and set. Not sent over the wire, so it has no serde shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsPatch {
    pub claude_path: Option<Option<String>>,
    pub font_size: Option<u32>,
    pub restore_tabs: Option<bool>,
    pub notifications: Option<bool>,
    pub limit_alerts: Option<bool>,
    pub auto_switch: Option<bool>,
    pub tray: Option<bool>,
    pub open_at_login: Option<bool>,
}

impl SettingsPatch {
    /// `{ ...settings, ...patch }`.
    pub fn apply_to(&self, settings: &mut AppSettings) {
        if let Some(path) = &self.claude_path {
            settings.claude_path.clone_from(path);
        }
        if let Some(size) = self.font_size {
            settings.font_size = size;
        }
        let flags = [
            (self.restore_tabs, &mut settings.restore_tabs),
            (self.notifications, &mut settings.notifications),
            (self.limit_alerts, &mut settings.limit_alerts),
            (self.auto_switch, &mut settings.auto_switch),
            (self.tray, &mut settings.tray),
            (self.open_at_login, &mut settings.open_at_login),
        ];
        for (value, slot) in flags {
            if value.is_some() {
                *slot = value;
            }
        }
    }
}

// ---------- the Explorer menu and the claude binary ----------

/// `package` shows in the Windows 11 menu and the classic one; `classic` only under "Show more options".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplorerMenuTier {
    Package,
    Classic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerMenuStatus {
    pub supported: bool,
    /// The tier that is registered right now, if any (`null` on the wire).
    pub tier: Option<ExplorerMenuTier>,
    /// The tier that enabling would use on this machine.
    pub available: ExplorerMenuTier,
    /// Why the Windows 11 menu cannot be used here, when `available` is `classic`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Error of the last registration attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeBinaryInfo {
    /// `null` on the wire when no binary was found.
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------- the window's state ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub platform: String,
    /// False for a development run, which cannot register itself to start with the system.
    pub packaged: bool,
    /// Windows build number, used by the terminal for ConPTY quirks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_build: Option<u32>,
    pub home_dir: String,
    pub profiles: Vec<ProfileView>,
    pub sessions: Vec<SessionInfo>,
    /// How the tabs are arranged, as the window last reported it or as it was restored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// Profile id to its usage.
    pub usage: BTreeMap<String, UsageState>,
    pub settings: AppSettings,
    pub recent_folders: Vec<String>,
    /// Folders pinned to an account: folder key (see `folderKey`) to profile id.
    pub folder_accounts: BTreeMap<String, String>,
    pub claude: ClaudeBinaryInfo,
    pub shareable_dirs: Vec<String>,
    pub shareable_files: Vec<String>,
    pub workflows: Vec<Workflow>,
    /// The workflow runs that are under way, and the last ones that ended.
    pub runs: Vec<WorkflowRun>,
}

/// Payload of `app:changed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppChanged {
    pub settings: AppSettings,
    pub claude: ClaudeBinaryInfo,
    pub recent_folders: Vec<String>,
    pub folder_accounts: BTreeMap<String, String>,
}

// ---------- making and changing accounts ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfileInput {
    pub name: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthKind>,
    /// Token accounts only: the output of `claude setup-token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub share_dirs: Vec<String>,
    pub share_files: Vec<String>,
    pub keep_model_separate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfileResult {
    pub profile: ProfileView,
    /// `null` on the wire for a token account.
    pub login_session: Option<SessionInfo>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_memory: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    /// true when nothing was written because local edits would be overwritten; retry with force.
    pub needs_confirm: bool,
    pub local_changed: Vec<String>,
    pub linked: Vec<String>,
    pub copied: Vec<String>,
    pub backed_up: Vec<String>,
    pub warnings: Vec<String>,
}

// ---------- small results ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardContent {
    pub text: String,
    pub has_image: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

/// Payload of `app:notice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub level: NoticeLevel,
    pub message: String,
}

/// `missing`: not in the other account; `different`: there under the same name, defined differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum McpServerStatus {
    Missing,
    Same,
    Different,
}

/// A user-level MCP server of the main account, compared with another account. Never its definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRow {
    pub name: String,
    /// stdio, http, sse, ...
    pub transport: String,
    pub status: McpServerStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCopyResult {
    pub name: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
