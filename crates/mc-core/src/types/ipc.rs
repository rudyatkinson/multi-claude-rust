//! `src/shared/ipc.ts`: the channels between the window and the backend and their payloads,
//! plus the `ui:unsaved` payload of `src/shared/unsaved.ts`.
//!
//! Invoke results and most invoke arguments are the domain types of the other modules; the
//! payloads that only exist as inline object types in the TypeScript are named here.

use serde::{Deserialize, Serialize};

use super::serde_util::js_number;
use super::{Layout, ProfileView, SessionInfo, UsageCache, Workflow, WorkflowRun};

/// The channel names, as the renderer uses them.
pub mod channel {
    // renderer -> backend, request/response
    pub const APP_GET_STATE: &str = "app:getState";
    pub const APP_READY: &str = "app:ready";
    pub const PROFILES_CREATE: &str = "profiles:create";
    pub const PROFILES_UPDATE: &str = "profiles:update";
    pub const PROFILES_DELETE: &str = "profiles:delete";
    pub const PROFILES_LOGIN: &str = "profiles:login";
    pub const PROFILES_LOGOUT: &str = "profiles:logout";
    pub const PROFILES_SET_TOKEN: &str = "profiles:setToken";
    pub const PROFILES_REFRESH_STATUS: &str = "profiles:refreshStatus";
    pub const PROFILES_SYNC_FROM_MAIN: &str = "profiles:syncFromMain";
    pub const PROFILES_OPEN_CONFIG_DIR: &str = "profiles:openConfigDir";
    pub const SESSIONS_CREATE: &str = "sessions:create";
    pub const SESSIONS_ATTACH: &str = "sessions:attach";
    pub const SESSIONS_CLOSE: &str = "sessions:close";
    pub const SESSIONS_CLOSE_MANY: &str = "sessions:closeMany";
    pub const SESSIONS_RENAME: &str = "sessions:rename";
    pub const SESSIONS_REVEAL_FOLDER: &str = "sessions:revealFolder";
    pub const SESSIONS_TRANSFER: &str = "sessions:transfer";
    pub const SESSIONS_RESTART: &str = "sessions:restart";
    pub const SESSIONS_OPEN_EXTERNAL: &str = "sessions:openExternal";
    pub const DIALOG_PICK_FOLDER: &str = "dialog:pickFolder";
    pub const FS_FOLDER_FOR: &str = "fs:folderFor";
    pub const CONVERSATIONS_LIST: &str = "conversations:list";
    pub const CONVERSATIONS_OPEN: &str = "conversations:open";
    pub const FOLDERS_SET_ACCOUNT: &str = "folders:setAccount";
    pub const MCP_LIST: &str = "mcp:list";
    pub const MCP_COPY: &str = "mcp:copy";
    pub const WORKFLOWS_SAVE: &str = "workflows:save";
    pub const WORKFLOWS_DELETE: &str = "workflows:delete";
    pub const WORKFLOWS_RUN: &str = "workflows:run";
    pub const WORKFLOWS_APPROVE: &str = "workflows:approve";
    pub const WORKFLOWS_RETRY: &str = "workflows:retry";
    pub const WORKFLOWS_STOP: &str = "workflows:stop";
    pub const WORKFLOWS_DISMISS: &str = "workflows:dismiss";
    pub const WORKFLOWS_TAKE_ANSWER: &str = "workflows:takeAnswer";
    pub const WORKFLOWS_OUTPUT: &str = "workflows:output";
    pub const SKILLS_LIST: &str = "skills:list";
    pub const SKILLS_READ: &str = "skills:read";
    pub const SKILLS_SAVE: &str = "skills:save";
    pub const SKILLS_CREATE: &str = "skills:create";
    pub const SKILLS_DUPLICATE: &str = "skills:duplicate";
    pub const SKILLS_RENAME: &str = "skills:rename";
    pub const SKILLS_DELETE: &str = "skills:delete";
    pub const SKILLS_TRANSFER: &str = "skills:transfer";
    pub const SKILLS_EXPORT: &str = "skills:export";
    pub const SKILLS_PICK_IMPORT: &str = "skills:pickImport";
    pub const SKILLS_IMPORT: &str = "skills:import";
    pub const SKILLS_REVEAL: &str = "skills:reveal";
    pub const SKILLS_OPEN_FILE: &str = "skills:openFile";
    pub const SKILLS_ASSIST: &str = "skills:assist";
    pub const SKILLS_ADD_FOLDER: &str = "skills:addFolder";
    pub const SKILLS_REMOVE_FOLDER: &str = "skills:removeFolder";
    pub const AGENTS_LIST: &str = "agents:list";
    pub const AGENTS_READ: &str = "agents:read";
    pub const AGENTS_SAVE: &str = "agents:save";
    pub const AGENTS_CREATE: &str = "agents:create";
    pub const AGENTS_DUPLICATE: &str = "agents:duplicate";
    pub const AGENTS_RENAME: &str = "agents:rename";
    pub const AGENTS_DELETE: &str = "agents:delete";
    pub const AGENTS_TRANSFER: &str = "agents:transfer";
    pub const AGENTS_REVEAL: &str = "agents:reveal";
    pub const AGENTS_ADD_FOLDER: &str = "agents:addFolder";
    pub const AGENTS_REMOVE_FOLDER: &str = "agents:removeFolder";
    pub const MEMORY_LIST: &str = "memory:list";
    pub const MEMORY_ENTRIES: &str = "memory:entries";
    pub const MEMORY_READ: &str = "memory:read";
    pub const MEMORY_SAVE: &str = "memory:save";
    pub const MEMORY_CREATE: &str = "memory:create";
    pub const MEMORY_DELETE: &str = "memory:delete";
    pub const MEMORY_SEARCH: &str = "memory:search";
    pub const MEMORY_ADOPT: &str = "memory:adopt";
    pub const MEMORY_REVEAL: &str = "memory:reveal";
    pub const INSTRUCTIONS_SCOPES: &str = "instructions:scopes";
    pub const INSTRUCTIONS_FILES: &str = "instructions:files";
    pub const INSTRUCTIONS_READ: &str = "instructions:read";
    pub const INSTRUCTIONS_SAVE: &str = "instructions:save";
    pub const INSTRUCTIONS_CREATE: &str = "instructions:create";
    pub const INSTRUCTIONS_DELETE: &str = "instructions:delete";
    pub const INSTRUCTIONS_VIEW: &str = "instructions:view";
    pub const INSTRUCTIONS_REVEAL: &str = "instructions:reveal";
    pub const INSTRUCTIONS_ADD_FOLDER: &str = "instructions:addFolder";
    pub const SETTINGS_UPDATE: &str = "settings:update";
    pub const SHELL_OPEN_EXTERNAL: &str = "shell:openExternal";
    pub const CLIPBOARD_READ: &str = "clipboard:read";
    pub const CLIPBOARD_WRITE_TEXT: &str = "clipboard:writeText";
    pub const EXPLORER_MENU_STATUS: &str = "explorerMenu:status";
    pub const EXPLORER_MENU_SET: &str = "explorerMenu:set";
    pub const UPDATER_CHECK: &str = "updater:check";
    pub const UPDATER_INSTALL: &str = "updater:install";
    pub const UPDATER_DISMISS: &str = "updater:dismiss";

    // renderer -> backend, fire and forget
    pub const PTY_WRITE: &str = "pty:write";
    pub const PTY_RESIZE: &str = "pty:resize";
    pub const UI_LAYOUT: &str = "ui:layout";
    pub const SESSION_ATTENTION: &str = "session:attention";
    pub const UI_UNSAVED: &str = "ui:unsaved";

    // backend -> renderer
    pub const PTY_DATA: &str = "pty:data";
    pub const SESSIONS_CHANGED: &str = "sessions:changed";
    pub const PROFILES_CHANGED: &str = "profiles:changed";
    pub const USAGE_CHANGED: &str = "usage:changed";
    pub const APP_CHANGED: &str = "app:changed";
    pub const APP_NOTICE: &str = "app:notice";
    pub const APP_OPEN_FOLDER: &str = "app:openFolder";
    pub const SESSIONS_ACTIVATE: &str = "sessions:activate";
    pub const UI_NEW_SESSION: &str = "ui:newSession";
    pub const WORKFLOWS_CHANGED: &str = "workflows:changed";
    pub const RUNS_CHANGED: &str = "runs:changed";
    pub const UPDATER_AVAILABLE: &str = "updater:available";
    pub const UPDATER_PROGRESS: &str = "updater:progress";
}

use channel as c;

/// Request/response calls (renderer -> backend), in the TypeScript order.
pub const INVOKE_CHANNELS: [&str; 90] = [
    c::APP_GET_STATE,
    c::APP_READY,
    c::PROFILES_CREATE,
    c::PROFILES_UPDATE,
    c::PROFILES_DELETE,
    c::PROFILES_LOGIN,
    c::PROFILES_LOGOUT,
    c::PROFILES_SET_TOKEN,
    c::PROFILES_REFRESH_STATUS,
    c::PROFILES_SYNC_FROM_MAIN,
    c::PROFILES_OPEN_CONFIG_DIR,
    c::SESSIONS_CREATE,
    c::SESSIONS_ATTACH,
    c::SESSIONS_CLOSE,
    c::SESSIONS_CLOSE_MANY,
    c::SESSIONS_RENAME,
    c::SESSIONS_REVEAL_FOLDER,
    c::SESSIONS_TRANSFER,
    c::SESSIONS_RESTART,
    c::SESSIONS_OPEN_EXTERNAL,
    c::DIALOG_PICK_FOLDER,
    c::FS_FOLDER_FOR,
    c::CONVERSATIONS_LIST,
    c::CONVERSATIONS_OPEN,
    c::FOLDERS_SET_ACCOUNT,
    c::MCP_LIST,
    c::MCP_COPY,
    c::WORKFLOWS_SAVE,
    c::WORKFLOWS_DELETE,
    c::WORKFLOWS_RUN,
    c::WORKFLOWS_APPROVE,
    c::WORKFLOWS_RETRY,
    c::WORKFLOWS_STOP,
    c::WORKFLOWS_DISMISS,
    c::WORKFLOWS_TAKE_ANSWER,
    c::WORKFLOWS_OUTPUT,
    c::SKILLS_LIST,
    c::SKILLS_READ,
    c::SKILLS_SAVE,
    c::SKILLS_CREATE,
    c::SKILLS_DUPLICATE,
    c::SKILLS_RENAME,
    c::SKILLS_DELETE,
    c::SKILLS_TRANSFER,
    c::SKILLS_EXPORT,
    c::SKILLS_PICK_IMPORT,
    c::SKILLS_IMPORT,
    c::SKILLS_REVEAL,
    c::SKILLS_OPEN_FILE,
    c::SKILLS_ASSIST,
    c::SKILLS_ADD_FOLDER,
    c::SKILLS_REMOVE_FOLDER,
    c::AGENTS_LIST,
    c::AGENTS_READ,
    c::AGENTS_SAVE,
    c::AGENTS_CREATE,
    c::AGENTS_DUPLICATE,
    c::AGENTS_RENAME,
    c::AGENTS_DELETE,
    c::AGENTS_TRANSFER,
    c::AGENTS_REVEAL,
    c::AGENTS_ADD_FOLDER,
    c::AGENTS_REMOVE_FOLDER,
    c::MEMORY_LIST,
    c::MEMORY_ENTRIES,
    c::MEMORY_READ,
    c::MEMORY_SAVE,
    c::MEMORY_CREATE,
    c::MEMORY_DELETE,
    c::MEMORY_SEARCH,
    c::MEMORY_ADOPT,
    c::MEMORY_REVEAL,
    c::INSTRUCTIONS_SCOPES,
    c::INSTRUCTIONS_FILES,
    c::INSTRUCTIONS_READ,
    c::INSTRUCTIONS_SAVE,
    c::INSTRUCTIONS_CREATE,
    c::INSTRUCTIONS_DELETE,
    c::INSTRUCTIONS_VIEW,
    c::INSTRUCTIONS_REVEAL,
    c::INSTRUCTIONS_ADD_FOLDER,
    c::SETTINGS_UPDATE,
    c::SHELL_OPEN_EXTERNAL,
    c::CLIPBOARD_READ,
    c::CLIPBOARD_WRITE_TEXT,
    c::EXPLORER_MENU_STATUS,
    c::EXPLORER_MENU_SET,
    c::UPDATER_CHECK,
    c::UPDATER_INSTALL,
    c::UPDATER_DISMISS,
];

/// Fire-and-forget messages (renderer -> backend).
pub const SEND_CHANNELS: [&str; 5] = [c::PTY_WRITE, c::PTY_RESIZE, c::UI_LAYOUT, c::SESSION_ATTENTION, c::UI_UNSAVED];

/// Pushed events (backend -> renderer).
pub const EVENT_CHANNELS: [&str; 13] = [
    c::PTY_DATA,
    c::SESSIONS_CHANGED,
    c::PROFILES_CHANGED,
    c::USAGE_CHANGED,
    c::APP_CHANGED,
    c::APP_NOTICE,
    c::APP_OPEN_FOLDER,
    c::SESSIONS_ACTIVATE,
    c::UI_NEW_SESSION,
    c::WORKFLOWS_CHANGED,
    c::RUNS_CHANGED,
    c::UPDATER_AVAILABLE,
    c::UPDATER_PROGRESS,
];

// ---------- send payloads ----------

/// `pty:write`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyWrite {
    pub id: String,
    pub data: String,
}

/// `pty:resize`. The size is as the renderer sent it, any number; `validate::pty_size` decides
/// whether it is one a pty takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyResize {
    pub id: String,
    #[serde(serialize_with = "js_number")]
    pub cols: f64,
    #[serde(serialize_with = "js_number")]
    pub rows: f64,
}

/// `ui:layout` carries a [`Layout`], unchecked; `parseLayout` decides what is kept.
pub type UiLayout = Layout;

/// `session:attention`: a session stopped working while the user was not looking at it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAttention {
    pub id: String,
    pub title: String,
}

/// `ui:unsaved`: what has changes that are not saved in the Library, so that quitting can ask.
/// Each item names one draft and starts a sentence: `The agent "reviewer"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsavedPayload {
    #[serde(default)]
    pub items: Vec<String>,
}

/// How many drafts a quit question names, and how much of each (`cleanUnsaved`).
pub const UNSAVED_MAX_ITEMS: usize = 8;
pub const UNSAVED_MAX_CHARS: usize = 120;

// ---------- event payloads ----------

/// `pty:data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyData {
    pub id: String,
    pub data: String,
    /// UTF-16 units of output the session has produced, this chunk included.
    pub seq: u64,
}

/// `sessions:changed`: the sessions, and with restored tabs the arrangement they come back in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsChanged {
    pub sessions: Vec<SessionInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
}

/// `profiles:changed`.
pub type ProfilesChanged = Vec<ProfileView>;
/// `usage:changed`: profile id to usage.
pub type UsageChanged = UsageCache;
/// `workflows:changed`.
pub type WorkflowsChanged = Vec<Workflow>;
/// `runs:changed`.
pub type RunsChanged = Vec<WorkflowRun>;
// `app:changed` carries an `AppChanged`, `app:notice` a `Notice`.

/// `app:openFolder`: a folder was opened from outside the app (Explorer context menu).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenFolder {
    pub cwd: String,
}

/// `sessions:activate`: bring this tab to the front (restored tabs, a clicked notification).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activate {
    pub id: String,
}

/// `ui:newSession`: open the account and folder picker. `Record<string, never>`: always `{}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSession {}

// ---------- compound invoke arguments ----------

/// `'copy' | 'move'` of `skills:transfer` and `agents:transfer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferMode {
    Copy,
    Move,
}
// `Place` and `SessionRef` live in `layout`, `CreateSessionInput` in `app`, `StartRunInput` in
// `workflows`, `MemoryCreateInput` in `memory`, `InstructionTarget` in `instructions`.
