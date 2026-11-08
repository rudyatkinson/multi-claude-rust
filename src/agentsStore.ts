import { create } from 'zustand'
import { agentKey, agentStem, inspectAgent, type AgentDetail, type AgentListing, type AgentRef } from '@shared/agents'
import { nameOf, parseDoc } from '@shared/frontmatter'
import { errorMessage, mc } from './api'
import { NO_DRAFT, draftActions, isDirty, type DraftState } from './draft'
import { registerSection, reportUnsaved } from './library'
import { ask, attempt, toast } from './store'

interface AgentsState extends DraftState<AgentDetail> {
  listing: AgentListing | null
  loading: boolean
  tab: 'form' | 'source'
  query: string
}

/**
 * The agent manager's own state. The draft lives here and not in the panel, so that closing the
 * panel (to look at a tab underneath, say) never loses what was typed.
 */
export const useAgents = create<AgentsState>(() => ({ ...NO_DRAFT, listing: null, loading: false, tab: 'form', query: '' }))

const set = useAgents.setState
const get = useAgents.getState

/** What an agent is called: the name in its file, or its file when it has none. */
export const agentLabel = (agent: { name?: string; file: string }): string => agent.name || agentStem(agent.file)

/** The name in the text of the draft, as far as it can be read. */
const draftName = (detail: AgentDetail, text: string): string => nameOf(parseDoc(text)) ?? agentStem(detail.file)

const draft = draftActions<AgentRef, AgentDetail>({
  get,
  set,
  key: agentKey,
  label: (detail) => `"${draftName(detail, detail.text)}"`,
  read: (ref) => mc.invoke('agents:read', { rootId: ref.rootId, file: ref.file }),
  write: (ref, text, baseHash) => mc.invoke('agents:save', { rootId: ref.rootId, file: ref.file }, text, baseHash),
  problem: ({ text }) => inspectAgent(parseDoc(text)).find((issue) => issue.level === 'error')?.message ?? null,
  saved: () => void refreshAgents(),
  ask,
  fail: (err) => toast('error', errorMessage(err)),
  warn: (message) => toast('warn', message)
})

export const { checkDisk, changeText, changeField, changeBody, loadDisk } = draft
export const openAgent = draft.open
export const saveAgent = draft.save
export const revertAgent = draft.revert

// ---------- the list ----------

export async function refreshAgents(): Promise<void> {
  set({ loading: true })
  const listing = await attempt(mc.invoke('agents:list'))
  set({ loading: false })
  if (!listing) return
  const open = get().draft
  // an agent that is gone and has nothing unsaved leaves the editor too
  const gone = open !== null && !listing.agents.some((a) => agentKey(a) === agentKey(open.detail)) && !isDirty(get())
  set(gone ? { listing, draft: null, onDisk: null } : { listing })
}

export function setQuery(query: string): void {
  set({ query })
}

export function setTab(tab: 'form' | 'source'): void {
  set({ tab })
}

// ---------- managing ----------

async function showAgent(ref: AgentRef): Promise<void> {
  await refreshAgents()
  await openAgent(ref, true)
}

export async function createAgent(rootId: string, name: string, description: string): Promise<boolean> {
  const ref = await attempt(mc.invoke('agents:create', rootId, name, description))
  if (ref) await showAgent(ref)
  return !!ref
}

export async function duplicateAgent(ref: AgentRef, name: string): Promise<boolean> {
  const copy = await attempt(mc.invoke('agents:duplicate', { rootId: ref.rootId, file: ref.file }, name))
  if (copy) await showAgent(copy)
  return !!copy
}

export async function renameAgent(ref: AgentRef, name: string): Promise<boolean> {
  const moved = await attempt(mc.invoke('agents:rename', { rootId: ref.rootId, file: ref.file }, name))
  if (!moved) return false
  if (moved.warning) toast('warn', moved.warning)
  await showAgent(moved.ref)
  return true
}

export async function deleteAgent(ref: AgentRef): Promise<void> {
  const { listing } = get()
  const open = get().draft
  const isOpen = open !== null && agentKey(open.detail) === agentKey(ref)
  const root = listing?.roots.find((r) => r.id === ref.rootId)
  const label = agentLabel(listing?.agents.find((a) => agentKey(a) === agentKey(ref)) ?? ref)
  const shared = root?.kind === 'personal' && root.profileIds.length > 1 ? ` It goes for all ${root.profileIds.length} accounts that share this folder.` : ''
  const ok = await ask({
    title: 'Delete agent',
    message: `"${label}" (${ref.file}) is moved to the bin.${shared}${isOpen && isDirty(get()) ? ' Its unsaved changes are lost.' : ''}`,
    confirmLabel: 'Delete',
    danger: true
  })
  if (!ok) return
  const done = await attempt(mc.invoke('agents:delete', { rootId: ref.rootId, file: ref.file }).then(() => true))
  if (!done) return
  if (isOpen) draft.close()
  await refreshAgents()
}

export async function transferAgent(ref: AgentRef, toRootId: string, mode: 'copy' | 'move', name: string): Promise<boolean> {
  const moved = await attempt(mc.invoke('agents:transfer', { rootId: ref.rootId, file: ref.file }, toRootId, mode, name))
  if (!moved) return false
  if (moved.warning) toast('warn', moved.warning)
  await showAgent(moved.ref)
  return true
}

/** Asks for a project folder and puts it on the list; the id of its root. */
export async function addAgentFolder(): Promise<string | null> {
  const rootId = await attempt(mc.invoke('agents:addFolder'))
  if (rootId === undefined) return null
  await refreshAgents()
  if (rootId === null && get().listing) toast('info', 'The agents of that folder are an account\'s own; they are in the list already.')
  return rootId
}

export async function removeAgentFolder(rootId: string): Promise<void> {
  await attempt(mc.invoke('agents:removeFolder', rootId))
  await refreshAgents()
}

/** The key of the agent in the editor, for marking it in the list. */
export const openKey = (state: Pick<AgentsState, 'draft'>): string | null => (state.draft ? agentKey(state.draft.detail) : null)

export { isDirty }

// ---------- keeping the Library and the main process in step ----------

registerSection('agents', { save: () => void saveAgent(), refresh: () => refreshAgents().then(checkDisk) })

useAgents.subscribe((state) => reportUnsaved('agents', isDirty(state) ? `The agent "${draftName(state.draft!.detail, state.draft!.text)}"` : null))
