import { create } from 'zustand'
import { MEMORY_INDEX, memoryKey, type MemoryDetail, type MemoryHit, type MemoryListing, type MemoryProject, type MemoryRef, type MemoryView } from '@shared/memory'
import { errorMessage, mc } from './api'
import { NO_DRAFT, draftActions, isDirty, type DraftState } from './draft'
import { splitPath } from './format'
import { registerSection, reportUnsaved } from './library'
import { ask, attempt, mainProfileName, setModal, toast } from './store'

interface MemoryState extends DraftState<MemoryDetail> {
  listing: MemoryListing | null
  /** The project that shows, by its name under `projects`. */
  projectKey: string | null
  /** Whose memory of it shows: one of the project's folders. */
  folderId: string | null
  /** The memories of that folder. */
  view: MemoryView | null
  query: string
  /** What the search found; null while nothing is searched. */
  hits: MemoryHit[] | null
}

/**
 * The memory browser's own state. The draft lives here and not in the panel, so that closing the
 * Library's tab never loses what was typed.
 */
export const useMemory = create<MemoryState>(() => ({ ...NO_DRAFT, listing: null, projectKey: null, folderId: null, view: null, query: '', hits: null }))

const set = useMemory.setState
const get = useMemory.getState

const refOf = (ref: MemoryRef): MemoryRef => ({ folderId: ref.folderId, file: ref.file })

const draft = draftActions<MemoryRef, MemoryDetail>({
  get,
  set,
  key: memoryKey,
  label: (detail) => `"${detail.file}"`,
  read: (ref) => mc.invoke('memory:read', refOf(ref)),
  write: (ref, text, baseHash) => mc.invoke('memory:save', refOf(ref), text, baseHash),
  saved: () => void loadView(),
  ask,
  fail: (err) => toast('error', errorMessage(err)),
  warn: (message) => toast('warn', message)
})

export const { checkDisk, changeText, loadDisk } = draft
export const openMemory = draft.open
export const saveMemory = draft.save
export const revertMemory = draft.revert
export { isDirty }

/** What a project is called: its folder's name, or Claude Code's name for it when the folder is not known. */
export function projectLabel(project: Pick<MemoryProject, 'key' | 'cwd'>): { name: string; where: string } {
  if (!project.cwd) return { name: project.key, where: 'Its folder is not known: no conversation of this project is left' }
  const { name, parent } = splitPath(project.cwd)
  return { name, where: parent }
}

// ---------- the lists ----------

/** Reads the memories of the folder that shows. */
async function loadView(): Promise<void> {
  const { folderId } = get()
  if (!folderId) return
  const view = await attempt(mc.invoke('memory:entries', folderId))
  // another folder may show by now
  if (view && get().folderId === folderId) set({ view })
}

export async function refreshMemory(): Promise<void> {
  const listing = await attempt(mc.invoke('memory:list'))
  if (!listing) return
  set({ listing })
  const state = get()
  const there = listing.projects.some((p) => p.key === state.projectKey && p.folders.some((f) => f.id === state.folderId))
  if (there) await loadView()
  else {
    // nothing showed yet, or the folder that did is gone with its last memory
    if (state.draft && !isDirty(state)) draft.close()
    set({ projectKey: null, folderId: null, view: null })
    const first = listing.projects[0]
    // text that is not saved stays in the editor until it is dealt with
    if (first && !get().draft) await selectProject(first.key)
  }
  if (get().query.trim()) await search(get().query)
}

/** Unsaved text in another folder than the one about to show is asked about. True to go on. */
async function leaveDraft(folderId: string): Promise<boolean> {
  const state = get()
  if (!state.draft || state.draft.detail.folderId === folderId) return true
  if (isDirty(state)) {
    const ok = await ask({
      title: 'Unsaved changes',
      message: `"${state.draft.detail.file}" has changes that are not saved. Opening another project discards them.`,
      confirmLabel: 'Discard',
      danger: true
    })
    if (!ok) return false
  }
  draft.close()
  return true
}

/** Shows one account's memory of a project, with its index in the editor. */
export async function selectFolder(projectKey: string, folderId: string): Promise<boolean> {
  if (get().folderId === folderId && get().view) return true
  if (!(await leaveDraft(folderId))) return false
  set({ projectKey, folderId, view: null })
  await loadView()
  const view = get().view
  if (view && get().folderId === folderId && !get().draft) {
    const first = view.entries.find((e) => e.file === MEMORY_INDEX) ?? view.entries[0]
    if (first) await openMemory({ folderId, file: first.file }, true)
  }
  return true
}

export async function selectProject(key: string): Promise<void> {
  const project = get().listing?.projects.find((p) => p.key === key)
  if (project?.folders[0]) await selectFolder(key, project.folders[0].id)
}

// ---------- searching ----------

let searches = 0

export async function search(query: string): Promise<void> {
  set({ query })
  const mine = ++searches
  if (!query.trim()) {
    set({ hits: null })
    return
  }
  const hits = await attempt(mc.invoke('memory:search', query))
  // a later search may have been typed meanwhile
  if (hits && mine === searches) set({ hits })
}

export async function openHit(hit: MemoryHit): Promise<void> {
  if (!(await selectFolder(hit.key, hit.folderId))) return
  await openMemory(refOf(hit))
}

// ---------- managing ----------

export async function createMemory(folderId: string, input: { name: string; title: string; description: string; type: string }): Promise<boolean> {
  const ref = await attempt(mc.invoke('memory:create', folderId, input))
  if (!ref) return false
  await refreshMemory()
  await openMemory(ref, true)
  return true
}

export async function deleteMemory(ref: MemoryRef): Promise<void> {
  const open = get().draft
  const isOpen = open !== null && memoryKey(open.detail) === memoryKey(ref)
  const folder = get()
    .listing?.projects.flatMap((p) => p.folders)
    .find((f) => f.id === ref.folderId)
  const shared = folder && folder.sharedWith.length > 0 ? ' The accounts that share this memory lose it as well.' : ''
  const ok = await ask({
    title: 'Delete memory',
    message: `"${ref.file}" is moved to the bin, and its line is taken out of MEMORY.md.${shared}${isOpen && isDirty(get()) ? ' Its unsaved changes are lost.' : ''}`,
    confirmLabel: 'Delete',
    danger: true
  })
  if (!ok) return
  const done = await attempt(mc.invoke('memory:delete', refOf(ref)).then(() => true))
  if (!done) return
  if (isOpen) draft.close()
  await refreshMemory()
  await checkDisk()
}

// ---------- sharing ----------

/** Lets an account's sessions use the main account's memory, or its own again. */
export async function setSharing(profileId: string, on: boolean): Promise<void> {
  await attempt(mc.invoke('profiles:update', profileId, { shareMemory: on }))
  await refreshMemory()
}

/** Looks what bringing a folder's memories into the main account's would do, and shows it. */
export async function planAdoption(folderId: string): Promise<void> {
  const plan = await attempt(mc.invoke('memory:adopt', folderId, false))
  if (plan) setModal({ type: 'memoryAdopt', folderId, plan })
}

export async function adoptMemories(folderId: string): Promise<boolean> {
  const result = await attempt(mc.invoke('memory:adopt', folderId, true))
  if (!result) return false
  const count = result.copy.length
  toast('info', count === 0 ? 'There was nothing left to bring in.' : `${count} ${count === 1 ? 'memory was' : 'memories were'} copied into ${mainProfileName()}'s memory.`)
  await refreshMemory()
  await checkDisk()
  return true
}

/** The key of the memory in the editor, for marking it in the list. */
export const openKey = (state: Pick<MemoryState, 'draft'>): string | null => (state.draft ? memoryKey(state.draft.detail) : null)

// ---------- keeping the Library and the main process in step ----------

registerSection('memory', { save: () => void saveMemory(), refresh: () => refreshMemory().then(checkDisk) })

useMemory.subscribe((state) => reportUnsaved('memory', isDirty(state) ? `The memory file "${state.draft!.detail.file}"` : null))
