import { create } from 'zustand'
import { skillKey, type SkillDetail, type SkillListing, type SkillRef } from '@shared/skills'
import { inspectSkill, parseSkill, setBody, setField, type FieldInput } from '@shared/skillText'
import { mc } from './api'
import { registerSection, reportUnsaved } from './library'
import { ask, attempt, closePanel, setActive, setModal, toast, useStore } from './store'

/** The skill in the editor: as it was read from disk, and its text as it stands now. */
interface Draft {
  detail: SkillDetail
  text: string
}

interface SkillsState {
  listing: SkillListing | null
  loading: boolean
  draft: Draft | null
  /** What is on disk by now, when it changed there while the draft has changes of its own. */
  onDisk: SkillDetail | null
  tab: 'form' | 'source'
  saving: boolean
  query: string
}

/**
 * The skill manager's own state. The draft lives here and not in the panel, so that closing the
 * panel (to look at a tab underneath, say) never loses what was typed.
 */
export const useSkills = create<SkillsState>(() => ({ listing: null, loading: false, draft: null, onDisk: null, tab: 'form', saving: false, query: '' }))

const set = useSkills.setState
const get = useSkills.getState

export const isDirty = (state: Pick<SkillsState, 'draft'>): boolean => state.draft !== null && state.draft.text !== state.draft.detail.text

const sameRef = (a: SkillRef, b: SkillRef): boolean => a.rootId === b.rootId && a.dirName === b.dirName

export { rootLabel } from './library'

// ---------- the list ----------

export async function refreshSkills(): Promise<void> {
  set({ loading: true })
  const listing = await attempt(mc.invoke('skills:list'))
  set({ loading: false })
  if (!listing) return
  const { draft } = get()
  // a skill that is gone and has nothing unsaved leaves the editor too
  const gone = draft !== null && !listing.skills.some((s) => sameRef(s, draft.detail)) && !isDirty(get())
  set(gone ? { listing, draft: null, onDisk: null } : { listing })
}

export function setQuery(query: string): void {
  set({ query })
}

export function setTab(tab: 'form' | 'source'): void {
  set({ tab })
}

/** Shows a skill in the editor. Asks first when the one that is open has unsaved changes. */
export async function openSkill(ref: SkillRef, sure = false): Promise<void> {
  const { draft } = get()
  if (draft && sameRef(draft.detail, ref) && !sure) return
  if (!sure && isDirty(get())) {
    const ok = await ask({
      title: 'Unsaved changes',
      message: `"${draft!.detail.dirName}" has changes that are not saved. Opening another skill discards them.`,
      confirmLabel: 'Discard',
      danger: true
    })
    if (!ok) return
  }
  const detail = await attempt(mc.invoke('skills:read', ref))
  if (detail) set({ draft: { detail, text: detail.text }, onDisk: null })
}

/**
 * Looks whether the open skill changed on disk, as it does when a Claude session edits it.
 * Without unsaved changes the editor simply shows the new text; with them it says so and waits.
 */
export async function checkDisk(): Promise<void> {
  const { draft } = get()
  if (!draft) return
  let fresh: SkillDetail
  try {
    fresh = await mc.invoke('skills:read', draft.detail)
  } catch {
    // gone or unreadable: the list says so when it is read again
    return
  }
  const now = get().draft
  if (!now || !sameRef(now.detail, fresh) || fresh.hash === now.detail.hash) {
    // the files next to it may have changed all the same
    if (now && sameRef(now.detail, fresh)) set({ draft: { ...now, detail: { ...now.detail, files: fresh.files, moreFiles: fresh.moreFiles } } })
    return
  }
  if (isDirty(get())) set({ onDisk: fresh })
  else set({ draft: { detail: fresh, text: fresh.text }, onDisk: null })
}

// ---------- editing ----------

export function changeText(text: string): void {
  const { draft } = get()
  if (draft && draft.text !== text) set({ draft: { ...draft, text } })
}

// one message for a value that cannot be written, not one per key press
let lastRefusal = ''

/** Sets a field of the frontmatter in the draft; null or an empty text takes it out. */
export function changeField(key: string, value: FieldInput): void {
  const { draft } = get()
  if (!draft) return
  const result = setField(draft.text, key, value)
  if ('text' in result) {
    lastRefusal = ''
    changeText(result.text)
  } else if (result.failed !== lastRefusal) {
    lastRefusal = result.failed
    toast('warn', result.failed)
  }
}

export function changeBody(body: string): void {
  const { draft } = get()
  if (draft) changeText(setBody(draft.text, body))
}

/** Saves the draft. `overwrite` writes it over a file that changed on disk in the meantime. */
export async function saveSkill(overwrite = false): Promise<void> {
  const { draft, saving } = get()
  if (!draft || saving || draft.detail.readOnly || (!isDirty(get()) && !overwrite)) return
  const broken = inspectSkill(parseSkill(draft.text), draft.detail.dirName).find((issue) => issue.scope === 'claude' && issue.level === 'error')
  if (broken) {
    const ok = await ask({ title: 'Save with a problem', message: `${broken.message}\n\nSave it like this anyway?`, confirmLabel: 'Save anyway' })
    if (!ok) return
  }
  set({ saving: true })
  const result = await attempt(mc.invoke('skills:save', draft.detail, draft.text, overwrite ? null : draft.detail.hash))
  set({ saving: false })
  if (!result) return
  if ('conflict' in result) {
    set({ onDisk: result.conflict })
    return
  }
  // what was typed while the file was being written stays in the editor
  const typed = get().draft
  set({ draft: { detail: result.saved, text: typed && typed.text !== draft.text ? typed.text : result.saved.text }, onDisk: null })
  void refreshSkills()
}

/** Takes the version on disk in place of the draft, after a change there. */
export function loadDisk(): void {
  const { onDisk } = get()
  if (onDisk) set({ draft: { detail: onDisk, text: onDisk.text }, onDisk: null })
}

export async function revertSkill(): Promise<void> {
  const { draft } = get()
  if (!draft || !isDirty(get())) return
  const ok = await ask({ title: 'Revert', message: `The changes to "${draft.detail.dirName}" that are not saved are discarded.`, confirmLabel: 'Revert', danger: true })
  if (ok) await openSkill(draft.detail, true)
}

// ---------- managing ----------

async function showSkill(ref: SkillRef): Promise<void> {
  await refreshSkills()
  await openSkill(ref, true)
}

export async function createSkill(rootId: string, name: string, description: string): Promise<SkillRef | null> {
  const ref = await attempt(mc.invoke('skills:create', rootId, name, description))
  if (!ref) return null
  await showSkill(ref)
  return ref
}

export async function duplicateSkill(ref: SkillRef, name: string): Promise<boolean> {
  const copy = await attempt(mc.invoke('skills:duplicate', ref, name))
  if (copy) await showSkill(copy)
  return !!copy
}

export async function renameSkill(ref: SkillRef, name: string): Promise<boolean> {
  const moved = await attempt(mc.invoke('skills:rename', ref, name))
  if (!moved) return false
  if (moved.warning) toast('warn', moved.warning)
  await showSkill(moved.ref)
  return true
}

export async function deleteSkill(ref: SkillRef): Promise<void> {
  const { draft, listing } = get()
  const open = draft !== null && sameRef(draft.detail, ref)
  const linked = listing?.skills.find((s) => sameRef(s, ref))?.linked
  const root = listing?.roots.find((r) => r.id === ref.rootId)
  const shared = root?.kind === 'personal' && root.profileIds.length > 1 ? ` It goes for all ${root.profileIds.length} accounts that share this folder.` : ''
  const ok = await ask({
    title: 'Delete skill',
    message: linked
      ? `"${ref.dirName}" is a link to a folder elsewhere. Only the link is removed; the folder it points to stays.`
      : `"${ref.dirName}" and its files are moved to the bin.${shared}${open && isDirty(get()) ? ' Its unsaved changes are lost.' : ''}`,
    confirmLabel: 'Delete',
    danger: true
  })
  if (!ok) return
  const done = await attempt(mc.invoke('skills:delete', ref).then(() => true))
  if (!done) return
  if (open) set({ draft: null, onDisk: null })
  await refreshSkills()
}

export async function transferSkill(ref: SkillRef, toRootId: string, mode: 'copy' | 'move', name: string): Promise<boolean> {
  const moved = await attempt(mc.invoke('skills:transfer', ref, toRootId, mode, name))
  if (!moved) return false
  if (moved.warning) toast('warn', moved.warning)
  await showSkill(moved.ref)
  return true
}

export async function exportSkill(ref: SkillRef): Promise<void> {
  const file = await attempt(mc.invoke('skills:export', ref))
  if (file) toast('info', `Exported to ${file}`)
}

/** Asks for an archive; what it holds is then shown before anything is brought in. */
export async function pickImport(): Promise<void> {
  const preview = await attempt(mc.invoke('skills:pickImport'))
  if (preview) setModal({ type: 'skillImport', preview })
}

export async function importSkill(token: string, rootId: string, name: string): Promise<boolean> {
  const ref = await attempt(mc.invoke('skills:import', token, rootId, name))
  if (ref) await showSkill(ref)
  return !!ref
}

/** Opens a session that works on the skill, and shows its tab. */
export async function askClaude(ref: SkillRef, profileId: string, request: string): Promise<boolean> {
  const info = await attempt(mc.invoke('skills:assist', ref, profileId, request, { groupId: useStore.getState().layout.focused }))
  if (!info) return false
  setModal(null)
  closePanel()
  setActive(info.id)
  return true
}

/** Asks for a project folder and puts it on the list; the id of its root. */
export async function addSkillFolder(): Promise<string | null> {
  const rootId = await attempt(mc.invoke('skills:addFolder'))
  if (rootId === undefined) return null
  await refreshSkills()
  if (rootId === null && get().listing) toast('info', 'The skills of that folder are an account\'s own; they are in the list already.')
  return rootId
}

export async function removeSkillFolder(rootId: string): Promise<void> {
  await attempt(mc.invoke('skills:removeFolder', rootId))
  await refreshSkills()
}

/** The key of the skill in the editor, for marking it in the list. */
export const openKey = (state: Pick<SkillsState, 'draft'>): string | null => (state.draft ? skillKey(state.draft.detail) : null)

// ---------- keeping the Library and the main process in step ----------

registerSection('skills', { save: () => void saveSkill(), refresh: () => refreshSkills().then(checkDisk) })

useSkills.subscribe((state) => reportUnsaved('skills', isDirty(state) ? `The skill "${state.draft!.detail.dirName}"` : null))
