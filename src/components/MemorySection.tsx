import { useEffect, useState, type CSSProperties, type FormEvent } from 'react'
import { MEMORY_INDEX, MEMORY_LIMITS, MEMORY_TYPES, memoryKey, memoryNameProblem, type AdoptResult, type MemoryFolder, type MemoryProject, type MemoryType } from '@shared/memory'
import { slugifySkillName } from '@shared/skills'
import type { ProfileView } from '@shared/types'
import { mc } from '../api'
import { ago } from '../format'
import {
  adoptMemories,
  changeText,
  checkDisk,
  createMemory,
  deleteMemory,
  isDirty,
  loadDisk,
  openHit,
  openKey,
  openMemory,
  planAdoption,
  projectLabel,
  refreshMemory,
  revertMemory,
  saveMemory,
  search,
  selectFolder,
  selectProject,
  setSharing,
  useMemory
} from '../memoryStore'
import { attempt, setModal, useStore } from '../store'
import { Dialog } from './Dialogs'
import { ConflictBanner, Issues, NameField, SaveBar } from './LibraryParts'

/** The accounts as small dots in their colours. */
function AccountDots({ ids, profiles, shared }: { ids: readonly string[]; profiles: readonly ProfileView[]; shared?: boolean }) {
  return (
    <>
      {ids.map((id) => {
        const profile = profiles.find((p) => p.id === id)
        if (!profile) return null
        return <span key={id} className={`mem-dot${shared ? ' shared' : ''}`} style={{ '--accent': profile.color } as CSSProperties} title={shared ? `${profile.name} uses this memory as well` : profile.name} />
      })}
    </>
  )
}

const namesOf = (ids: readonly string[], profiles: readonly ProfileView[]): string =>
  ids
    .map((id) => profiles.find((p) => p.id === id)?.name)
    .filter(Boolean)
    .join(', ')

/**
 * The memory browser: the projects Claude wrote something down about on the left, then the
 * memories of the one that is chosen and the text of one of them.
 */
export function MemorySection() {
  const listing = useMemory((s) => s.listing)
  const projectKey = useMemory((s) => s.projectKey)
  const query = useMemory((s) => s.query)
  const hits = useMemory((s) => s.hits)
  const open = useMemory(openKey)
  const hasDraft = useMemory((s) => s.draft !== null)
  const profiles = useStore((s) => s.profiles)
  const now = useStore((s) => s.now)

  useEffect(() => {
    void refreshMemory().then(checkDisk)
  }, [])

  const project = listing?.projects.find((p) => p.key === projectKey) ?? null

  return (
    <div className="wf-body">
      <nav className="wf-list sk-list">
        <input className="sk-search" autoFocus value={query} placeholder="Search every memory" spellCheck={false} onChange={(e) => void search(e.target.value)} />
        {!listing && <p className="note">Reading the memories…</p>}
        {hits !== null ? (
          <>
            {hits.length === 0 && <p className="note">Nothing matches.</p>}
            {hits.map((hit) => (
              <button key={memoryKey(hit)} className={`wf-item mem-hit${memoryKey(hit) === open ? ' selected' : ''}`} onClick={() => void openHit(hit)}>
                <span className="sk-item-head">
                  <span className="wf-item-name">{hit.title}</span>
                  <AccountDots ids={hit.profileIds} profiles={profiles} />
                </span>
                <span className="wf-item-steps">{projectLabel(hit).name}</span>
                <span className="mem-snippet">{hit.snippet}</span>
              </button>
            ))}
            {hits.length >= MEMORY_LIMITS.hits && <p className="note">Only the first {MEMORY_LIMITS.hits} are shown.</p>}
          </>
        ) : (
          listing?.projects.map((p) => {
            const label = projectLabel(p)
            const count = Math.max(...p.folders.map((f) => f.count))
            return (
              <button key={p.key} className={`wf-item${p.key === projectKey ? ' selected' : ''}`} title={p.cwd ?? p.key} onClick={() => void selectProject(p.key)}>
                <span className="sk-item-head">
                  <span className="wf-item-name">{label.name}</span>
                  <span className="sk-mark" title="Memories">
                    {count}
                  </span>
                  {p.folders.map((f) => (
                    <span key={f.id} className="mem-dots">
                      <AccountDots ids={f.profileIds} profiles={profiles} />
                      <AccountDots ids={f.sharedWith} profiles={profiles} shared />
                    </span>
                  ))}
                </span>
                <span className="wf-item-steps">
                  {ago(now - Math.max(...p.folders.map((f) => f.updatedAt)))} · {label.where}
                </span>
              </button>
            )
          })
        )}
        {listing && listing.projects.length === 0 && <div className="sk-none">No project has a memory yet</div>}
        <Sharing />
      </nav>

      {project ? (
        <ProjectView project={project} />
      ) : hasDraft ? (
        <section className="wf-design sk-editor">
          <MemoryEditor />
        </section>
      ) : (
        <section className="wf-design wf-empty">
          <div className="empty-title">{listing && listing.projects.length === 0 ? 'No memories yet' : 'Choose a project'}</div>
          <div className="muted">
            Claude Code writes down by itself what it learns about a project and about how you work: one file per thing, and MEMORY.md as the index every session
            reads when it starts. Each account keeps its own memory of each project.
          </div>
        </section>
      )}
    </div>
  )
}

/** The switches that let the other accounts use the main account's memory. */
function Sharing() {
  const profiles = useStore((s) => s.profiles)
  const problem = useMemory((s) => s.listing?.shareProblem ?? null)
  const main = profiles.find((p) => p.isDefault)
  const others = profiles.filter((p) => !p.isDefault)
  if (!main || others.length === 0) return null
  return (
    <div className="mem-sharing">
      <div className="sk-root-head">
        <span className="sk-root-name">Shared memory</span>
      </div>
      {others.map((p) => (
        <label key={p.id} className="check" title={`Sessions this app starts for ${p.name} read and write ${main.name}'s memory of the project they run in, in place of ${p.name}'s own.`}>
          <input type="checkbox" checked={p.shareMemory === true} onChange={(e) => void setSharing(p.id, e.target.checked)} />
          {p.name} uses {main.name}'s memory
        </label>
      ))}
      <p className="note">Nothing is moved on disk, and switching it off brings an account's own memory back. It goes for sessions that start from now on.</p>
      {problem && <p className="note warn">Sharing is off for now. {problem}</p>}
    </div>
  )
}

/** Whose memory this is, for the switch between the accounts of one project. */
function folderLabel(folder: MemoryFolder, profiles: readonly ProfileView[]): string {
  return namesOf(folder.profileIds, profiles) || 'Unknown account'
}

function ProjectView({ project }: { project: MemoryProject }) {
  const folderId = useMemory((s) => s.folderId)
  const view = useMemory((s) => s.view)
  const open = useMemory(openKey)
  const dirty = useMemory(isDirty)
  const hasDraft = useMemory((s) => s.draft !== null)
  const profiles = useStore((s) => s.profiles)
  const label = projectLabel(project)
  const folder = project.folders.find((f) => f.id === folderId) ?? null
  const main = profiles.find((p) => p.isDefault)
  // a folder of another account can be brought into the main account's
  const own = folder !== null && main !== undefined && !folder.profileIds.includes(main.id)
  const sharing = folder ? profiles.filter((p) => folder.profileIds.includes(p.id) && p.shareMemory === true) : []

  return (
    <section className="wf-design sk-editor mem-view">
      <div className="wf-toolbar">
        <div className="sk-title">
          <span className="sk-name">{label.name}</span>
          <span className="muted mem-where" title={project.cwd ?? project.key}>
            {project.cwd ?? label.where}
          </span>
        </div>
        {project.folders.length > 1 && (
          <div className="segmented" title="Each of these accounts has a memory of its own for this project">
            {project.folders.map((f) => (
              <button key={f.id} type="button" className={f.id === folderId ? 'selected' : ''} onClick={() => void selectFolder(project.key, f.id)}>
                {folderLabel(f, profiles)}
              </button>
            ))}
          </div>
        )}
        {own && (
          <button className="btn" title={`Copy these memories into ${main.name}'s memory of this project. Nothing is removed here.`} onClick={() => void planAdoption(folder.id)}>
            Bring into {main.name}'s memory…
          </button>
        )}
        <button className="btn" disabled={!folder} title="Write something down for Claude to remember in this project" onClick={() => folder && setModal({ type: 'memoryNew', folderId: folder.id })}>
          + New memory
        </button>
        <button className="btn" disabled={!folder} onClick={() => folder && void attempt(mc.invoke('memory:reveal', folder.id))}>
          Open folder
        </button>
      </div>
      {folder && (
        <div className="sk-where">
          <span>
            {folderLabel(folder, profiles)}
            {folder.sharedWith.length > 0 && `, shared with ${namesOf(folder.sharedWith, profiles)}`}
          </span>
          <span className="sk-path" title={folder.dir}>
            {folder.dir}
          </span>
        </div>
      )}
      {((view && view.issues.length > 0) || (own && sharing.length > 0)) && (
        <div className="mem-issues">
          {own && sharing.length > 0 && (
            <p className="note warn">
              {namesOf(
                sharing.map((p) => p.id),
                profiles
              )}{' '}
              uses {main.name}'s memory now, so new sessions do not read the files here. Bring them into {main.name}'s memory to keep them in use.
            </p>
          )}
          <Issues issues={view?.issues ?? []} />
        </div>
      )}

      <div className="mem-cols">
        <nav className="mem-entries">
          {!view && <p className="note">Reading…</p>}
          {view?.entries.map((entry) => {
            const key = memoryKey({ folderId: view.folderId, file: entry.file })
            return (
              <button key={key} className={`wf-item${key === open ? ' selected' : ''}`} title={entry.file} onClick={() => void openMemory({ folderId: view.folderId, file: entry.file })}>
                <span className="sk-item-head">
                  <span className="wf-item-name">{entry.title}</span>
                  {key === open && dirty && <span className="sk-dot" title="Unsaved changes" />}
                  {entry.type && <span className="sk-mark">{entry.type}</span>}
                  {!entry.indexed && (
                    <span className="sk-mark" title="MEMORY.md has no line for this file, so a session does not know of it">
                      not listed
                    </span>
                  )}
                </span>
                <span className="wf-item-steps">{entry.description || 'No description'}</span>
              </button>
            )
          })}
        </nav>
        <div className="mem-editor">{hasDraft ? <MemoryEditor /> : <p className="note">Choose a memory to read or change it.</p>}</div>
      </div>
    </section>
  )
}

/** The text of one memory file, as it is written. */
function MemoryEditor() {
  const draft = useMemory((s) => s.draft)!
  const onDisk = useMemory((s) => s.onDisk)
  const saving = useMemory((s) => s.saving)
  const dirty = useMemory(isDirty)
  const { detail, text } = draft
  const index = detail.file === MEMORY_INDEX
  const lines = text === '' ? 0 : text.replace(/\n$/, '').split('\n').length

  return (
    <>
      {onDisk && <ConflictBanner onLoad={loadDisk} onOverwrite={() => void saveMemory(true)} />}
      {detail.readOnly !== undefined ? (
        <p className="note warn sk-readonly">{detail.readOnly} Show the file to work on it with another program.</p>
      ) : (
        <>
          <SaveBar dirty={dirty} saving={saving} saveTitle="Ctrl+S. Sessions read the new text when they start." onRevert={() => void revertMemory()} onSave={() => void saveMemory()}>
            <span className="mem-file" title={detail.path}>
              {detail.file}
            </span>
            <span className={`muted${index && lines > MEMORY_LIMITS.indexLines ? ' over' : ''}`}>{index ? `${lines} of the ${MEMORY_LIMITS.indexLines} lines a session reads` : `${lines} lines`}</span>
            <button className="link-btn" onClick={() => void attempt(mc.invoke('memory:reveal', detail.folderId, detail.file))}>
              Show file
            </button>
            {!index && (
              <button className="link-btn" onClick={() => void deleteMemory(detail)}>
                Delete
              </button>
            )}
          </SaveBar>
          <textarea className="sk-source" value={text} spellCheck={false} onChange={(e) => changeText(e.target.value)} />
        </>
      )}
    </>
  )
}

const close = (): void => setModal(null)

/** Shows what bringing an account's memories into the main account's folder would do, and does it on a yes. */
export function AdoptMemoryDialog({ folderId, plan }: { folderId: string; plan: AdoptResult }) {
  const main = useStore((s) => s.profiles.find((p) => p.isDefault)?.name ?? 'Main')
  const [busy, setBusy] = useState(false)
  const renamed = plan.copy.filter((c) => c.to !== c.from).length

  const submit = async (): Promise<void> => {
    setBusy(true)
    const done = await adoptMemories(folderId)
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title={`Bring into ${main}'s memory`} onClose={close}>
      {plan.copy.length === 0 ? (
        <p className="confirm-text">Everything here is in {main}'s memory of this project already.</p>
      ) : (
        <div className="field">
          <span>
            {plan.copy.length} {plan.copy.length === 1 ? 'file is' : 'files are'} copied to <em>{plan.target}</em>
          </span>
          <div className="sk-import-files">
            {plan.copy.map((c) => (
              <span key={c.to}>{c.to === c.from ? c.from : `${c.from} → ${c.to}`}</span>
            ))}
          </div>
        </div>
      )}
      {renamed > 0 && (
        <p className="note">
          {renamed === 1 ? 'One has' : `${renamed} have`} the name of a memory that is there with another text. {renamed === 1 ? 'It arrives' : 'They arrive'} under a name of{' '}
          {renamed === 1 ? 'its' : 'their'} own, and both stay.
        </p>
      )}
      {plan.same.length > 0 && (
        <p className="note">
          {plan.same.length} {plan.same.length === 1 ? 'is' : 'are'} there already with the same text.
        </p>
      )}
      <p className="note">
        Nothing is removed from this account's own folder. Of {main}'s files only MEMORY.md changes: it gets a line for each file that arrives.
      </p>
      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          {plan.copy.length === 0 ? 'Close' : 'Cancel'}
        </button>
        {plan.copy.length > 0 && (
          <button className="btn primary" disabled={busy} onClick={() => void submit()}>
            Bring in
          </button>
        )}
      </div>
    </Dialog>
  )
}

/** Writes something down by hand: a memory file with its line in the index. */
export function NewMemoryDialog({ folderId }: { folderId: string }) {
  const view = useMemory((s) => s.view)
  const taken = view?.folderId === folderId ? view.entries.map((e) => e.file.toLowerCase()) : []
  const [title, setTitle] = useState('')
  const [name, setName] = useState('')
  // the name follows the title until it is typed itself
  const [named, setNamed] = useState(false)
  const [type, setType] = useState<MemoryType>('project')
  const [description, setDescription] = useState('')
  const [busy, setBusy] = useState(false)
  const problem = name === '' ? null : (memoryNameProblem(name) ?? (taken.includes(`${name}.md`) ? `There is already a memory named "${name}.md" there.` : null))
  const ready = name !== '' && problem === null && !busy

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = await createMemory(folderId, { name, title, description, type })
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title="New memory" onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <label className="field">
          <span>
            Title <em>(how the index calls it)</em>
          </span>
          <input
            autoFocus
            value={title}
            maxLength={MEMORY_LIMITS.title}
            placeholder="Release steps"
            onChange={(e) => {
              setTitle(e.target.value)
              if (!named) setName(slugifySkillName(e.target.value))
            }}
          />
        </label>
        <NameField
          value={name}
          onChange={(next) => {
            setName(next)
            setNamed(true)
          }}
          problem={problem}
          max={MEMORY_LIMITS.name}
          label="File name"
          focus={false}
          note="lower-case letters, digits and hyphens"
          placeholder="release-steps"
        />
        <div className="field" title="user: about you. feedback: how you want Claude to work. project: about this project. reference: where something is found.">
          <span>Kind</span>
          <div className="segmented">
            {MEMORY_TYPES.map((t) => (
              <button key={t} type="button" className={t === type ? 'selected' : ''} onClick={() => setType(t)}>
                {t}
              </button>
            ))}
          </div>
        </div>
        <label className="field">
          <span>
            Description <em>(one line: what it is, so that Claude knows when to read it)</em>
          </span>
          <input value={description} maxLength={MEMORY_LIMITS.description} placeholder="how a release is cut: tag, build, install" onChange={(e) => setDescription(e.target.value)} />
        </label>
        <p className="note">The file is made with this line in MEMORY.md; write the memory itself in the editor afterwards.</p>
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!ready}>
            Create
          </button>
        </div>
      </form>
    </Dialog>
  )
}
