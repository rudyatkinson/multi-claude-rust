import type { Layout, Place } from './layout'
import type { Turn } from './turn'
import type { Workflow, WorkflowRun } from './workflows'

export interface AccountInfo {
  loggedIn: boolean
  email?: string
  orgId?: string
  orgName?: string
  subscriptionType?: string
  authMethod?: string
  checkedAt: number
  error?: string
}

export interface ShareConfig {
  /** Directories linked from the Main profile (junction on Windows, symlink elsewhere). */
  dirs: string[]
  /** Files copied from the Main profile on sync. */
  files: string[]
  /** settings.json keys the profile keeps for itself across syncs. */
  localSettingsKeys: string[]
}

export interface SyncSnapshot {
  at: number
  mainHashes: Record<string, string>
  localHashes: Record<string, string>
}

export type AuthKind = 'login' | 'token'

export interface Profile {
  id: string
  name: string
  slug: string
  color: string
  isDefault: boolean
  /** null for Main. Otherwise passed verbatim as CLAUDE_CONFIG_DIR and never rewritten. */
  configDir: string | null
  share: ShareConfig
  emailHint?: string
  /** How the account authenticates; a missing value means a browser login. */
  auth?: AuthKind
  /** The encrypted OAuth token of a token account. It never leaves the main process. */
  tokenEnc?: string
  lastSync?: SyncSnapshot
  account?: AccountInfo
  /** The account's sessions use the main account's project memory in place of their own. Never set on Main. */
  shareMemory?: boolean
}

export interface SyncStatus {
  mainChanged: string[]
  localChanged: string[]
}

/** A profile as the renderer sees it: without the token. */
export type ProfileView = Omit<Profile, 'tokenEnc'> & {
  sync?: SyncStatus
}

export interface UsageWindow {
  /** 0-100 */
  usedPct: number
  /** epoch ms; missing when Claude Code reported no reset time */
  resetsAt?: number
}

/** A weekly limit that applies to one model, named as Claude Code names it (e.g. "Fable"). */
export interface ModelUsage extends UsageWindow {
  name: string
}

export interface UsageState {
  fiveHour?: UsageWindow
  sevenDay?: UsageWindow
  models?: ModelUsage[]
  /** epoch ms of the last sample that confirmed these numbers */
  observedAt?: number
}

export type SessionKind = 'claude' | 'login'

/** How a conversation came to the account of its tab, when a usage limit made it move there. */
export interface Arrival {
  /** The profile it came from. */
  from: string
  at: number
  /** The limit that was reached there, e.g. "5-hour limit"; "usage limit" when the numbers did not say which. */
  limit: string
  /** Claude was asked to pick the task up. */
  pickedUp: boolean
  /** The app moved it by itself. */
  auto?: true
}

export interface SessionInfo {
  id: string
  profileId: string
  kind: SessionKind
  cwd: string
  title: string
  /** `sleeping`: the tab keeps its place, output and conversation but runs no process until it is woken. */
  status: SessionStatus
  exitCode?: number | null
  startedAt: number
  /** The user's own name for the tab. It wins over the conversation's topic. */
  name?: string
  /** Set in the one list that introduces this session: the session whose tab it took over. */
  replaces?: string
  /** Set in the one list that introduces this session: where its tab was asked to appear. */
  place?: Place
  /** Claude Code's own id of the conversation in this tab, once known. */
  claudeSessionId?: string
  /** Display name of the model in use, e.g. "Opus". */
  model?: string
  /** Whole percent of the context window in use. */
  contextPct?: number
  /** What the session's turn has come to, as its hooks report it. Missing for a session without hooks. */
  turn?: Turn
  /** On its way to another account: it looks ended for a moment, and must not be restarted or moved again. */
  moving?: true
  arrived?: Arrival
  /** Never put to sleep for being idle. */
  keepAwake?: true
  /** When the session went to sleep (epoch ms), while it sleeps. */
  sleptAt?: number
}

export type SessionStatus = 'running' | 'exited' | 'sleeping'

/** Limits of the terminal font size, in px. */
export const FONT_SIZE = { min: 8, max: 28, default: 13 } as const

/** Minutes without activity after which a Claude tab sleeps: `sleepIdleMinutes`, 0 for never. */
export const SLEEP_IDLE_MINUTES = { max: 1440, default: 15 } as const

/** `sleepIdleMinutes` as it is used: the default when missing, 0 for never. */
export const sleepIdleMinutesOf = (settings: Pick<AppSettings, 'sleepIdleMinutes'>): number =>
  Math.min(SLEEP_IDLE_MINUTES.max, Math.max(0, Math.round(settings.sleepIdleMinutes ?? SLEEP_IDLE_MINUTES.default)))

export interface AppSettings {
  /** Explicit path to the claude binary; auto-detected when empty. */
  claudePath?: string
  fontSize: number
  /** Keep the File Explorer context menu entry registered (Windows). */
  explorerMenu?: boolean
  /** Reopen the tabs of the last run on startup. On unless set to false. */
  restoreTabs?: boolean
  /** Alert when a session in the background needs input. On unless set to false. */
  notifications?: boolean
  /** Alert when an account nears a usage limit, and when such a limit starts over. On unless set to false. */
  limitAlerts?: boolean
  /** Move a conversation to another account without asking when a usage limit cuts it off. Off unless set to true. */
  autoSwitch?: boolean
  /** Show a tray icon and keep running in it when the window is closed. On unless set to false. */
  tray?: boolean
  /** Start with the operating system (the installed app only). */
  openAtLogin?: boolean
  /** The note about still running in the tray has been shown once. */
  trayHintShown?: boolean
  /** Project folders the user added to the skill manager's list. */
  skillFolders?: string[]
  /** Where each area's tabs are: a row on top (the default) or a column on the left. */
  tabStrip?: TabStrip
  /** Look for a new version at start and once a day. On unless set to false. */
  checkUpdates?: boolean
  /** Minutes without activity after which a Claude tab is put to sleep; 0 never, missing 15. */
  sleepIdleMinutes?: number
}

export type TabStrip = 'top' | 'side'

/** The tab strip setting as it is used: anything but `side` is the row on top. */
export const tabStripOf = (settings: Pick<AppSettings, 'tabStrip'>): TabStrip => (settings.tabStrip === 'side' ? 'side' : 'top')

/** `package` shows in the Windows 11 menu and the classic one; `classic` only under "Show more options". */
export type ExplorerMenuTier = 'package' | 'classic'

export interface ExplorerMenuStatus {
  supported: boolean
  /** The tier that is registered right now, if any. */
  tier: ExplorerMenuTier | null
  /** The tier that enabling would use on this machine. */
  available: ExplorerMenuTier
  /** Why the Windows 11 menu cannot be used here, when `available` is `classic`. */
  reason?: string
  /** Error of the last registration attempt. */
  detail?: string
}

export interface ClaudeBinaryInfo {
  path: string | null
  version?: string
  error?: string
}

export interface AppState {
  platform: string
  /** False for a development run, which cannot register itself to start with the system. */
  packaged: boolean
  /** Windows build number, used by the terminal for ConPTY quirks. */
  osBuild?: number
  homeDir: string
  profiles: ProfileView[]
  sessions: SessionInfo[]
  /** How the tabs are arranged, as the window last reported it or as it was restored. */
  layout?: Layout
  usage: Record<string, UsageState>
  settings: AppSettings
  recentFolders: string[]
  /** Folders pinned to an account: folder key (see `folderKey`) to profile id. */
  folderAccounts: Record<string, string>
  claude: ClaudeBinaryInfo
  shareableDirs: string[]
  shareableFiles: string[]
  workflows: Workflow[]
  /** The workflow runs that are under way, and the last ones that ended. */
  runs: WorkflowRun[]
}

export interface CreateProfileInput {
  name: string
  color: string
  auth?: AuthKind
  /** Token accounts only: the output of `claude setup-token`. */
  token?: string
  email?: string
  shareDirs: string[]
  shareFiles: string[]
  keepModelSeparate: boolean
}

export interface CreateProfileResult {
  profile: ProfileView
  loginSession: SessionInfo | null
  warnings: string[]
}

export interface ProfilePatch {
  name?: string
  color?: string
  shareMemory?: boolean
}

export interface SyncResult {
  /** true when nothing was written because local edits would be overwritten; retry with force. */
  needsConfirm: boolean
  localChanged: string[]
  linked: string[]
  copied: string[]
  backedUp: string[]
  warnings: string[]
}

/** `last` continues the folder's most recent conversation; `pick` opens Claude Code's conversation list. */
export type ResumeMode = 'last' | 'pick'

export interface CreateSessionInput {
  profileId: string
  cwd: string
  /** Missing for a new conversation. */
  resume?: ResumeMode
  /** Where the tab should appear; without it, in the area that has the keyboard. */
  place?: Place
}

export interface AttachResult {
  info: SessionInfo
  replay: string
  seq: number
}

export interface ClipboardContent {
  text: string
  hasImage: boolean
}

export interface Notice {
  level: 'info' | 'warn' | 'error'
  message: string
}

export interface AppChanged {
  settings: AppSettings
  claude: ClaudeBinaryInfo
  recentFolders: string[]
  folderAccounts: Record<string, string>
}

/** A user-level MCP server of the main account, compared with another account. Never its definition. */
export interface McpServerRow {
  name: string
  /** stdio, http, sse, ... */
  transport: string
  /** `missing`: not in the other account; `different`: there under the same name, defined differently. */
  status: 'missing' | 'same' | 'different'
}

export interface McpCopyResult {
  name: string
  ok: boolean
  error?: string
}

export const PROFILE_COLORS = ['#d97757', '#5b9bd5', '#6bbf6b', '#c678dd', '#e5c07b', '#56b6c2', '#e06c75', '#a0a8b8']
