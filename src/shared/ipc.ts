import type { ConversationEntry } from './conversations'
import type { Layout, Place } from './layout'
import type {
  AppChanged,
  AppSettings,
  AppState,
  AttachResult,
  ClipboardContent,
  CreateProfileInput,
  CreateProfileResult,
  CreateSessionInput,
  ExplorerMenuStatus,
  McpCopyResult,
  McpServerRow,
  Notice,
  ProfilePatch,
  ProfileView,
  SessionInfo,
  SyncResult,
  UsageState
} from './types'
import type { AgentDetail, AgentListing, AgentMoved, AgentRef, AgentSaveResult } from './agents'
import type { InstructionDetail, InstructionFile, InstructionRef, InstructionSaveResult, InstructionScope, LoadStep } from './instructions'
import type { AdoptResult, MemoryDetail, MemoryHit, MemoryListing, MemoryRef, MemorySaveResult, MemoryView } from './memory'
import type { ImportPreview, SaveResult, SkillDetail, SkillListing, SkillMoved, SkillRef } from './skills'
import type { UpdateInfo, UpdateProgress } from './updates'
import type { StartRunInput, Workflow, WorkflowRun } from './workflows'

/** Request/response calls (renderer -> main). */
export interface IpcInvoke {
  'app:getState': () => AppState
  /** The renderer has its listeners in place and can take queued "open folder" requests. */
  'app:ready': () => void
  'profiles:create': (input: CreateProfileInput) => CreateProfileResult
  'profiles:update': (id: string, patch: ProfilePatch) => void
  'profiles:delete': (id: string) => void
  'profiles:login': (id: string) => SessionInfo
  'profiles:logout': (id: string) => void
  'profiles:setToken': (id: string, token: string) => void
  'profiles:refreshStatus': (id: string) => void
  'profiles:syncFromMain': (id: string, force: boolean) => SyncResult
  'profiles:openConfigDir': (id: string) => void
  'sessions:create': (input: CreateSessionInput) => SessionInfo
  'sessions:attach': (id: string) => AttachResult | null
  'sessions:close': (id: string) => void
  'sessions:closeMany': (ids: string[]) => void
  /** An empty name gives the tab its automatic title back. */
  'sessions:rename': (id: string, name: string) => void
  'sessions:revealFolder': (id: string) => void
  /** Continues a tab's conversation under another account, in the same tab. */
  'sessions:transfer': (id: string, profileId: string) => SessionInfo
  'sessions:restart': (id: string, continueLast: boolean) => SessionInfo
  'sessions:openExternal': (input: CreateSessionInput) => void
  /** Ends the process of a running Claude tab; the tab stays, asleep, with its output and its conversation. */
  'sessions:sleep': (id: string) => void
  /** Starts a sleeping tab's Claude again under the same session id, continuing its conversation. */
  'sessions:wake': (id: string) => SessionInfo
  /** A tab kept awake is never put to sleep for being idle. */
  'sessions:setKeepAwake': (id: string, on: boolean) => void
  'dialog:pickFolder': (defaultPath?: string) => string | null
  /** The folder a dropped path stands for: itself, or the folder of a file. Null when it is neither. */
  'fs:folderFor': (path: string) => string | null
  /** The past conversations of every account. */
  'conversations:list': () => ConversationEntry[]
  /** Opens a conversation from the last listing in a tab. */
  'conversations:open': (profileId: string, id: string, place?: Place) => SessionInfo
  /** Pins a folder to an account, so that it opens with it without asking; null removes the pin. */
  'folders:setAccount': (cwd: string, profileId: string | null) => void
  /** The main account's user-level MCP servers, compared with those of another account. */
  'mcp:list': (profileId: string) => McpServerRow[]
  'mcp:copy': (profileId: string, names: string[]) => McpCopyResult[]
  /** Adds a workflow, or replaces the one with the same id. */
  'workflows:save': (workflow: Workflow) => void
  'workflows:delete': (id: string) => void
  /** Starts a run; `place` says where the tab of its first step appears. */
  'workflows:run': (input: StartRunInput, place?: Place) => WorkflowRun
  /** Lets a run that waits at a gate go on with its next step. */
  'workflows:approve': (runId: string) => void
  /** Runs the step a run has paused at once more. */
  'workflows:retry': (runId: string) => void
  'workflows:stop': (runId: string) => void
  /** Takes a run that is over off the list. */
  'workflows:dismiss': (runId: string) => void
  /** Takes what the current step's tab last said as its output. False when it has said nothing. */
  'workflows:takeAnswer': (runId: string) => boolean
  /** The output of a step, when it has one. */
  'workflows:output': (runId: string, index: number) => string | null
  /** The folders that hold skills, and their skills. */
  'skills:list': () => SkillListing
  'skills:read': (ref: SkillRef) => SkillDetail
  /** Writes a SKILL.md over the state named by `baseHash`; null overwrites whatever is there. */
  'skills:save': (ref: SkillRef, text: string, baseHash: string | null) => SaveResult
  'skills:create': (rootId: string, name: string, description: string) => SkillRef
  'skills:duplicate': (ref: SkillRef, name: string) => SkillRef
  'skills:rename': (ref: SkillRef, name: string) => SkillMoved
  /** Into the system's bin. */
  'skills:delete': (ref: SkillRef) => void
  'skills:transfer': (ref: SkillRef, toRootId: string, mode: 'copy' | 'move', name: string) => SkillMoved
  /** Asks where to save the archive; the path it was written to, or null when that was cancelled. */
  'skills:export': (ref: SkillRef) => string | null
  /** Asks for an archive and checks it; null when that was cancelled. */
  'skills:pickImport': () => ImportPreview | null
  'skills:import': (token: string, rootId: string, name: string) => SkillRef
  /** Shows a skill's folder in the file manager, or one of its files there. */
  'skills:reveal': (ref: SkillRef, file?: string) => void
  /** Opens a plain text file of a skill with its program; any other file is only shown. */
  'skills:openFile': (ref: SkillRef, file: string) => void
  /** Opens a session tab that works on the skill as `request` says. */
  'skills:assist': (ref: SkillRef, profileId: string, request: string, place?: Place) => SessionInfo
  /** Asks for a project folder and adds it to the list; the id of its root. */
  'skills:addFolder': () => string | null
  'skills:removeFolder': (rootId: string) => void
  /** The folders that hold subagents, and their agents. */
  'agents:list': () => AgentListing
  'agents:read': (ref: AgentRef) => AgentDetail
  /** `baseHash` is the state the text was read in; null overwrites whatever is on disk. */
  'agents:save': (ref: AgentRef, text: string, baseHash: string | null) => AgentSaveResult
  'agents:create': (rootId: string, name: string, description: string) => AgentRef
  'agents:duplicate': (ref: AgentRef, name: string) => AgentRef
  /** Changes the name in the file, and the file's name when it was the agent's. */
  'agents:rename': (ref: AgentRef, name: string) => AgentMoved
  /** Moves the agent's file to the bin. */
  'agents:delete': (ref: AgentRef) => void
  'agents:transfer': (ref: AgentRef, toRootId: string, mode: 'copy' | 'move', name: string) => AgentMoved
  /** Shows an agent's file in the file manager. */
  'agents:reveal': (ref: AgentRef) => void
  /** Asks for a project folder and adds it to the list; the id of its agents root. */
  'agents:addFolder': () => string | null
  'agents:removeFolder': (rootId: string) => void
  /** The projects that have a memory in some account. */
  'memory:list': () => MemoryListing
  /** The memories of one folder, with what is wrong with its index. */
  'memory:entries': (folderId: string) => MemoryView
  'memory:read': (ref: MemoryRef) => MemoryDetail
  /** `baseHash` is the state the text was read in; null overwrites whatever is on disk. */
  'memory:save': (ref: MemoryRef, text: string, baseHash: string | null) => MemorySaveResult
  /** Makes a memory file and its line in MEMORY.md. */
  'memory:create': (folderId: string, input: { name: string; title: string; description: string; type: string }) => MemoryRef
  /** Moves a memory to the bin and takes its line out of MEMORY.md. */
  'memory:delete': (ref: MemoryRef) => void
  'memory:search': (query: string) => MemoryHit[]
  /** Copies an account's own memories of a project into the main account's folder; without `apply`, only says what would be copied. */
  'memory:adopt': (folderId: string, apply: boolean) => AdoptResult
  /** Shows a memory's file in the file manager, or opens its folder when no file is named. */
  'memory:reveal': (folderId: string, file?: string) => void
  /** The accounts and the project folders whose instruction files can be looked after. */
  'instructions:scopes': () => InstructionScope[]
  /** The CLAUDE.md files of a scope, there or not, and its rules. */
  'instructions:files': (scopeId: string) => InstructionFile[]
  'instructions:read': (ref: InstructionRef) => InstructionDetail
  /** `baseHash` is the state the text was read in; null overwrites whatever is on disk. */
  'instructions:save': (ref: InstructionRef, text: string, baseHash: string | null) => InstructionSaveResult
  /** Makes one of a scope's fixed files that is not there yet, or a rule of the given name. */
  'instructions:create': (scopeId: string, target: { file: string } | { rule: string }) => InstructionRef
  /** Moves an instruction file to the bin. */
  'instructions:delete': (ref: InstructionRef) => void
  /** What a session of an account reads when it starts in a project folder, in order. */
  'instructions:view': (profileId: string, scopeId: string) => LoadStep[]
  /** Shows a file in the file manager, or opens the scope's folder when no file is named. */
  'instructions:reveal': (scopeId: string, file?: string) => void
  /** Asks for a project folder and adds it to the list; the id of its scope. */
  'instructions:addFolder': () => string | null
  'settings:update': (patch: Partial<AppSettings>) => void
  'shell:openExternal': (url: string) => void
  'clipboard:read': () => ClipboardContent
  'clipboard:writeText': (text: string) => void
  'explorerMenu:status': () => ExplorerMenuStatus
  'explorerMenu:set': (enabled: boolean) => ExplorerMenuStatus
  /** Looks for a newer version now; null when this one is the newest. */
  'updater:check': () => UpdateInfo | null
  /** Downloads the update the last check found, ends the app as quitting does, installs it and starts the new version. Returns only when that failed. */
  'updater:install': () => void
  /** "Later": the version is not announced again until the app starts anew. */
  'updater:dismiss': (version: string) => void
}

/** Fire-and-forget messages (renderer -> main). */
export interface IpcSend {
  'pty:write': { id: string; data: string }
  'pty:resize': { id: string; cols: number; rows: number }
  /** How the tabs are arranged now, so that it can be saved and restored on the next start. */
  'ui:layout': Layout
  /** A session stopped working while the user was not looking at it. */
  'session:attention': { id: string; title: string }
  /** What has changes that are not saved in the Library, so that quitting can ask. Each item names one draft. */
  'ui:unsaved': { items: string[] }
  /**
   * The session the user is looking at: the tab in front in the area that has the keyboard, while
   * the window has the focus; null for none. It is never put to sleep for being idle.
   */
  'ui:visible': { id: string | null }
}

/** Pushed events (main -> renderer). */
export interface IpcEvents {
  'pty:data': { id: string; data: string; seq: number }
  /** The sessions, and with restored tabs the arrangement they come back in. */
  'sessions:changed': { sessions: SessionInfo[]; layout?: Layout }
  'profiles:changed': ProfileView[]
  'usage:changed': Record<string, UsageState>
  'app:changed': AppChanged
  'app:notice': Notice
  /** A folder was opened from outside the app (Explorer context menu). */
  'app:openFolder': { cwd: string }
  /** Bring this tab to the front (restored tabs, a clicked notification). */
  'sessions:activate': { id: string }
  /** Open the account and folder picker ("New session" in the tray menu). */
  'ui:newSession': Record<string, never>
  'workflows:changed': Workflow[]
  'runs:changed': WorkflowRun[]
  /** A newer version was found by the check at start or the daily one; once per version and run of the app. */
  'updater:available': { info: UpdateInfo }
  /** How far the download of the update is. */
  'updater:progress': UpdateProgress
}

export const INVOKE_CHANNELS = [
  'app:getState',
  'app:ready',
  'profiles:create',
  'profiles:update',
  'profiles:delete',
  'profiles:login',
  'profiles:logout',
  'profiles:setToken',
  'profiles:refreshStatus',
  'profiles:syncFromMain',
  'profiles:openConfigDir',
  'sessions:create',
  'sessions:attach',
  'sessions:close',
  'sessions:closeMany',
  'sessions:rename',
  'sessions:revealFolder',
  'sessions:transfer',
  'sessions:restart',
  'sessions:openExternal',
  'sessions:sleep',
  'sessions:wake',
  'sessions:setKeepAwake',
  'dialog:pickFolder',
  'fs:folderFor',
  'conversations:list',
  'conversations:open',
  'folders:setAccount',
  'mcp:list',
  'mcp:copy',
  'workflows:save',
  'workflows:delete',
  'workflows:run',
  'workflows:approve',
  'workflows:retry',
  'workflows:stop',
  'workflows:dismiss',
  'workflows:takeAnswer',
  'workflows:output',
  'skills:list',
  'skills:read',
  'skills:save',
  'skills:create',
  'skills:duplicate',
  'skills:rename',
  'skills:delete',
  'skills:transfer',
  'skills:export',
  'skills:pickImport',
  'skills:import',
  'skills:reveal',
  'skills:openFile',
  'skills:assist',
  'skills:addFolder',
  'skills:removeFolder',
  'agents:list',
  'agents:read',
  'agents:save',
  'agents:create',
  'agents:duplicate',
  'agents:rename',
  'agents:delete',
  'agents:transfer',
  'agents:reveal',
  'agents:addFolder',
  'agents:removeFolder',
  'memory:list',
  'memory:entries',
  'memory:read',
  'memory:save',
  'memory:create',
  'memory:delete',
  'memory:search',
  'memory:adopt',
  'memory:reveal',
  'instructions:scopes',
  'instructions:files',
  'instructions:read',
  'instructions:save',
  'instructions:create',
  'instructions:delete',
  'instructions:view',
  'instructions:reveal',
  'instructions:addFolder',
  'settings:update',
  'shell:openExternal',
  'clipboard:read',
  'clipboard:writeText',
  'explorerMenu:status',
  'explorerMenu:set',
  'updater:check',
  'updater:install',
  'updater:dismiss'
] as const satisfies readonly (keyof IpcInvoke)[]

export const SEND_CHANNELS = ['pty:write', 'pty:resize', 'ui:layout', 'session:attention', 'ui:unsaved', 'ui:visible'] as const satisfies readonly (keyof IpcSend)[]

export const EVENT_CHANNELS = [
  'pty:data',
  'sessions:changed',
  'profiles:changed',
  'usage:changed',
  'app:changed',
  'app:notice',
  'app:openFolder',
  'sessions:activate',
  'ui:newSession',
  'workflows:changed',
  'runs:changed',
  'updater:available',
  'updater:progress'
] as const satisfies readonly (keyof IpcEvents)[]

export interface McApi {
  invoke<K extends keyof IpcInvoke>(channel: K, ...args: Parameters<IpcInvoke[K]>): Promise<ReturnType<IpcInvoke[K]>>
  send<K extends keyof IpcSend>(channel: K, payload: IpcSend[K]): void
  /** Returns at once; the listener is registered with the backend a moment later (see `listening` in api.ts). */
  on<K extends keyof IpcEvents>(channel: K, listener: (payload: IpcEvents[K]) => void): () => void
}
