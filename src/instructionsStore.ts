import { create } from 'zustand'
import { instructionKey, type InstructionDetail, type InstructionFile, type InstructionRef, type InstructionScope, type LoadStep } from '@shared/instructions'
import { errorMessage, mc } from './api'
import { NO_DRAFT, draftActions, isDirty, type DraftState } from './draft'
import { registerSection, reportUnsaved } from './library'
import { ask, attempt, toast, useStore } from './store'

interface InstructionsState extends DraftState<InstructionDetail> {
  scopes: InstructionScope[] | null
  /** The account or project folder that shows. */
  scopeId: string | null
  /** Its files. */
  files: InstructionFile[] | null
  /** A file of the scope that is chosen but not there yet, or a copy that is not opened as it is. */
  chosen: string | null
  /** The right side shows what a session reads, in place of a file. */
  showOrder: boolean
  /** Whose sessions the load order is of. */
  orderProfileId: string | null
  order: LoadStep[] | null
  /** Accounts whose copy of the main account's CLAUDE.md is edited as it is, by scope. */
  editCopy: Record<string, true>
}

/**
 * The instructions editor's own state. The draft lives here and not in the panel, so that
 * closing the Library's tab never loses what was typed.
 */
export const useInstructions = create<InstructionsState>(() => ({
  ...NO_DRAFT,
  scopes: null,
  scopeId: null,
  files: null,
  chosen: null,
  showOrder: false,
  orderProfileId: null,
  order: null,
  editCopy: {}
}))

const set = useInstructions.setState
const get = useInstructions.getState

const refOf = (ref: InstructionRef): InstructionRef => ({ scopeId: ref.scopeId, file: ref.file })

const draft = draftActions<InstructionRef, InstructionDetail>({
  get,
  set,
  key: instructionKey,
  label: (detail) => `"${detail.file}"`,
  read: (ref) => mc.invoke('instructions:read', refOf(ref)),
  write: (ref, text, baseHash) => mc.invoke('instructions:save', refOf(ref), text, baseHash),
  saved: () => void loadFiles(),
  ask,
  fail: (err) => toast('error', errorMessage(err)),
  warn: (message) => toast('warn', message)
})

export const { checkDisk, changeText, loadDisk } = draft
export const saveInstruction = draft.save
export const revertInstruction = draft.revert
export { isDirty }

// ---------- the lists ----------

async function loadFiles(): Promise<void> {
  const { scopeId } = get()
  if (!scopeId) return
  const files = await attempt(mc.invoke('instructions:files', scopeId))
  // another scope may show by now
  if (files && get().scopeId === scopeId) set({ files })
}

async function loadOrder(): Promise<void> {
  const { scopeId, orderProfileId, showOrder } = get()
  if (!showOrder || !scopeId || !orderProfileId) return
  const order = await attempt(mc.invoke('instructions:view', orderProfileId, scopeId))
  if (order && get().scopeId === scopeId && get().orderProfileId === orderProfileId) set({ order })
}

export async function refreshInstructions(): Promise<void> {
  const scopes = await attempt(mc.invoke('instructions:scopes'))
  if (!scopes) return
  set({ scopes })
  const state = get()
  if (scopes.some((s) => s.id === state.scopeId)) {
    await loadFiles()
    await loadOrder()
    return
  }
  // nothing showed yet, or the folder that did left the list
  if (state.draft && !isDirty(state)) draft.close()
  set({ scopeId: null, files: null, chosen: null, showOrder: false, order: null })
  if (scopes[0] && !get().draft) await selectScope(scopes[0].id)
}

/** Unsaved text in another scope than the one about to show is asked about. True to go on. */
async function leaveDraft(scopeId: string): Promise<boolean> {
  const state = get()
  if (!state.draft || state.draft.detail.scopeId === scopeId) return true
  if (isDirty(state)) {
    const ok = await ask({
      title: 'Unsaved changes',
      message: `"${state.draft.detail.file}" has changes that are not saved. Opening another folder discards them.`,
      confirmLabel: 'Discard',
      danger: true
    })
    if (!ok) return false
  }
  draft.close()
  return true
}

/** Whether a file of a scope is a copy that is not opened as it is. */
export function isHeldCopy(scope: InstructionScope | undefined, file: string, editCopy: Record<string, true>): boolean {
  return scope?.kind === 'account' && scope.syncedClaudeMd === true && file === 'CLAUDE.md' && !editCopy[scope.id]
}

/** Shows the files of an account or a project folder, with the first one that is there in the editor. */
export async function selectScope(scopeId: string): Promise<boolean> {
  if (get().scopeId === scopeId && get().files) return true
  if (!(await leaveDraft(scopeId))) return false
  set({ scopeId, files: null, chosen: null, showOrder: false, order: null })
  await loadFiles()
  const { files, scopes, editCopy } = get()
  if (files && get().scopeId === scopeId && !get().draft) {
    const scope = scopes?.find((s) => s.id === scopeId)
    const first = files.find((f) => f.exists && !isHeldCopy(scope, f.file, editCopy)) ?? files[0]
    if (first) await chooseFile(first.file)
  }
  return true
}

/** Shows a file of the scope: in the editor when it is there, else with the offer to make it. */
export async function chooseFile(file: string): Promise<void> {
  const { scopeId, files, scopes, editCopy } = get()
  if (!scopeId) return
  const entry = files?.find((f) => f.file === file)
  const held = isHeldCopy(scopes?.find((s) => s.id === scopeId), file, editCopy)
  if (!entry?.exists || held) {
    // nothing to open; unsaved text of another file is asked about first
    const open = get().draft
    if (open && isDirty(get())) {
      const ok = await ask({ title: 'Unsaved changes', message: `"${open.detail.file}" has changes that are not saved. Leaving it discards them.`, confirmLabel: 'Discard', danger: true })
      if (!ok) return
    }
    draft.close()
    set({ chosen: file, showOrder: false })
    return
  }
  if (await draft.open({ scopeId, file })) set({ chosen: null, showOrder: false })
}

/** Opens the account's copy of the main account's CLAUDE.md for editing after all. */
export async function editCopyAnyway(scopeId: string): Promise<void> {
  set({ editCopy: { ...get().editCopy, [scopeId]: true } })
  await chooseFile('CLAUDE.md')
}

/** Goes to the main account's CLAUDE.md, the file an account's copy is made from. */
export async function openMainClaudeMd(): Promise<void> {
  const mainId = useStore.getState().profiles.find((p) => p.isDefault)?.id
  const scope = get().scopes?.find((s) => s.kind === 'account' && s.profileId === mainId)
  if (!scope || !(await selectScope(scope.id))) return
  await chooseFile('CLAUDE.md')
}

// ---------- what a session reads ----------

export async function showLoadOrder(profileId?: string): Promise<void> {
  const state = get()
  const profiles = useStore.getState().profiles
  const chosen = profileId ?? state.orderProfileId ?? profiles.find((p) => p.isDefault)?.id ?? profiles[0]?.id ?? null
  if (!chosen) return
  if (state.draft && isDirty(state)) {
    const ok = await ask({ title: 'Unsaved changes', message: `"${state.draft.detail.file}" has changes that are not saved. Leaving it discards them.`, confirmLabel: 'Discard', danger: true })
    if (!ok) return
  }
  draft.close()
  set({ showOrder: true, orderProfileId: chosen, order: chosen === state.orderProfileId ? state.order : null, chosen: null })
  await loadOrder()
}

// ---------- managing ----------

export async function createInstruction(scopeId: string, target: { file: string } | { rule: string }): Promise<boolean> {
  const ref = await attempt(mc.invoke('instructions:create', scopeId, target))
  if (!ref) return false
  await loadFiles()
  await chooseFile(ref.file)
  return true
}

export async function deleteInstruction(ref: InstructionRef): Promise<void> {
  const open = get().draft
  const isOpen = open !== null && instructionKey(open.detail) === instructionKey(ref)
  const ok = await ask({
    title: 'Delete file',
    message: `"${ref.file}" is moved to the bin.${isOpen && isDirty(get()) ? ' Its unsaved changes are lost.' : ''} Sessions that start from now on no longer read it.`,
    confirmLabel: 'Delete',
    danger: true
  })
  if (!ok) return
  const done = await attempt(mc.invoke('instructions:delete', refOf(ref)).then(() => true))
  if (!done) return
  if (isOpen) draft.close()
  set({ chosen: ref.file })
  await loadFiles()
  // a rule that is gone has no place left; a fixed file offers to be made again
  if (!get().files?.some((f) => f.file === ref.file)) set({ chosen: null })
}

/** Asks for a project folder and shows its files. */
export async function addInstructionFolder(): Promise<void> {
  const scopeId = await attempt(mc.invoke('instructions:addFolder'))
  // nothing was chosen, or the folder is an account's own
  if (!scopeId) return
  await refreshInstructions()
  await selectScope(scopeId)
}

/** The key of the file in the editor, for marking it in the list. */
export const openFile = (state: Pick<InstructionsState, 'draft' | 'chosen' | 'scopeId'>): string | null => state.draft?.detail.file ?? state.chosen

// ---------- keeping the Library and the main process in step ----------

registerSection('instructions', { save: () => void saveInstruction(), refresh: () => refreshInstructions().then(checkDisk) })

useInstructions.subscribe((state) => reportUnsaved('instructions', isDirty(state) ? `The instructions file "${state.draft!.detail.file}"` : null))
