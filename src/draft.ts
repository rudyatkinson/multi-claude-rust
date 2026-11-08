import { setBody, setField, type FieldInput } from '@shared/frontmatter'

/**
 * A text file in an editor of the Library: how it was read from disk, and its text as it stands
 * now. Every section that edits files (agents, memory, instructions) keeps one such draft in its
 * own store and gets the same behaviour from `draftActions`: ask before unsaved text is dropped,
 * save against the state the file was read in, and say so when the file changed on disk meanwhile.
 */

export interface DocOnDisk {
  /** The whole file, with plain line feeds. */
  text: string
  /** Stands for the bytes on disk. */
  hash: string
  /** Why the file cannot be edited here, when it cannot. */
  readOnly?: string
}

export interface Draft<D extends DocOnDisk> {
  detail: D
  text: string
}

export interface DraftState<D extends DocOnDisk> {
  draft: Draft<D> | null
  /** What is on disk by now, when it changed there while the draft has changes of its own. */
  onDisk: D | null
  saving: boolean
}

export const NO_DRAFT = { draft: null, onDisk: null, saving: false } as const

export const isDirty = (state: Pick<DraftState<DocOnDisk>, 'draft'>): boolean => state.draft !== null && state.draft.text !== state.draft.detail.text

export interface Question {
  title: string
  message: string
  confirmLabel: string
  danger?: boolean
}

export interface DraftHost<R, D extends R & DocOnDisk> {
  get(): DraftState<D>
  set(patch: Partial<DraftState<D>>): void
  /** What tells one file from another. */
  key(ref: R): string
  /** What the file is called in a question. */
  label(detail: D): string
  read(ref: R): Promise<D>
  /** `baseHash` null overwrites whatever is on disk. */
  write(ref: R, text: string, baseHash: string | null): Promise<{ saved: D } | { conflict: D }>
  /** Something about the text that is asked about before it is saved. */
  problem?(draft: Draft<D>): string | null
  /** A save went through. */
  saved?(detail: D): void
  ask(question: Question): Promise<boolean>
  /** A call to the main process failed. */
  fail(err: unknown): void
  warn(message: string): void
}

export interface DraftActions<R> {
  /** Shows a file in the editor. Asks first when the one that is open has unsaved changes. True when it is shown. */
  open(ref: R, sure?: boolean): Promise<boolean>
  /** Looks whether the open file changed on disk, as it does when a Claude session edits it. */
  checkDisk(): Promise<void>
  changeText(text: string): void
  /** Sets a field of the frontmatter; null, an empty text or an empty list takes it out. */
  changeField(key: string, value: FieldInput): void
  changeBody(body: string): void
  /** Saves the draft. `overwrite` writes it over a file that changed on disk in the meantime. */
  save(overwrite?: boolean): Promise<void>
  /** Takes the version on disk in place of the draft, after a change there. */
  loadDisk(): void
  revert(): Promise<void>
  /** Takes the file out of the editor without asking. */
  close(): void
}

export function draftActions<R, D extends R & DocOnDisk>(host: DraftHost<R, D>): DraftActions<R> {
  // one message for a value that cannot be written, not one per key press
  let lastRefusal = ''

  const read = async (ref: R): Promise<D | undefined> => {
    try {
      return await host.read(ref)
    } catch (err) {
      host.fail(err)
      return undefined
    }
  }

  const open = async (ref: R, sure = false): Promise<boolean> => {
    const { draft } = host.get()
    if (draft && host.key(draft.detail) === host.key(ref) && !sure) return true
    if (!sure && draft && isDirty(host.get())) {
      const ok = await host.ask({
        title: 'Unsaved changes',
        message: `${host.label(draft.detail)} has changes that are not saved. Opening something else discards them.`,
        confirmLabel: 'Discard',
        danger: true
      })
      if (!ok) return false
    }
    const detail = await read(ref)
    if (!detail) return false
    host.set({ draft: { detail, text: detail.text }, onDisk: null })
    return true
  }

  const checkDisk = async (): Promise<void> => {
    const { draft } = host.get()
    if (!draft) return
    let fresh: D
    try {
      fresh = await host.read(draft.detail)
    } catch {
      // gone or unreadable: the list says so when it is read again
      return
    }
    const now = host.get().draft
    if (!now || host.key(now.detail) !== host.key(fresh) || fresh.hash === now.detail.hash) return
    if (isDirty(host.get())) host.set({ onDisk: fresh })
    else host.set({ draft: { detail: fresh, text: fresh.text }, onDisk: null })
  }

  const changeText = (text: string): void => {
    const { draft } = host.get()
    if (draft && draft.text !== text) host.set({ draft: { ...draft, text } })
  }

  const changeField = (key: string, value: FieldInput): void => {
    const { draft } = host.get()
    if (!draft) return
    const result = setField(draft.text, key, value)
    if ('text' in result) {
      lastRefusal = ''
      changeText(result.text)
    } else if (result.failed !== lastRefusal) {
      lastRefusal = result.failed
      host.warn(result.failed)
    }
  }

  const changeBody = (body: string): void => {
    const { draft } = host.get()
    if (draft) changeText(setBody(draft.text, body))
  }

  const save = async (overwrite = false): Promise<void> => {
    const { draft, saving } = host.get()
    if (!draft || saving || draft.detail.readOnly || (!isDirty(host.get()) && !overwrite)) return
    const problem = host.problem?.(draft)
    if (problem) {
      const ok = await host.ask({ title: 'Save with a problem', message: `${problem}\n\nSave it like this anyway?`, confirmLabel: 'Save anyway' })
      if (!ok) return
    }
    host.set({ saving: true })
    let result: { saved: D } | { conflict: D }
    try {
      result = await host.write(draft.detail, draft.text, overwrite ? null : draft.detail.hash)
    } catch (err) {
      host.set({ saving: false })
      host.fail(err)
      return
    }
    if ('conflict' in result) {
      host.set({ saving: false, onDisk: result.conflict })
      return
    }
    // what was typed while the file was being written stays in the editor
    const typed = host.get().draft
    host.set({ saving: false, draft: { detail: result.saved, text: typed && typed.text !== draft.text ? typed.text : result.saved.text }, onDisk: null })
    host.saved?.(result.saved)
  }

  const loadDisk = (): void => {
    const { onDisk } = host.get()
    if (onDisk) host.set({ draft: { detail: onDisk, text: onDisk.text }, onDisk: null })
  }

  const revert = async (): Promise<void> => {
    const { draft } = host.get()
    if (!draft || !isDirty(host.get())) return
    const ok = await host.ask({ title: 'Revert', message: `The changes to ${host.label(draft.detail)} that are not saved are discarded.`, confirmLabel: 'Revert', danger: true })
    if (ok) await open(draft.detail, true)
  }

  const close = (): void => host.set({ draft: null, onDisk: null })

  return { open, checkDisk, changeText, changeField, changeBody, save, loadDisk, revert, close }
}
