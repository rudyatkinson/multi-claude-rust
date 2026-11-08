import { useEffect, useMemo, useState, type CSSProperties } from 'react'
import { WORKFLOW_LIMITS, preflight, promptRefs, validateWorkflow } from '@shared/workflows'
import { errorMessage, mc } from '../api'
import { attempt, runWorkflow, setModal, toast, useStore } from '../store'
import { Dialog } from './Dialogs'

const close = (): void => setModal(null)

/** Asks where a workflow should run and what its task is, and starts it. */
export function RunDialog({ workflowId }: { workflowId: string }) {
  const workflow = useStore((s) => s.workflows.find((w) => w.id === workflowId))
  const profiles = useStore((s) => s.profiles)
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const recentFolders = useStore((s) => s.recentFolders)
  // the folder of the tab that was in front is the likeliest place
  const [cwd, setCwd] = useState(() => {
    const s = useStore.getState()
    return s.sessions.find((x) => x.id === s.activeId && x.kind === 'claude')?.cwd ?? s.recentFolders[0] ?? ''
  })
  const [task, setTask] = useState('')
  const [starting, setStarting] = useState(false)

  const issues = useMemo(
    () => (workflow ? [...validateWorkflow(workflow, new Set(profiles.map((p) => p.id))), ...preflight(workflow, profiles, usage, now)] : []),
    [workflow, profiles, usage, now]
  )
  useEffect(() => {
    if (!workflow) close()
  }, [workflow])
  if (!workflow) return null

  const usesTask = workflow.steps.some((s) => promptRefs(s.prompt).some((ref) => ref.kind === 'task'))
  const blocked = issues.some((i) => i.level === 'error')
  const ready = !blocked && cwd.trim() !== '' && (!usesTask || task.trim() !== '') && !starting

  const start = async (): Promise<void> => {
    if (!ready) return
    setStarting(true)
    // the dialog closes itself when the run has started
    if (!(await runWorkflow(workflow.id, cwd.trim(), task))) setStarting(false)
  }

  const browse = async (): Promise<void> => {
    const picked = await attempt(mc.invoke('dialog:pickFolder', cwd || undefined))
    if (picked) setCwd(picked)
  }

  return (
    <Dialog title={`Run "${workflow.name}"`} onClose={close}>
      <div className="run-steps">
        {workflow.steps.map((step, index) => {
          const profile = profiles.find((p) => p.id === step.profileId)
          return (
            <span key={step.id} className="run-step" style={{ '--accent': profile?.color ?? '#888' } as CSSProperties}>
              {index > 0 && <span className="run-arrow">→</span>}
              <span className="dot" />
              {step.name}
              <span className="muted">
                {profile?.name}
                {step.model && ` · ${step.model}`}
              </span>
            </span>
          )
        })}
      </div>

      <label className="field">
        <span>Folder</span>
        <div className="run-folder">
          <input list="run-folders" value={cwd} spellCheck={false} placeholder="The project folder every step works in" onChange={(e) => setCwd(e.target.value)} />
          <datalist id="run-folders">
            {recentFolders.map((folder) => (
              <option key={folder} value={folder} />
            ))}
          </datalist>
          <button type="button" className="btn" onClick={() => void browse()}>
            Browse…
          </button>
        </div>
      </label>

      <label className="field">
        <span>
          Task <em>{usesTask ? '(what the prompts call {{task}})' : '(no step of this workflow uses {{task}})'}</em>
        </span>
        <textarea
          className="run-task"
          autoFocus
          value={task}
          maxLength={WORKFLOW_LIMITS.task}
          placeholder="What should be done?"
          onChange={(e) => setTask(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) void start()
          }}
        />
      </label>

      {issues.map((issue) => (
        <p key={issue.message} className={`note ${issue.level === 'error' ? 'bad' : 'warn'}`}>
          {issue.message}
        </p>
      ))}
      <p className="note">
        Each step opens in a tab of its own, in the area that has the keyboard now. The run lasts as long as the app is open; its tabs and conversations stay
        afterwards.
      </p>

      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          Cancel
        </button>
        <button className="btn primary" disabled={!ready} title="Ctrl+Enter" onClick={() => void start()}>
          {starting ? 'Starting…' : 'Run'}
        </button>
      </div>
    </Dialog>
  )
}

/** Shows what a step of a run handed on. */
export function StepOutputDialog({ runId, index }: { runId: string; index: number }) {
  const step = useStore((s) => s.runs.find((r) => r.id === runId)?.steps[index])
  const [text, setText] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  // read again when the step gets a newer answer while this is open
  const version = `${step?.state}:${step?.hasOutput}`

  useEffect(() => {
    mc.invoke('workflows:output', runId, index).then(
      (output) => setText(output ?? ''),
      (err) => setError(errorMessage(err))
    )
  }, [runId, index, version])

  const copy = async (): Promise<void> => {
    if (text === null) return
    if (await attempt(mc.invoke('clipboard:writeText', text).then(() => true))) toast('info', 'Copied.')
  }

  return (
    <Dialog title={`Output of ${step?.name ?? 'the step'}`} onClose={close}>
      {error && <p className="note bad">{error}</p>}
      {text === null && !error && <p className="note">Reading…</p>}
      {text !== null && (text ? <pre className="run-output">{text}</pre> : <p className="note">This step has handed on nothing yet.</p>)}
      <p className="note">This is what the next step receives as {'{{previous.output}}'}.</p>
      <div className="dialog-actions">
        <button className="btn" disabled={!text} onClick={() => void copy()}>
          Copy
        </button>
        <button className="btn primary" onClick={close}>
          Close
        </button>
      </div>
    </Dialog>
  )
}
