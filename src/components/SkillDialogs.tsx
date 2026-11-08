import { useState, type CSSProperties, type FormEvent } from 'react'
import { SKILL_LIMITS, freeSkillName, newNameProblem, slugifySkillName, type ImportPreview, type SkillRef } from '@shared/skills'
import { addSkillFolder, askClaude, createSkill, duplicateSkill, importSkill, renameSkill, transferSkill, useSkills } from '../skillsStore'
import { setModal, useStore } from '../store'
import { Dialog } from './Dialogs'
import { NameField, RootChoice, asName } from './LibraryParts'

const close = (): void => setModal(null)

const NAME_NOTE = 'lower-case letters, digits and hyphens; it is what you type after /'

const problemOf = (name: string, taken: readonly string[]): string | null =>
  name === '' ? null : (newNameProblem(name) ?? (taken.some((n) => n.toLowerCase() === name) ? `There is already a skill named "${name}" there.` : null))

function SkillName({ value, onChange, taken, label }: { value: string; onChange(name: string): void; taken: readonly string[]; label?: string }) {
  return <NameField value={value} onChange={onChange} problem={problemOf(value, taken)} max={SKILL_LIMITS.name} label={label} note={NAME_NOTE} placeholder="review-pr" />
}

/** The names skills of a folder answer to; `except` is the folder of the skill that is being renamed. */
const namesIn = (rootId: string | null, except?: string): string[] =>
  (useSkills.getState().listing?.skills ?? []).filter((s) => s.rootId === rootId && s.dirName !== except).flatMap((s) => [s.dirName, s.name])

/** Makes a skill: a name, where it lives, and a first description. */
export function NewSkillDialog({ rootId }: { rootId?: string }) {
  const roots = useSkills((s) => s.listing?.roots ?? [])
  const [name, setName] = useState('')
  const [where, setWhere] = useState<string | null>(rootId ?? roots[0]?.id ?? null)
  const [description, setDescription] = useState('')
  const [busy, setBusy] = useState(false)
  const taken = namesIn(where)
  const ready = where !== null && newNameProblem(name) === null && !taken.some((n) => n.toLowerCase() === name) && !busy

  const create = async (thenAsk: boolean): Promise<void> => {
    if (!ready) return
    setBusy(true)
    const ref = await createSkill(where, name, description)
    setBusy(false)
    if (!ref) return
    if (thenAsk) setModal({ type: 'skillAssist', ref })
    else close()
  }

  return (
    <Dialog title="New skill" onClose={close}>
      <form
        onSubmit={(e: FormEvent) => {
          e.preventDefault()
          void create(false)
        }}
      >
        <SkillName value={name} onChange={setName} taken={taken} />
        <RootChoice roots={roots} value={where} onChange={setWhere} onAddFolder={addSkillFolder} />
        <label className="field">
          <span>
            Description <em>(what it does and when to use it; you can write it later)</em>
          </span>
          <textarea className="sk-long" rows={3} value={description} maxLength={2000} spellCheck={false} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="button" className="btn" disabled={!ready} title="Make the skill, then tell Claude what it should do. It spends messages." onClick={() => void create(true)}>
            Create and ask Claude…
          </button>
          <button type="submit" className="btn primary" disabled={!ready}>
            Create
          </button>
        </div>
      </form>
    </Dialog>
  )
}

/** A new name for a skill, or the name of its copy. */
export function SkillNameDialog({ mode, target }: { mode: 'rename' | 'duplicate'; target: SkillRef }) {
  const taken = namesIn(target.rootId, mode === 'rename' ? target.dirName : undefined)
  const base = slugifySkillName(target.dirName) || 'skill'
  const [name, setName] = useState(() => (mode === 'rename' ? asName(target.dirName, SKILL_LIMITS.name) : freeSkillName(`${base.slice(0, SKILL_LIMITS.name - 5)}-copy`, taken)))
  const [busy, setBusy] = useState(false)
  const same = mode === 'rename' && name === target.dirName
  const ready = newNameProblem(name) === null && !taken.some((n) => n.toLowerCase() === name) && !same && !busy

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = mode === 'rename' ? await renameSkill(target, name) : await duplicateSkill(target, name)
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title={mode === 'rename' ? `Rename "${target.dirName}"` : `Duplicate "${target.dirName}"`} onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <SkillName value={name} onChange={setName} taken={taken} label={mode === 'rename' ? 'New name' : 'Name of the copy'} />
        {mode === 'rename' && <p className="note">The folder gets this name, and so does the name inside SKILL.md when it was the folder name. A session that is working in the folder can make Windows refuse.</p>}
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!ready}>
            {mode === 'rename' ? 'Rename' : 'Duplicate'}
          </button>
        </div>
      </form>
    </Dialog>
  )
}

/** Copies or moves a skill between the personal skills and a project. */
export function SkillTransferDialog({ target }: { target: SkillRef }) {
  const listing = useSkills((s) => s.listing)
  const roots = (listing?.roots ?? []).filter((r) => r.id !== target.rootId)
  const linked = listing?.skills.find((s) => s.rootId === target.rootId && s.dirName === target.dirName)?.linked === true
  const [where, setWhere] = useState<string | null>(roots[0]?.id ?? null)
  const [mode, setMode] = useState<'copy' | 'move'>('copy')
  const [name, setName] = useState(() => (newNameProblem(target.dirName) === null ? target.dirName : slugifySkillName(target.dirName) || 'skill'))
  const [busy, setBusy] = useState(false)
  const taken = namesIn(where)
  const ready = where !== null && newNameProblem(name) === null && !taken.some((n) => n.toLowerCase() === name) && !busy
  const to = listing?.roots.find((r) => r.id === where)

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = await transferSkill(target, where, mode, name)
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title={`Copy or move "${target.dirName}"`} onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <RootChoice roots={roots} value={where} onChange={setWhere} onAddFolder={addSkillFolder} />
        <div className="field">
          <span>What happens to this one</span>
          <div className="segmented">
            <button type="button" className={mode === 'copy' ? 'selected' : ''} onClick={() => setMode('copy')}>
              Copy: it stays here
            </button>
            <button type="button" className={mode === 'move' ? 'selected' : ''} disabled={linked} title={linked ? 'A link to a folder elsewhere cannot be moved' : 'It is copied there, then this one goes to the bin'} onClick={() => setMode('move')}>
              Move: it goes to the bin here
            </button>
          </div>
        </div>
        <SkillName value={name} onChange={setName} taken={taken} label="Name there" />
        {to?.kind === 'personal' && <p className="note">A personal skill of the same name is used instead of a project's.</p>}
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!ready}>
            {mode === 'copy' ? 'Copy' : 'Move'}
          </button>
        </div>
      </form>
    </Dialog>
  )
}

/** Shows what an archive holds and asks where it goes. */
export function SkillImportDialog({ preview }: { preview: ImportPreview }) {
  const roots = useSkills((s) => s.listing?.roots ?? [])
  const [where, setWhere] = useState<string | null>(roots[0]?.id ?? null)
  const [name, setName] = useState(preview.name)
  const [busy, setBusy] = useState(false)
  const taken = namesIn(where)
  const ready = where !== null && newNameProblem(name) === null && !taken.some((n) => n.toLowerCase() === name) && !busy
  const scripts = preview.files.filter((f) => /\.(py|sh|bat|cmd|ps1|js|mjs|ts|exe|rb|pl)$/i.test(f.path)).length

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = await importSkill(preview.token, where, name)
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title="Import skill" onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        {preview.description && <p className="sk-import-text">{preview.description}</p>}
        {preview.issues.map((issue) => (
          <p key={issue.message} className={`note ${issue.level === 'error' ? 'bad' : 'warn'}`}>
            {issue.message}
          </p>
        ))}
        <div className="field">
          <span>
            It brings SKILL.md{preview.files.length > 0 ? ` and ${preview.files.length}${preview.moreFiles ? '+' : ''} more ${preview.files.length === 1 ? 'file' : 'files'}` : ''}
          </span>
          {preview.files.length > 0 && (
            <div className="sk-import-files">
              {preview.files.map((file) => (
                <span key={file.path}>{file.path}</span>
              ))}
            </div>
          )}
        </div>
        {scripts > 0 && <p className="note warn">It contains {scripts === 1 ? 'a script' : `${scripts} scripts`}. This app never runs them, but Claude may when it uses the skill. Bring in only what you trust.</p>}
        <SkillName value={name} onChange={setName} taken={taken} />
        <RootChoice roots={roots} value={where} onChange={setWhere} onAddFolder={addSkillFolder} />
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!ready}>
            Import
          </button>
        </div>
      </form>
    </Dialog>
  )
}

/** Asks an account's Claude to write or improve a skill, in a session tab of its own. */
export function SkillAssistDialog({ target }: { target: SkillRef }) {
  const profiles = useStore((s) => s.profiles)
  const root = useSkills((s) => s.listing?.roots.find((r) => r.id === target.rootId))
  // a personal skill is only there for the accounts that share its folder
  const accounts = profiles.filter((p) => p.account?.loggedIn && (root?.kind !== 'personal' || root.profileIds.includes(p.id)))
  const [profileId, setProfileId] = useState<string | null>(() => (accounts.find((p) => p.isDefault) ?? accounts[0])?.id ?? null)
  const [request, setRequest] = useState('')
  const [busy, setBusy] = useState(false)
  const ready = profileId !== null && request.trim() !== '' && !busy

  const submit = async (): Promise<void> => {
    if (!ready) return
    setBusy(true)
    // the dialog closes itself when the tab has opened
    if (!(await askClaude(target, profileId, request))) setBusy(false)
  }

  return (
    <Dialog title={`Ask Claude about "${target.dirName}"`} onClose={close}>
      <div className="field">
        <span>Account</span>
        <div className="chips">
          {accounts.map((p) => (
            <button key={p.id} type="button" className={`chip${p.id === profileId ? ' selected' : ''}`} style={{ '--accent': p.color } as CSSProperties} title={p.account?.email} onClick={() => setProfileId(p.id)}>
              <span className="dot" />
              {p.name}
            </button>
          ))}
        </div>
        {accounts.length === 0 && <span className="note warn">No account that sees this skill is signed in.</span>}
      </div>
      <label className="field">
        <span>What should Claude do?</span>
        <textarea
          className="run-task"
          autoFocus
          value={request}
          maxLength={SKILL_LIMITS.request}
          placeholder="Write the instructions for reviewing a pull request: … / Make the description say more clearly when to use it."
          onChange={(e) => setRequest(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) void submit()
          }}
        />
      </label>
      <p className="note">
        A session tab opens in {root?.kind === 'project' ? 'the project folder' : 'the skills folder'} with the account's usual model and works on the skill's files. It spends
        messages of that account. When it is done, come back here to look at the result.
      </p>
      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          Cancel
        </button>
        <button className="btn primary" disabled={!ready} title="Ctrl+Enter" onClick={() => void submit()}>
          {busy ? 'Opening…' : 'Ask Claude'}
        </button>
      </div>
    </Dialog>
  )
}
