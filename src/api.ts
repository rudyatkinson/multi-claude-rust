/**
 * The bridge between the window and the Rust backend.
 *
 * The renderer still talks in the channels of `@shared/ipc` (`mc.invoke('profiles:update', id, patch)`);
 * this file turns them into Tauri calls. The backend mirrors the rules and the table below exactly.
 *
 * Commands
 *   - The command of a channel is the channel with `:` replaced by `_`: `profiles:create` is the
 *     command `profiles_create`; camelCase becomes snake_case, so `app:getState` is `app_get_state`
 *     and `skills:pickImport` is `skills_pick_import` (see `commandName`).
 *   - A command takes named arguments. `INVOKE_PARAMS` lists the names of each channel's arguments
 *     in the order the channel takes them: `mc.invoke('skills:save', ref, text, baseHash)` calls
 *     `invoke('skills_save', { ref, text, baseHash })`.
 *   - The names are the keys of the JSON object, in camelCase. That is what `#[tauri::command]`
 *     expects by default: the Rust parameter `base_hash` receives the key `baseHash`, and `r#ref`
 *     receives `ref`. An argument that is `undefined` is left out, so an optional one is an `Option`.
 *   - A failed command rejects with the error the backend returns; a plain string is shown as is.
 *
 * Fire-and-forget messages (`SEND_CHANNELS`) are commands too, with one argument named `payload`:
 * `mc.send('pty:write', { id, data })` calls `invoke('pty_write', { payload: { id, data } })`.
 * The calls of one channel and one session go out one at a time, in order (see `send`).
 *
 * Events keep the channel names of `IpcEvents` (`pty:data`, `sessions:changed`, ...); their
 * payload is the type listed there.
 */
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { IpcEvents, IpcInvoke, IpcSend, McApi } from '@shared/ipc'

/** One name per argument of a channel; the type makes sure there are as many names as arguments. */
type ParamNames<T extends readonly unknown[]> = { readonly [I in keyof T]-?: string }

/** The names of each channel's arguments, in the order the channel takes them. The backend uses the same names. */
export const INVOKE_PARAMS: { readonly [K in keyof IpcInvoke]: ParamNames<Parameters<IpcInvoke[K]>> } = {
  'app:getState': [],
  'app:ready': [],
  'profiles:create': ['input'],
  'profiles:update': ['id', 'patch'],
  'profiles:delete': ['id'],
  'profiles:login': ['id'],
  'profiles:logout': ['id'],
  'profiles:setToken': ['id', 'token'],
  'profiles:refreshStatus': ['id'],
  'profiles:syncFromMain': ['id', 'force'],
  'profiles:openConfigDir': ['id'],
  'sessions:create': ['input'],
  'sessions:attach': ['id'],
  'sessions:close': ['id'],
  'sessions:closeMany': ['ids'],
  'sessions:rename': ['id', 'name'],
  'sessions:revealFolder': ['id'],
  'sessions:transfer': ['id', 'profileId'],
  'sessions:restart': ['id', 'continueLast'],
  'sessions:openExternal': ['input'],
  'dialog:pickFolder': ['defaultPath'],
  'fs:folderFor': ['path'],
  'conversations:list': [],
  'conversations:open': ['profileId', 'id', 'place'],
  'folders:setAccount': ['cwd', 'profileId'],
  'mcp:list': ['profileId'],
  'mcp:copy': ['profileId', 'names'],
  'workflows:save': ['workflow'],
  'workflows:delete': ['id'],
  'workflows:run': ['input', 'place'],
  'workflows:approve': ['runId'],
  'workflows:retry': ['runId'],
  'workflows:stop': ['runId'],
  'workflows:dismiss': ['runId'],
  'workflows:takeAnswer': ['runId'],
  'workflows:output': ['runId', 'index'],
  'skills:list': [],
  'skills:read': ['ref'],
  'skills:save': ['ref', 'text', 'baseHash'],
  'skills:create': ['rootId', 'name', 'description'],
  'skills:duplicate': ['ref', 'name'],
  'skills:rename': ['ref', 'name'],
  'skills:delete': ['ref'],
  'skills:transfer': ['ref', 'toRootId', 'mode', 'name'],
  'skills:export': ['ref'],
  'skills:pickImport': [],
  'skills:import': ['token', 'rootId', 'name'],
  'skills:reveal': ['ref', 'file'],
  'skills:openFile': ['ref', 'file'],
  'skills:assist': ['ref', 'profileId', 'request', 'place'],
  'skills:addFolder': [],
  'skills:removeFolder': ['rootId'],
  'agents:list': [],
  'agents:read': ['ref'],
  'agents:save': ['ref', 'text', 'baseHash'],
  'agents:create': ['rootId', 'name', 'description'],
  'agents:duplicate': ['ref', 'name'],
  'agents:rename': ['ref', 'name'],
  'agents:delete': ['ref'],
  'agents:transfer': ['ref', 'toRootId', 'mode', 'name'],
  'agents:reveal': ['ref'],
  'agents:addFolder': [],
  'agents:removeFolder': ['rootId'],
  'memory:list': [],
  'memory:entries': ['folderId'],
  'memory:read': ['ref'],
  'memory:save': ['ref', 'text', 'baseHash'],
  'memory:create': ['folderId', 'input'],
  'memory:delete': ['ref'],
  'memory:search': ['query'],
  'memory:adopt': ['folderId', 'apply'],
  'memory:reveal': ['folderId', 'file'],
  'instructions:scopes': [],
  'instructions:files': ['scopeId'],
  'instructions:read': ['ref'],
  'instructions:save': ['ref', 'text', 'baseHash'],
  'instructions:create': ['scopeId', 'target'],
  'instructions:delete': ['ref'],
  'instructions:view': ['profileId', 'scopeId'],
  'instructions:reveal': ['scopeId', 'file'],
  'instructions:addFolder': [],
  'settings:update': ['patch'],
  'shell:openExternal': ['url'],
  'clipboard:read': [],
  'clipboard:writeText': ['text'],
  'explorerMenu:status': [],
  'explorerMenu:set': ['enabled']
}

/** The Tauri command behind a channel: `profiles:create` is `profiles_create`. */
export function commandName(channel: keyof IpcInvoke | keyof IpcSend): string {
  return channel.replaceAll(':', '_').replace(/[A-Z]/g, (c) => '_' + c.toLowerCase())
}

/** The named arguments of a call: each positional argument under its name, `undefined` ones left out. */
export function namedArgs(channel: keyof IpcInvoke, args: readonly unknown[]): Record<string, unknown> {
  const names: readonly string[] = INVOKE_PARAMS[channel]
  const named: Record<string, unknown> = {}
  names.forEach((name, i) => {
    if (args[i] !== undefined) named[name] = args[i]
  })
  return named
}

// ---------- fire-and-forget messages ----------

/**
 * Separate commands are not guaranteed to run in the order they were called, and keystrokes must
 * reach a terminal in the order they were typed. So each lane (a channel, and for messages about
 * one session that session) has at most one call on the way. What comes meanwhile waits: typed
 * text is added up, and of anything else only the latest message is kept, since it replaces the
 * ones before it (a size, an arrangement, a list).
 */
interface Lane {
  waiting: unknown
  hasWaiting: boolean
}

const lanes = new Map<string, Lane>()

function laneOf(channel: keyof IpcSend, payload: unknown): string {
  const id = typeof payload === 'object' && payload !== null && 'id' in payload ? payload.id : undefined
  return typeof id === 'string' ? `${channel}\u0000${id}` : channel
}

function combine<K extends keyof IpcSend>(channel: K, before: IpcSend[K], next: IpcSend[K]): IpcSend[K] {
  if (channel !== 'pty:write') return next
  const a = before as IpcSend['pty:write']
  const b = next as IpcSend['pty:write']
  return { id: b.id, data: a.data + b.data } as IpcSend[K]
}

function deliver<K extends keyof IpcSend>(channel: K, key: string, payload: IpcSend[K]): void {
  invoke(commandName(channel), { payload })
    .catch((err: unknown) => console.warn(`${channel} failed:`, errorMessage(err)))
    .finally(() => {
      const lane = lanes.get(key)
      if (!lane) return
      if (!lane.hasWaiting) {
        lanes.delete(key)
        return
      }
      const next = lane.waiting as IpcSend[K]
      lane.waiting = undefined
      lane.hasWaiting = false
      deliver(channel, key, next)
    })
}

function send<K extends keyof IpcSend>(channel: K, payload: IpcSend[K]): void {
  const key = laneOf(channel, payload)
  const lane = lanes.get(key)
  if (!lane) {
    lanes.set(key, { waiting: undefined, hasWaiting: false })
    deliver(channel, key, payload)
    return
  }
  lane.waiting = lane.hasWaiting ? combine(channel, lane.waiting as IpcSend[K], payload) : payload
  lane.hasWaiting = true
}

// ---------- events ----------

// listeners whose registration with the backend is still on the way
const registering = new Set<Promise<void>>()

function on<K extends keyof IpcEvents>(channel: K, listener: (payload: IpcEvents[K]) => void): () => void {
  let unlisten: UnlistenFn | null = null
  let stopped = false
  const registered = listen<IpcEvents[K]>(channel, (event) => listener(event.payload)).then(
    (fn) => {
      if (stopped) fn()
      else unlisten = fn
    },
    (err: unknown) => console.warn(`Cannot listen to ${channel}:`, errorMessage(err))
  )
  registering.add(registered)
  void registered.then(() => registering.delete(registered))
  return () => {
    stopped = true
    unlisten?.()
    unlisten = null
  }
}

/**
 * Resolves once every listener added so far is registered with the backend. Registering takes a
 * call of its own, so an event sent before that is missed: wait for this before asking the
 * backend for the state that the events then keep up to date.
 */
export async function listening(): Promise<void> {
  await Promise.all([...registering])
}

// ---------- the bridge ----------

export const mc: McApi = {
  invoke(channel, ...args) {
    return invoke(commandName(channel), namedArgs(channel, args))
  },
  send,
  on
}

/** The version of this app, as the backend has it. */
export function getAppVersion(): Promise<string> {
  return invoke<string>('app_get_version')
}

/** The text of a failed call: the backend rejects with the message itself, or with an Error. */
export function errorMessage(err: unknown): string {
  if (typeof err === 'string') return err
  if (err instanceof Error) return err.message
  if (typeof err === 'object' && err !== null && 'message' in err && typeof err.message === 'string') return err.message
  return String(err)
}
