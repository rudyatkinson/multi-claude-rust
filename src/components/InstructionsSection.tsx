import { useEffect, useState, type CSSProperties, type FormEvent } from 'react'
import { INSTRUCTION_LIMITS, findImports, fixedFiles, lineCount, ruleNameProblem, rulesDir, type InstructionFile, type InstructionScope, type LoadStep } from '@shared/instructions'
import type { ProfileView } from '@shared/types'
import { mc } from '../api'
import { splitPath } from '../format'
import {
  addInstructionFolder,
  changeText,
  checkDisk,
  chooseFile,
  createInstruction,
  deleteInstruction,
  editCopyAnyway,
  isDirty,
  isHeldCopy,
  loadDisk,
  openFile,
  openMainClaudeMd,
  refreshInstructions,
  revertInstruction,
  saveInstruction,
  selectScope,
  showLoadOrder,
  useInstructions
} from '../instructionsStore'
import { attempt, setModal, useStore } from '../store'
import { Dialog } from './Dialogs'
import { ConflictBanner, NameField, SaveBar } from './LibraryParts'

/** What a scope is called: the account's name, or the folder's. */
function scopeLabel(scope: InstructionScope, profiles: readonly ProfileView[]): { name: string; where: string } {
  if (scope.kind === 'account') return { name: profiles.find((p) => p.id === scope.profileId)?.name ?? 'Account', where: scope.dir }
  const { name, parent } = splitPath(scope.dir)
  return { name, where: parent }
}

/** A rule's file without the folder every rule of the scope lies in. */
const shortName = (file: string, scope: InstructionScope): string => (file.startsWith(`${rulesDir(scope.kind)}/`) ? file.slice(rulesDir(scope.kind).length + 1).replace(/\.md$/i, '') : file)

/**
 * The instructions editor: the accounts and the project folders on the left, then the CLAUDE.md
 * files and rules of the one that is chosen and the text of one of them, or what a session reads
 * there in which order.
 */
export function InstructionsSection() {
  const scopes = useInstructions((s) => s.scopes)
  const scopeId = useInstructions((s) => s.scopeId)
  const hasDraft = useInstructions((s) => s.draft !== null)
  const profiles = useStore((s) => s.profiles)

  useEffect(() => {
    void refreshInstructions().then(checkDisk)
  }, [])

  const scope = scopes?.find((s) => s.id === scopeId) ?? null
  const item = (s: InstructionScope) => {
    const label = scopeLabel(s, profiles)
    const profile = profiles.find((p) => p.id === s.profileId)
    return (
      <button key={s.id} className={`wf-item${s.id === scopeId ? ' selected' : ''}`} title={s.dir} onClick={() => void selectScope(s.id)}>
        <span className="sk-item-head">
          {profile && <span className="mem-dot" style={{ '--accent': profile.color } as CSSProperties} />}
          <span className="wf-item-name">{label.name}</span>
          {s.syncedClaudeMd && (
            <span className="sk-mark" title="Its CLAUDE.md is a copy that syncing from the main account writes over">
              synced
            </span>
          )}
        </span>
        <span className="wf-item-steps">{s.kind === 'account' ? 'Read in every project' : label.where}</span>
      </button>
    )
  }

  return (
    <div className="wf-body">
      <nav className="wf-list sk-list">
        {!scopes && <p className="note">Reading the folders…</p>}
        {scopes && (
          <>
            <section className="sk-root">
              <div className="sk-root-head">
                <span className="sk-root-name">Accounts</span>
              </div>
              {scopes.filter((s) => s.kind === 'account').map(item)}
            </section>
            <section className="sk-root">
              <div className="sk-root-head">
                <span className="sk-root-name">Projects</span>
              </div>
              {scopes.filter((s) => s.kind === 'project').map(item)}
              {!scopes.some((s) => s.kind === 'project') && <div className="sk-none">No project folder in sight yet</div>}
            </section>
          </>
        )}
        <div className="wf-list-actions">
          <button className="btn" title="Show the instruction files of a folder that is not in the list" onClick={() => void addInstructionFolder()}>
            Add project folder…
          </button>
        </div>
      </nav>

      {scope ? (
        <ScopeView scope={scope} />
      ) : hasDraft ? (
        <section className="wf-design sk-editor">
          <InstructionEditor />
        </section>
      ) : (
        <section className="wf-design wf-empty">
          <div className="empty-title">Choose an account or a project</div>
          <div className="muted">
            CLAUDE.md files and rules are what every session reads when it starts: an account's for every project it works in, a project's for everyone who works
            there.
          </div>
        </section>
      )}
    </div>
  )
}

function ScopeView({ scope }: { scope: InstructionScope }) {
  const files = useInstructions((s) => s.files)
  const current = useInstructions(openFile)
  const dirty = useInstructions(isDirty)
  const hasDraft = useInstructions((s) => s.draft !== null)
  const chosen = useInstructions((s) => s.chosen)
  const showOrder = useInstructions((s) => s.showOrder)
  const editCopy = useInstructions((s) => s.editCopy)
  const profiles = useStore((s) => s.profiles)
  const label = scopeLabel(scope, profiles)
  const about = (file: InstructionFile): string => {
    if (file.kind !== 'rule') return fixedFiles(scope.kind).find((f) => f.file === file.file)?.about ?? ''
    return file.paths.length > 0 ? `Only for ${file.paths.join(', ')}` : 'Read by every session'
  }
  const chosenFile = files?.find((f) => f.file === chosen) ?? null

  return (
    <section className="wf-design sk-editor mem-view">
      <div className="wf-toolbar">
        <div className="sk-title">
          <span className="sk-name">{label.name}</span>
          <span className="muted mem-where" title={scope.dir}>
            {scope.dir}
          </span>
        </div>
        <button className="btn" title="A rule is one topic in a file of its own; it can be limited to some files" onClick={() => setModal({ type: 'instructionRule', scopeId: scope.id })}>
          + New rule
        </button>
        <button className="btn" onClick={() => void attempt(mc.invoke('instructions:reveal', scope.id))}>
          Open folder
        </button>
      </div>

      <div className="mem-cols">
        <nav className="mem-entries">
          {scope.kind === 'project' && (
            <button className={`wf-item${showOrder ? ' selected' : ''}`} onClick={() => void showLoadOrder()}>
              <span className="sk-item-head">
                <span className="wf-item-name">What a session reads</span>
              </span>
              <span className="wf-item-steps">Every file, in the order Claude Code reads them here</span>
            </button>
          )}
          {!files && <p className="note">Reading…</p>}
          {files?.map((file) => (
            <button key={file.file} className={`wf-item${!showOrder && file.file === current ? ' selected' : ''}${file.exists ? '' : ' ins-absent'}`} title={file.file} onClick={() => void chooseFile(file.file)}>
              <span className="sk-item-head">
                <span className="wf-item-name">{shortName(file.file, scope)}</span>
                {!showOrder && file.file === current && dirty && <span className="sk-dot" title="Unsaved changes" />}
                {file.kind === 'rule' && <span className="sk-mark">rule</span>}
                {file.exists ? (
                  <span className={`sk-mark${file.kind !== 'rule' && file.lines > INSTRUCTION_LIMITS.lines ? ' over' : ''}`} title={file.kind !== 'rule' && file.lines > INSTRUCTION_LIMITS.lines ? `Anthropic advises to stay under ${INSTRUCTION_LIMITS.lines} lines` : undefined}>
                    {file.lines} {file.lines === 1 ? 'line' : 'lines'}
                  </span>
                ) : (
                  <span className="sk-mark">not there</span>
                )}
                {isHeldCopy(scope, file.file, editCopy) && <span className="sk-mark">copy</span>}
              </span>
              <span className="wf-item-steps">{about(file)}</span>
            </button>
          ))}
        </nav>
        <div className="mem-editor">
          {showOrder ? (
            <LoadOrder scope={scope} />
          ) : hasDraft ? (
            <InstructionEditor />
          ) : chosenFile && isHeldCopy(scope, chosenFile.file, editCopy) ? (
            <HeldCopy scope={scope} />
          ) : chosenFile && !chosenFile.exists ? (
            <div className="ins-offer">
              <div className="empty-title">{chosenFile.file} is not there yet</div>
              <div className="muted">{about(chosenFile)}.</div>
              <button className="btn primary" onClick={() => void createInstruction(scope.id, { file: chosenFile.file })}>
                Create {chosenFile.file}
              </button>
            </div>
          ) : (
            <p className="note">Choose a file to read or change it.</p>
          )}
        </div>
      </div>
    </section>
  )
}

/** An account's CLAUDE.md that is a copy of the main account's: where to change it instead. */
function HeldCopy({ scope }: { scope: InstructionScope }) {
  const profiles = useStore((s) => s.profiles)
  const main = profiles.find((p) => p.isDefault)?.name ?? 'Main'
  const name = profiles.find((p) => p.id === scope.profileId)?.name ?? 'This account'
  return (
    <div className="ins-offer">
      <div className="empty-title">A copy of {main}'s CLAUDE.md</div>
      <div className="muted">
        {name} gets this file from {main} when it is synced, and a sync writes over what was changed here. Change {main}'s file and sync {name}, or edit the copy if{' '}
        {name} should differ from now on.
      </div>
      <button className="btn primary" onClick={() => void openMainClaudeMd()}>
        Open {main}'s CLAUDE.md
      </button>
      <button className="btn" onClick={() => void editCopyAnyway(scope.id)}>
        Edit this copy anyway
      </button>
    </div>
  )
}

/** A path as short as it gets without losing where the file is. */
function shortPath(file: string, scope: InstructionScope, homeDir: string): string {
  const under = (base: string): string | null => (file.toLowerCase().startsWith(`${base.toLowerCase()}\\`) || file.toLowerCase().startsWith(`${base.toLowerCase()}/`) ? file.slice(base.length + 1) : null)
  const inProject = under(scope.dir)
  if (inProject !== null) return inProject
  const inHome = homeDir ? under(homeDir) : null
  return inHome !== null ? `~\\${inHome}` : file
}

function Steps({ steps, scope }: { steps: readonly LoadStep[]; scope: InstructionScope }) {
  const homeDir = useStore((s) => s.homeDir)
  return (
    <>
      {steps.map((step, index) => (
        <div key={`${index}:${step.path}`} className={`ins-step${step.when === 'missing' ? ' missing' : ''}`} style={{ paddingLeft: 8 + step.depth * 18 }} title={step.path}>
          <span className="sk-mark">{step.depth > 0 ? '↳ import' : step.role.toLowerCase()}</span>
          <span className="ins-step-path">{shortPath(step.path, scope, homeDir)}</span>
          {step.when !== 'missing' && (
            <span className="muted">
              {step.lines} {step.lines === 1 ? 'line' : 'lines'}
            </span>
          )}
          {step.note && <span className={`ins-step-note${step.when === 'demand' ? '' : ' warn'}`}>{step.note}</span>}
        </div>
      ))}
    </>
  )
}

/** What a session of one account reads in the project folder, in order. */
function LoadOrder({ scope }: { scope: InstructionScope }) {
  const order = useInstructions((s) => s.order)
  const profileId = useInstructions((s) => s.orderProfileId)
  const profiles = useStore((s) => s.profiles)
  const atStart = (order ?? []).filter((s) => s.when !== 'demand')
  const onDemand = (order ?? []).filter((s) => s.when === 'demand')
  const read = atStart.filter((s) => s.when === 'start')
  const total = read.reduce((sum, s) => sum + s.lines, 0)

  return (
    <div className="ins-order">
      <div className="ins-order-head">
        <span>A session of</span>
        <div className="chips">
          {profiles.map((p) => (
            <button key={p.id} type="button" className={`chip${p.id === profileId ? ' selected' : ''}`} style={{ '--accent': p.color } as CSSProperties} onClick={() => void showLoadOrder(p.id)}>
              <span className="dot" />
              {p.name}
            </button>
          ))}
        </div>
        <span>that starts in this folder reads</span>
      </div>
      {!order && <p className="note">Reading…</p>}
      {order && (
        <>
          <div className="ins-group">
            When it starts: {read.length} {read.length === 1 ? 'file' : 'files'}, {total} lines
          </div>
          {atStart.length === 0 && <p className="note">Nothing. There is no CLAUDE.md for this account or on the way down to this folder.</p>}
          <Steps steps={atStart} scope={scope} />
          {onDemand.length > 0 && (
            <>
              <div className="ins-group">Later, when a matching file comes up</div>
              <Steps steps={onDemand} scope={scope} />
            </>
          )}
          <p className="note ins-caveat">
            This is worked out from the files on disk, as Claude Code documents its reading. What a session really read is shown by /context inside it. Files in folders below
            this one are read when Claude first works there.
          </p>
        </>
      )}
    </div>
  )
}

/** The text of one instruction file, as it is written. */
function InstructionEditor() {
  const draft = useInstructions((s) => s.draft)!
  const onDisk = useInstructions((s) => s.onDisk)
  const saving = useInstructions((s) => s.saving)
  const dirty = useInstructions(isDirty)
  const { detail, text } = draft
  const lines = lineCount(text)
  const imports = findImports(text)
  const long = !detail.file.includes('rules/') && lines > INSTRUCTION_LIMITS.lines

  return (
    <>
      {onDisk && <ConflictBanner onLoad={loadDisk} onOverwrite={() => void saveInstruction(true)} />}
      {detail.readOnly !== undefined ? (
        <p className="note warn sk-readonly">{detail.readOnly} Show the file to work on it with another program.</p>
      ) : (
        <>
          <SaveBar dirty={dirty} saving={saving} saveTitle="Ctrl+S. Sessions read the new text when they start." onRevert={() => void revertInstruction()} onSave={() => void saveInstruction()}>
            <span className="mem-file" title={detail.path}>
              {detail.file}
            </span>
            <span className={`muted${long ? ' over' : ''}`} title={long ? `Anthropic advises to stay under ${INSTRUCTION_LIMITS.lines} lines; long files are followed less closely` : undefined}>
              {lines} lines
            </span>
            <button className="link-btn" onClick={() => void attempt(mc.invoke('instructions:reveal', detail.scopeId, detail.file))}>
              Show file
            </button>
            <button className="link-btn" onClick={() => void deleteInstruction(detail)}>
              Delete
            </button>
          </SaveBar>
          <textarea className="sk-source" value={text} spellCheck={false} onChange={(e) => changeText(e.target.value)} />
          {imports.length > 0 && (
            <p className="note ins-imports" title="A line with @path pulls that file in, up to four files deep">
              Imports: {imports.map((target) => `@${target}`).join(', ')}
            </p>
          )}
        </>
      )}
    </>
  )
}

const close = (): void => setModal(null)

/** Makes a rule: one topic in a file of its own. */
export function NewRuleDialog({ scopeId }: { scopeId: string }) {
  const scope = useInstructions((s) => s.scopes?.find((x) => x.id === scopeId))
  const files = useInstructions((s) => s.files)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const dir = rulesDir(scope?.kind ?? 'project')
  const taken = (files ?? []).some((f) => f.file.toLowerCase() === `${dir}/${name}.md`)
  const problem = name === '' ? null : (ruleNameProblem(name) ?? (taken ? `There is already a rule named "${name}" there.` : null))
  const ready = name !== '' && problem === null && !busy

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = await createInstruction(scopeId, { rule: name })
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title="New rule" onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <NameField value={name} onChange={setName} problem={problem} max={INSTRUCTION_LIMITS.ruleName} note="lower-case letters, digits and hyphens" placeholder="code-style" />
        <p className="note">
          The file is made as {dir}/{name || 'name'}.md. Every session reads it, unless a "paths" list in its frontmatter limits it to some files.
        </p>
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
