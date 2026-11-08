import { create } from 'zustand'
import { shouldAlert } from '@shared/attention'
import type { ConversationEntry } from '@shared/conversations'
import { folderKey, pinnedAccount } from '@shared/folders'
import {
  activate,
  activeIdOf,
  canSplit,
  cycleTab,
  emptyLayout,
  focusStep,
  groupOf,
  isVisible,
  mergeAll,
  moveTab,
  parseLayout,
  reconcile,
  setRatio,
  splitRoot,
  splitWith,
  tabAt,
  visibleIds,
  type Edge,
  type Layout,
  type Place
} from '@shared/layout'
import { chooseOpenTarget } from '@shared/openTarget'
import { matchShortcut, type ShortcutAction } from '@shared/shortcuts'
import { parseTerminalTitle, type TabStatus } from '@shared/terminalTitle'
import { callsForYou, turnKey } from '@shared/turn'
import type { AgentRef } from '@shared/agents'
import type { AdoptResult } from '@shared/memory'
import type { ImportPreview, SkillRef } from '@shared/skills'
import { FONT_SIZE, type AppState, type Notice, type ResumeMode, type SessionInfo } from '@shared/types'
import { freeStepName, type Workflow, type WorkflowRun, type WorkflowStep } from '@shared/workflows'
import { errorMessage, getAppVersion, listening, mc } from './api'
import type { LibrarySection } from './library'
import { tabLabel } from './format'
import { clearSearch, disposeMissing, focusTerminal, handlePtyData, setFontSize } from './terminal/terminalRegistry'

export type Modal =
  | { type: 'addAccount' }
  | { type: 'settings' }
  | { type: 'editProfile'; profileId: string }
  | { type: 'openFolder'; cwd: string; place?: Place }
  | { type: 'replaceToken'; profileId: string }
  | { type: 'mcp'; profileId: string }
  | { type: 'transfer'; sessionId: string }
  | { type: 'history' }
  | { type: 'runWorkflow'; workflowId: string }
  | { type: 'stepOutput'; runId: string; index: number }
  | { type: 'skillNew'; rootId?: string }
  | { type: 'skillName'; mode: 'rename' | 'duplicate'; ref: SkillRef }
  | { type: 'skillTransfer'; ref: SkillRef }
  | { type: 'skillImport'; preview: ImportPreview }
  | { type: 'skillAssist'; ref: SkillRef }
  | { type: 'agentNew'; rootId?: string }
  | { type: 'agentName'; mode: 'rename' | 'duplicate'; ref: AgentRef }
  | { type: 'agentTransfer'; ref: AgentRef }
  | { type: 'memoryNew'; folderId: string }
  | { type: 'memoryAdopt'; folderId: string; plan: AdoptResult }
  | { type: 'instructionRule'; scopeId: string }
  | null

/** What lies over the terminals: the workflow designer, the Library, or nothing. */
export type Panel = 'workflows' | 'library' | null

/** What a session's terminal title says, and since when it has been saying that Claude works. */
export interface TabState extends TabStatus {
  busySince?: number
}

export interface Toast extends Notice {
  id: number
}

export interface ConfirmRequest {
  title: string
  message: string
  confirmLabel: string
  danger?: boolean
}

interface UiState extends Omit<AppState, 'layout'> {
  ready: boolean
  /** How the tabs are arranged: a tree of areas, each with its own tabs. This window decides it. */
  layout: Layout
  /** The session that has the keyboard: the tab shown in the focused area. Follows `layout`. */
  activeId: string | null
  /** The area whose "+" opened the account and folder picker. */
  newSession: { groupId: string } | null
  /** The tab that is being dragged. */
  tabDrag: string | null
  /** The version of this app; null until the backend said it. */
  appVersion: string | null
  /** Topic and activity of each session, read from its terminal title. */
  tabStatus: Record<string, TabState>
  /** Sessions that stopped working while the user was not looking at them. */
  attention: Record<string, true>
  /** The session whose terminal shows the search bar. */
  searchId: string | null
  /** Counts the requests for the search bar, so that asking again puts the keyboard back in it. */
  searchRequests: number
  /** The tab whose context menu is open, and where on screen. */
  tabMenu: { id: string; x: number; y: number } | null
  /** The tab whose name is being typed. */
  renamingId: string | null
  /** "Limit used up" bars the user closed, by session and limit window. */
  dismissedLimits: Record<string, true>
  /** The panel that is open; it covers the terminals. */
  panel: Panel
  /** The part of the Library that shows when it is open. */
  librarySection: LibrarySection
  /** The workflow shown in the designer. */
  workflowId: string | null
  sidebarCollapsed: boolean
  modal: Modal
  toasts: Toast[]
  confirm: (ConfirmRequest & { resolve(ok: boolean): void }) | null
  /** Ticks so that countdowns and "reset" states update without new data. */
  now: number
}

const EMPTY: Omit<AppState, 'layout'> = {
  platform: '',
  packaged: false,
  homeDir: '',
  profiles: [],
  sessions: [],
  usage: {},
  settings: { fontSize: FONT_SIZE.default },
  recentFolders: [],
  folderAccounts: {},
  claude: { path: null },
  shareableDirs: [],
  shareableFiles: [],
  workflows: [],
  runs: []
}

const SIDEBAR_KEY = 'mc.sidebarCollapsed'

// A layout preference of this window only. Storage can be unavailable; the sidebar then starts open.
function readSidebarCollapsed(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_KEY) === '1'
  } catch {
    return false
  }
}

export const useStore = create<UiState>(() => ({
  ...EMPTY,
  ready: false,
  layout: emptyLayout(),
  activeId: null,
  newSession: null,
  tabDrag: null,
  appVersion: null,
  tabStatus: {},
  attention: {},
  searchId: null,
  searchRequests: 0,
  tabMenu: null,
  renamingId: null,
  dismissedLimits: {},
  panel: null,
  librarySection: 'skills',
  workflowId: null,
  sidebarCollapsed: readSidebarCollapsed(),
  modal: null,
  toasts: [],
  confirm: null,
  now: Date.now()
}))

const set = useStore.setState
const get = useStore.getState
let toastId = 0

export function toast(level: Notice['level'], message: string): void {
  const id = ++toastId
  set((s) => ({ toasts: [...s.toasts, { id, level, message }] }))
  setTimeout(() => dismissToast(id), level === 'info' ? 6000 : 12000)
}

export function dismissToast(id: number): void {
  set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }))
}

/** Runs an IPC call and reports a failure as a toast instead of an unhandled rejection. */
export async function attempt<T>(work: Promise<T>): Promise<T | undefined> {
  try {
    return await work
  } catch (err) {
    toast('error', errorMessage(err))
    return undefined
  }
}

export function ask(request: ConfirmRequest): Promise<boolean> {
  return new Promise((resolve) => {
    set({
      confirm: {
        ...request,
        resolve: (ok) => {
          set({ confirm: null })
          resolve(ok)
        }
      }
    })
  })
}

/** The main profile can be renamed, so texts that refer to it use its current name. */
export function mainProfileName(): string {
  return get().profiles.find((p) => p.isDefault)?.name ?? 'Main'
}

export function setModal(modal: Modal): void {
  set({ modal })
}

/** Gives the keyboard back to the terminal after a menu, a popover, an input or a drag had it. */
export function refocus(): void {
  const { activeId, panel } = get()
  // a panel lies over the terminals: what is typed there must not reach a session
  if (activeId && !panel) focusTerminal(activeId)
}

/** What the tab of a session is called right now. */
function labelOf(session: SessionInfo): string {
  return tabLabel(session, get().tabStatus[session.id]?.topic)
}

// ---------- layout: the areas and their tabs ----------

/** With the window in front, the sessions on screen have been seen. */
function markSeen(): void {
  if (!document.hasFocus()) return
  const s = get()
  const seen = visibleIds(s.layout).filter((id) => s.attention[id])
  if (seen.length === 0) return
  set({ attention: Object.fromEntries(Object.entries(s.attention).filter(([id]) => !seen.includes(id))) })
}

/** Makes a new arrangement the current one and tells the main process, which saves it. */
function setLayout(next: Layout): void {
  if (next === get().layout) return
  set({ layout: next, activeId: activeIdOf(next) })
  mc.send('ui:layout', next)
  markSeen()
  // a search bar goes with its terminal
  const { searchId } = get()
  if (searchId && !isVisible(next, searchId)) closeSearch(false)
}

// A session to bring to the front that has no tab yet: its list is still on the way.
let pendingActivate: string | null = null

export function setActive(id: string): void {
  const { layout } = get()
  if (groupOf(layout, id)) setLayout(activate(layout, id))
  else pendingActivate = id
}

/** Moves a tab in front of another one in an area (to its end with null), from wherever it is. */
export function moveTabTo(id: string, groupId: string, beforeId: string | null): void {
  setLayout(moveTab(get().layout, id, groupId, beforeId))
  refocus()
}

/** Gives a tab an area of its own on one side of an area. */
export function splitTab(id: string, groupId: string, edge: Edge): void {
  const { layout } = get()
  if (!canSplit(layout, id, groupId)) {
    toast('info', 'An area needs a second tab before one of them can get an area of its own.')
    return
  }
  setLayout(splitWith(layout, id, groupId, edge))
  refocus()
}

/** Gives a tab an area along one side of the whole window. */
export function splitTabRoot(id: string, edge: Edge): void {
  setLayout(splitRoot(get().layout, id, edge))
  refocus()
}

export function mergeAreas(): void {
  setLayout(mergeAll(get().layout))
  refocus()
}

export function setSplitRatio(splitId: string, ratio: number): void {
  setLayout(setRatio(get().layout, splitId, ratio))
}

export function toggleSidebar(): void {
  const collapsed = !get().sidebarCollapsed
  set({ sidebarCollapsed: collapsed })
  try {
    localStorage.setItem(SIDEBAR_KEY, collapsed ? '1' : '0')
  } catch {
    // not remembered across restarts then
  }
}

// ---------- dragging a tab ----------

/** A tab is being dragged with the pointer (see `Workspace`). A tab that closes meanwhile ends it. */
export function startTabDrag(id: string): void {
  set({ tabDrag: id })
}

export function endTabDrag(): void {
  if (get().tabDrag) set({ tabDrag: null })
}

// ---------- tab status ----------

/** Called for every terminal title a session writes; Claude Code rewrites it about once a second while it works. */
export function setTabTitle(id: string, title: string): void {
  const next = parseTerminalTitle(title)
  const prev = get().tabStatus[id]
  if (!next || (prev && prev.topic === next.topic && prev.busy === next.busy)) return
  const now = Date.now()
  const busySince = next.busy ? (prev?.busy ? prev.busySince : now) : undefined
  set((s) => ({ tabStatus: { ...s.tabStatus, [id]: { ...next, busySince } } }))

  if (!prev?.busy || next.busy) return
  // the session went from working to waiting for input
  const busyMs = now - (prev.busySince ?? now)
  const state = get()
  // the tab shown in each area is being looked at
  if (!shouldAlert({ busyMs, tabActive: isVisible(state.layout, id), windowFocused: document.hasFocus() })) return
  callAttention([id])
}

/** Marks tabs as calling for attention and has the main process tell the user about them. */
function callAttention(ids: readonly string[]): void {
  if (ids.length === 0) return
  set((s) => ({ attention: { ...s.attention, ...Object.fromEntries(ids.map((id) => [id, true as const])) } }))
  const { sessions } = get()
  for (const id of ids) {
    const session = sessions.find((s) => s.id === id)
    mc.send('session:attention', { id, title: session ? labelOf(session) : '' })
  }
}

/** Opens the account and folder picker for an area; the new session's tab appears there. */
export function openNewSession(groupId?: string): void {
  set({ newSession: { groupId: groupId ?? get().layout.focused } })
}

export function closeNewSession(): void {
  if (!get().newSession) return
  set({ newSession: null })
  // the picker had the keyboard; hand it back to the terminal
  refocus()
}

function applySessions({ sessions, layout: restored }: { sessions: SessionInfo[]; layout?: Layout }): void {
  const live = new Set(sessions.map((s) => s.id))
  disposeMissing(live)

  const s = get()
  // restored tabs arrive together with the areas they were in
  const base = (restored && parseLayout(restored)) || s.layout
  // a closed tab hands its place to its neighbour; a restarted one keeps it under its new id
  let layout = reconcile(base, sessions)
  if (pendingActivate && groupOf(layout, pendingActivate)) {
    layout = activate(layout, pendingActivate)
    pendingActivate = null
  } else if (pendingActivate && !live.has(pendingActivate) && s.sessions.some((x) => x.id === pendingActivate)) {
    pendingActivate = null
  }

  const keep = <T>(record: Record<string, T>): Record<string, T> => Object.fromEntries(Object.entries(record).filter(([id]) => live.has(id)))
  set({
    sessions,
    tabStatus: keep(s.tabStatus),
    attention: keep(s.attention),
    tabMenu: s.tabMenu && live.has(s.tabMenu.id) ? s.tabMenu : null,
    renamingId: s.renamingId && live.has(s.renamingId) ? s.renamingId : null,
    tabDrag: s.tabDrag && live.has(s.tabDrag) ? s.tabDrag : null
  })
  setLayout(layout)

  // what the hooks say Claude waits for, or failed with, is worth knowing however short the turn was
  const before = new Map(s.sessions.map((x) => [x.id, x.turn]))
  const called = sessions.filter(({ id, turn }) => {
    if (!callsForYou(turn)) return false
    const old = before.get(id)
    return (!old || turnKey(old) !== turnKey(turn)) && !(isVisible(layout, id) && document.hasFocus())
  })
  callAttention(called.map((x) => x.id))
}

// ---------- sessions ----------

/** Starts a session and brings its tab to the front. Without `place` the tab opens in the area that has the keyboard. */
export async function startSession(profileId: string, cwd: string, resume?: ResumeMode, place?: Place): Promise<void> {
  const info = await attempt(mc.invoke('sessions:create', { profileId, cwd, resume, place: place ?? { groupId: get().layout.focused } }))
  if (info) setActive(info.id)
}

export async function closeSession(id: string): Promise<void> {
  const session = get().sessions.find((s) => s.id === id)
  if (!session) return
  if (session.status === 'running' && session.kind === 'claude') {
    const ok = await ask({
      title: 'Close session',
      message: `"${labelOf(session)}" is still running. Closing the tab ends its Claude process.`,
      confirmLabel: 'Close',
      danger: true
    })
    if (!ok) return
  }
  await attempt(mc.invoke('sessions:close', id))
}

/** Closes several tabs after asking once, however many of them are still running. */
export async function closeSessions(ids: readonly string[]): Promise<void> {
  const targets = get().sessions.filter((s) => ids.includes(s.id))
  if (targets.length === 0) return
  if (targets.length === 1) return closeSession(targets[0].id)
  const running = targets.filter((s) => s.status === 'running' && s.kind === 'claude').length
  if (running > 0) {
    const ok = await ask({
      title: `Close ${targets.length} tabs`,
      message: `${running === 1 ? 'One of them is' : `${running} of them are`} still running. Closing the tabs ends their Claude processes.`,
      confirmLabel: 'Close',
      danger: true
    })
    if (!ok) return
  }
  const closing = targets.map((s) => s.id)
  await attempt(mc.invoke('sessions:closeMany', closing))
}

/** The new session takes the tab of the old one; the session list that announces it moves the view along. */
export async function restartSession(id: string, continueLast: boolean): Promise<void> {
  await attempt(mc.invoke('sessions:restart', id, continueLast))
}

/** Continues the tab's conversation under another account; the tab stays where it is. */
export async function transferSession(id: string, profileId: string): Promise<void> {
  if (get().sessions.find((s) => s.id === id)?.moving) return
  const info = await attempt(mc.invoke('sessions:transfer', id, profileId))
  const account = get().profiles.find((p) => p.id === profileId)?.name
  if (info && account) toast('info', `The conversation continues on ${account}.`)
}

export function dismissLimit(key: string): void {
  set((s) => ({ dismissedLimits: { ...s.dismissedLimits, [key]: true } }))
}

/** Opens a past conversation in a tab, or shows the tab it is already open in. */
export async function openConversation(entry: ConversationEntry): Promise<void> {
  const open = get().sessions.find((s) => s.profileId === entry.profileId && s.claudeSessionId === entry.id && s.status === 'running')
  if (open) {
    setActive(open.id)
    refocus()
    return
  }
  const info = await attempt(mc.invoke('conversations:open', entry.profileId, entry.id, { groupId: get().layout.focused }))
  if (info) setActive(info.id)
}

/** A new conversation with the same account and folder, in a tab right after this one. */
export function duplicateSession(id: string): void {
  const session = get().sessions.find((s) => s.id === id)
  if (session) void startSession(session.profileId, session.cwd, undefined, { afterId: id })
}

export function revealFolder(id: string): void {
  void attempt(mc.invoke('sessions:revealFolder', id))
}

// ---------- tab menu and renaming ----------

export function openTabMenu(id: string, x: number, y: number): void {
  set({ tabMenu: { id, x, y } })
}

export function closeTabMenu(): void {
  if (!get().tabMenu) return
  set({ tabMenu: null })
  refocus()
}

export function startRename(id: string): void {
  set({ renamingId: id })
}

/** Ends the editing of a tab name. `name` is null when it was cancelled; an empty name removes it. */
export function finishRename(id: string, name: string | null): void {
  if (get().renamingId !== id) return
  set({ renamingId: null })
  if (name !== null) void attempt(mc.invoke('sessions:rename', id, name))
  refocus()
}

// ---------- search and zoom ----------

/** Opens the search bar of the terminal that has the keyboard, or puts the keyboard back into it. */
export function openSearch(): void {
  const { activeId, searchId } = get()
  if (!activeId) return
  if (searchId && searchId !== activeId) clearSearch(searchId)
  set((s) => ({ searchId: activeId, searchRequests: s.searchRequests + 1 }))
}

export function closeSearch(giveBackKeyboard = true): void {
  const { searchId } = get()
  if (!searchId) return
  clearSearch(searchId)
  set({ searchId: null })
  if (giveBackKeyboard) focusTerminal(searchId)
}

let saveFontSize: ReturnType<typeof setTimeout> | null = null

/** Changes the font size of all terminals by one step, or back to the default with step 0. */
export function zoom(step: 1 | -1 | 0): void {
  const { settings } = get()
  const fontSize = step === 0 ? FONT_SIZE.default : Math.min(FONT_SIZE.max, Math.max(FONT_SIZE.min, settings.fontSize + step))
  if (fontSize === settings.fontSize) return
  set({ settings: { ...settings, fontSize } })
  setFontSize(fontSize)
  // saved once the keys rest; until then the main process still reports the old size
  if (saveFontSize) clearTimeout(saveFontSize)
  saveFontSize = setTimeout(() => {
    saveFontSize = null
    void attempt(mc.invoke('settings:update', { fontSize: get().settings.fontSize }))
  }, 300)
}

// ---------- shortcuts ----------

export function runShortcut(action: ShortcutAction): void {
  const { layout, activeId } = get()
  switch (action.type) {
    case 'new':
      openNewSession()
      return
    case 'close':
      if (activeId) void closeSession(activeId)
      return
    case 'sidebar':
      toggleSidebar()
      return
    case 'search':
      openSearch()
      return
    case 'history':
      setModal({ type: 'history' })
      return
    case 'zoom':
      zoom(action.step)
      return
    case 'next':
    case 'prev':
      // through every area, so that the key also works with one tab per area
      setLayout(cycleTab(layout, action.type === 'next' ? 1 : -1))
      break
    case 'tab':
      setLayout(tabAt(layout, action.index))
      break
    case 'area':
      setLayout(focusStep(layout, action.step))
      break
    case 'split':
      if (activeId) splitTab(activeId, layout.focused, 'right')
      return
  }
  refocus()
}

/** The account a folder always opens with, if it was pinned to one that still exists. */
export function pinOf(cwd: string): string | null {
  const { folderAccounts, platform, profiles } = get()
  return pinnedAccount(folderAccounts, cwd, platform, profiles)
}

/** Pins a folder to an account, or removes its pin with null. */
export async function setFolderAccount(cwd: string, profileId: string | null): Promise<void> {
  const done = await attempt(mc.invoke('folders:setAccount', cwd, profileId).then(() => true))
  if (!done) return
  const account = get().profiles.find((p) => p.id === profileId)?.name
  toast('info', account ? `This folder now always opens with ${account}.` : 'This folder asks for the account again.')
}

/** A folder to open that did not come with an account: from the Explorer menu, or dropped on a tab strip. */
export function openFolderFromOutside(cwd: string, place?: Place): void {
  const target = chooseOpenTarget(get().profiles, pinOf(cwd))
  if (target.kind === 'direct') void startSession(target.profileId, cwd, undefined, place)
  else if (target.kind === 'pick') setModal({ type: 'openFolder', cwd, place })
  else toast('warn', 'Log in to an account before opening a folder.')
}

// more would be an accident rather than an intention
const MAX_DROPPED_FOLDERS = 4

/** Files or folders were dropped on an area's tab strip: each folder (a file stands for its own) opens there. */
export async function openDropped(paths: readonly string[], groupId: string): Promise<void> {
  const { platform, profiles } = get()
  const folders = new Map<string, string>()
  for (const dropped of paths.slice(0, 16)) {
    const folder = await attempt(mc.invoke('fs:folderFor', dropped))
    if (folder) folders.set(folderKey(folder, platform), folder)
  }
  let asked = false
  for (const folder of [...folders.values()].slice(0, MAX_DROPPED_FOLDERS)) {
    const direct = chooseOpenTarget(profiles, pinOf(folder)).kind === 'direct'
    // the account question can be asked for one folder at a time
    if (!direct && asked) continue
    asked ||= !direct
    openFolderFromOutside(folder, { groupId })
  }
  if (folders.size === 0) toast('warn', 'Drop a folder, or a file from the folder to open.')
}

// ---------- workflows ----------

const SAVE_WORKFLOW_MS = 400
// workflows that were changed here and are not saved yet: what the main process says about them is older
const unsaved = new Map<string, ReturnType<typeof setTimeout>>()

function sendWorkflow(id: string): Promise<void> {
  const timer = unsaved.get(id)
  if (timer === undefined) return Promise.resolve()
  clearTimeout(timer)
  unsaved.delete(id)
  const workflow = get().workflows.find((w) => w.id === id)
  return workflow ? attempt(mc.invoke('workflows:save', workflow)).then(() => undefined) : Promise.resolve()
}

/** Changes a workflow, or adds it. It is saved once the typing rests. */
export function updateWorkflow(workflow: Workflow): void {
  set((s) => ({ workflows: s.workflows.some((w) => w.id === workflow.id) ? s.workflows.map((w) => (w.id === workflow.id ? workflow : w)) : [...s.workflows, workflow] }))
  clearTimeout(unsaved.get(workflow.id))
  unsaved.set(
    workflow.id,
    setTimeout(() => void sendWorkflow(workflow.id), SAVE_WORKFLOW_MS)
  )
}

function applyWorkflows(saved: Workflow[]): void {
  const local = get().workflows
  const mine = (id: string): Workflow | undefined => (unsaved.has(id) ? local.find((w) => w.id === id) : undefined)
  set({ workflows: [...saved.map((w) => mine(w.id) ?? w), ...local.filter((w) => unsaved.has(w.id) && !saved.some((x) => x.id === w.id))] })
}

/** Opens a panel over the terminals, in place of the one that may be open. */
export function openPanel(panel: Exclude<Panel, null>): void {
  // what was typed in the designer is saved when it goes out of sight, whatever replaces it
  for (const id of [...unsaved.keys()]) void sendWorkflow(id)
  const { workflows, workflowId } = get()
  set({ panel, newSession: null, tabMenu: null, workflowId: workflows.some((w) => w.id === workflowId) ? workflowId : (workflows[0]?.id ?? null) })
}

/** Shows another part of the Library. What is typed in the part that goes out of sight stays in its own store. */
export function setLibrarySection(librarySection: LibrarySection): void {
  set({ librarySection })
}

/** Closes the open panel. It never asks: it is also called to show a tab that lies underneath. */
export function closePanel(): void {
  if (!get().panel) return
  for (const id of [...unsaved.keys()]) void sendWorkflow(id)
  set({ panel: null })
  refocus()
}

export function selectWorkflow(id: string): void {
  set({ workflowId: id })
}

/** The account new steps start with: the main one when it is signed in, else the first that is. */
function defaultAccount(): string {
  const { profiles } = get()
  const signedIn = profiles.filter((p) => p.account?.loggedIn)
  return (signedIn.find((p) => p.isDefault) ?? signedIn[0] ?? profiles[0])?.id ?? ''
}

const newId = (): string => crypto.randomUUID()

export function newStep(steps: readonly WorkflowStep[]): WorkflowStep {
  const before = steps[steps.length - 1]
  return {
    id: newId(),
    name: freeStepName(steps),
    profileId: before?.profileId ?? defaultAccount(),
    prompt: steps.length === 0 ? '{{task}}' : '{{previous.output}}',
    gate: true
  }
}

/** Adds a workflow with one step and shows it. */
export function createWorkflow(): void {
  const { workflows } = get()
  const taken = (name: string): boolean => workflows.some((w) => w.name === name)
  let name = 'New workflow'
  for (let n = 2; taken(name); n++) name = `New workflow ${n}`
  const workflow: Workflow = { id: newId(), name, steps: [newStep([])] }
  updateWorkflow(workflow)
  set({ workflowId: workflow.id })
}

export function duplicateWorkflow(id: string): void {
  const source = get().workflows.find((w) => w.id === id)
  if (!source) return
  const ids = new Map(source.steps.map((s) => [s.id, newId()]))
  const copy: Workflow = {
    ...source,
    id: newId(),
    name: `${source.name} copy`.slice(0, 60),
    steps: source.steps.map((s) => ({ ...s, id: ids.get(s.id)!, ...(s.continues !== undefined && ids.has(s.continues) ? { continues: ids.get(s.continues)! } : {}) }))
  }
  updateWorkflow(copy)
  set({ workflowId: copy.id })
}

export async function deleteWorkflow(id: string): Promise<void> {
  const workflow = get().workflows.find((w) => w.id === id)
  if (!workflow) return
  const ok = await ask({ title: 'Delete workflow', message: `"${workflow.name || 'Untitled'}" and its ${workflow.steps.length} steps are deleted.`, confirmLabel: 'Delete', danger: true })
  if (!ok) return
  clearTimeout(unsaved.get(id))
  unsaved.delete(id)
  const rest = get().workflows.filter((w) => w.id !== id)
  set({ workflows: rest, workflowId: get().workflowId === id ? (rest[0]?.id ?? null) : get().workflowId })
  await attempt(mc.invoke('workflows:delete', id))
}

// The run the user just moved along by hand: the tab that opens for it is what they want to see.
let awaited: { runId: string | null; until: number } | null = null
const AWAITED_MS = 15_000

function awaitTabOf(runId: string | null): void {
  awaited = { runId, until: Date.now() + AWAITED_MS }
}

/** Starts a run of a workflow in a folder. True when it started. */
export async function runWorkflow(workflowId: string, cwd: string, task: string): Promise<boolean> {
  // the main process runs what it has saved
  await sendWorkflow(workflowId)
  awaitTabOf(null)
  const run = await attempt(mc.invoke('workflows:run', { workflowId, cwd, task }, { groupId: get().layout.focused }))
  if (!run) return false
  awaitTabOf(run.id)
  set({ modal: null, panel: null })
  return true
}

export function approveRun(id: string): void {
  awaitTabOf(id)
  void attempt(mc.invoke('workflows:approve', id))
}

export function retryRun(id: string): void {
  awaitTabOf(id)
  void attempt(mc.invoke('workflows:retry', id))
}

export async function stopRun(id: string): Promise<void> {
  const run = get().runs.find((r) => r.id === id)
  if (!run) return
  const ok = await ask({
    title: 'Stop workflow',
    message: `"${run.name}" stops at "${run.steps[run.current]?.name}". Its tabs stay open and can be used as usual; the steps that are left do not run.`,
    confirmLabel: 'Stop',
    danger: true
  })
  if (ok) await attempt(mc.invoke('workflows:stop', id))
}

export function dismissRun(id: string): void {
  void attempt(mc.invoke('workflows:dismiss', id))
}

/** Finishes the current step of a run with what its tab last said. */
export async function takeAnswer(id: string): Promise<void> {
  const taken = await attempt(mc.invoke('workflows:takeAnswer', id))
  if (taken === false) toast('warn', 'Claude has not answered in that tab yet.')
}

function applyRuns(runs: WorkflowRun[]): void {
  const before = get().runs
  set({ runs })
  for (const run of runs) {
    const old = before.find((r) => r.id === run.id)
    run.steps.forEach((step, index) => {
      // a step whose tab has just opened
      if (!step.sessionId || step.state !== 'running' || old?.steps[index]?.state !== 'starting') return
      const asked = awaited !== null && Date.now() < awaited.until && (awaited.runId === run.id || awaited.runId === null)
      const previous = index > 0 ? old.steps[index - 1].sessionId : undefined
      // a run that moves on by itself only takes the front when the user was watching its last step
      if (asked || (previous !== undefined && get().activeId === previous)) setActive(step.sessionId)
      if (asked) awaited = null
    })
  }
}

// one notch of a mouse wheel; a touchpad pinch arrives as many small steps
const WHEEL_STEP = 100

function listenForShortcuts(): void {
  // One table for the whole window. Taking a matched key here, on its way down to the focused
  // element, keeps it from the terminal as well.
  window.addEventListener(
    'keydown',
    (e) => {
      const { modal, confirm, panel } = get()
      // a panel covers the tabs these keys act on
      if (modal || confirm || panel) return
      const action = matchShortcut(e)
      if (!action) return
      e.preventDefault()
      e.stopPropagation()
      runShortcut(action)
    },
    true
  )

  let travelled = 0
  window.addEventListener(
    'wheel',
    (e) => {
      if (!e.ctrlKey || !(e.target instanceof Element) || !e.target.closest('.workspace')) return
      // instead of scrolling the terminal
      e.preventDefault()
      e.stopPropagation()
      travelled += e.deltaY
      if (Math.abs(travelled) < WHEEL_STEP) return
      zoom(travelled < 0 ? 1 : -1)
      travelled = 0
    },
    { capture: true, passive: false }
  )
}

let started = false

export async function init(): Promise<void> {
  if (started) return
  started = true
  mc.on('pty:data', handlePtyData)
  mc.on('sessions:changed', applySessions)
  mc.on('profiles:changed', (profiles) => set({ profiles }))
  mc.on('usage:changed', (usage) => set({ usage }))
  mc.on('app:changed', (changed) => {
    // a zoom step that is not saved yet is newer than what the main process knows
    const settings = saveFontSize ? { ...changed.settings, fontSize: get().settings.fontSize } : changed.settings
    if (settings.fontSize !== get().settings.fontSize) setFontSize(settings.fontSize)
    set({ ...changed, settings })
  })
  mc.on('app:notice', (notice) => toast(notice.level, notice.message))
  mc.on('app:openFolder', ({ cwd }) => openFolderFromOutside(cwd))
  mc.on('ui:newSession', () => {
    if (get().claude.path) openNewSession()
  })
  mc.on('sessions:activate', ({ id }) => {
    if (!get().sessions.some((s) => s.id === id)) return
    // the tab may be under a panel
    closePanel()
    setActive(id)
  })
  mc.on('workflows:changed', applyWorkflows)
  mc.on('runs:changed', applyRuns)
  setInterval(() => set({ now: Date.now() }), 30_000)
  // looking at the window again counts as having seen what is on screen
  window.addEventListener('focus', markSeen)
  listenForShortcuts()
  getAppVersion().then(
    (appVersion) => set({ appVersion }),
    (err: unknown) => console.warn('The app version is not known:', errorMessage(err))
  )

  // an event sent before its listener is registered would be lost, and the state below is only kept up to date by events
  await listening()
  const { layout: known, ...state } = await mc.invoke('app:getState')
  // after a reload the main process still knows how the tabs were arranged
  const layout = reconcile((known && parseLayout(known)) || emptyLayout(), state.sessions)
  set({ ...state, ready: true, layout, activeId: activeIdOf(layout) })
  // from here on the main process may deliver folders that were queued while the window loaded
  await mc.invoke('app:ready')
}
