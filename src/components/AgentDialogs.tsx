import { useState, type FormEvent } from 'react'
import { AGENT_LIMITS, agentKey, agentNameProblem, type AgentRef } from '@shared/agents'
import { freeSkillName, slugifySkillName } from '@shared/skills'
import { addAgentFolder, agentLabel, createAgent, duplicateAgent, renameAgent, transferAgent, useAgents } from '../agentsStore'
import { setModal } from '../store'
import { Dialog } from './Dialogs'
import { NameField, RootChoice, asName } from './LibraryParts'

const close = (): void => setModal(null)

const NAME_NOTE = 'lower-case letters, digits and hyphens; Claude knows the agent by it'

/** The names the agents of a folder carry; `except` is the agent that is being renamed. */
const namesIn = (rootId: string | null, except?: AgentRef): string[] =>
  (useAgents.getState().listing?.agents ?? []).filter((a) => a.rootId === rootId && a.name !== '' && (!except || agentKey(a) !== agentKey(except))).map((a) => a.name)

const problemOf = (name: string, taken: readonly string[]): string | null =>
  name === '' ? null : (agentNameProblem(name) ?? (taken.includes(name) ? `There is already an agent named "${name}" there.` : null))

const usable = (name: string, taken: readonly string[]): boolean => agentNameProblem(name) === null && !taken.includes(name)

/** The agent a dialog is about, as the list has it. */
const summaryOf = (ref: AgentRef): { name: string; file: string } => useAgents.getState().listing?.agents.find((a) => agentKey(a) === agentKey(ref)) ?? { name: '', file: ref.file }

/** Makes an agent: a name, where it lives, and a first description. */
export function NewAgentDialog({ rootId }: { rootId?: string }) {
  const roots = useAgents((s) => s.listing?.roots ?? [])
  const [name, setName] = useState('')
  const [where, setWhere] = useState<string | null>(rootId ?? roots[0]?.id ?? null)
  const [description, setDescription] = useState('')
  const [busy, setBusy] = useState(false)
  const taken = namesIn(where)
  const ready = where !== null && usable(name, taken) && !busy

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = await createAgent(where, name, description)
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title="New agent" onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <NameField value={name} onChange={setName} problem={problemOf(name, taken)} max={AGENT_LIMITS.newName} note={NAME_NOTE} placeholder="code-reviewer" />
        <RootChoice roots={roots} value={where} onChange={setWhere} onAddFolder={addAgentFolder} />
        <label className="field">
          <span>
            Description <em>(what it is for and when Claude should use it; you can write it later)</em>
          </span>
          <textarea className="sk-long" rows={3} value={description} maxLength={2000} spellCheck={false} onChange={(e) => setDescription(e.target.value)} />
        </label>
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

/** A new name for an agent, or the name of its copy. */
export function AgentNameDialog({ mode, target }: { mode: 'rename' | 'duplicate'; target: AgentRef }) {
  const current = summaryOf(target)
  const label = agentLabel(current)
  const taken = namesIn(target.rootId, mode === 'rename' ? target : undefined)
  const base = slugifySkillName(label) || 'agent'
  const [name, setName] = useState(() => (mode === 'rename' ? asName(label, AGENT_LIMITS.newName) : freeSkillName(`${base.slice(0, AGENT_LIMITS.newName - 5)}-copy`, taken)))
  const [busy, setBusy] = useState(false)
  const same = mode === 'rename' && name === current.name
  const ready = usable(name, taken) && !same && !busy

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = mode === 'rename' ? await renameAgent(target, name) : await duplicateAgent(target, name)
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title={mode === 'rename' ? `Rename "${label}"` : `Duplicate "${label}"`} onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <NameField
          value={name}
          onChange={setName}
          problem={problemOf(name, taken)}
          max={AGENT_LIMITS.newName}
          label={mode === 'rename' ? 'New name' : 'Name of the copy'}
          note={NAME_NOTE}
          placeholder="code-reviewer"
        />
        {mode === 'rename' && (
          <p className="note">
            The name inside the file changes, and the file is renamed with it when it was named after the agent. Skills, workflows and prompts that call the agent by
            its old name have to be changed by hand.
          </p>
        )}
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

/** Copies or moves an agent between the personal agents and a project. */
export function AgentTransferDialog({ target }: { target: AgentRef }) {
  const listing = useAgents((s) => s.listing)
  const roots = (listing?.roots ?? []).filter((r) => r.id !== target.rootId)
  const current = summaryOf(target)
  const label = agentLabel(current)
  const [where, setWhere] = useState<string | null>(roots[0]?.id ?? null)
  const [mode, setMode] = useState<'copy' | 'move'>('copy')
  const [name, setName] = useState(current.name)
  const [busy, setBusy] = useState(false)
  const taken = namesIn(where)
  // a name the agent has already may stay as it is, whatever it looks like
  const kept = name === current.name && name !== ''
  const problem = kept ? (taken.includes(name) ? `There is already an agent named "${name}" there.` : null) : problemOf(name, taken)
  const ready = where !== null && name !== '' && problem === null && !busy
  const to = listing?.roots.find((r) => r.id === where)

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    if (!ready) return
    setBusy(true)
    const done = await transferAgent(target, where, mode, name)
    setBusy(false)
    if (done) close()
  }

  return (
    <Dialog title={`Copy or move "${label}"`} onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <RootChoice roots={roots} value={where} onChange={setWhere} onAddFolder={addAgentFolder} />
        <div className="field">
          <span>What happens to this one</span>
          <div className="segmented">
            <button type="button" className={mode === 'copy' ? 'selected' : ''} onClick={() => setMode('copy')}>
              Copy: it stays here
            </button>
            <button type="button" className={mode === 'move' ? 'selected' : ''} title="It is copied there, then this one goes to the bin" onClick={() => setMode('move')}>
              Move: it goes to the bin here
            </button>
          </div>
        </div>
        <NameField value={name} onChange={setName} problem={problem} max={AGENT_LIMITS.newName} label="Name there" note={NAME_NOTE} placeholder="code-reviewer" />
        {to?.kind === 'project' && mode === 'copy' && <p className="note">In that project its agent is used instead of a personal one of the same name.</p>}
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
