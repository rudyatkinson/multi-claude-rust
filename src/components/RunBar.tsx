import type { CSSProperties } from 'react'
import { isActiveRun, type RunStep, type WorkflowRun } from '@shared/workflows'
import { splitPath } from '../format'
import { approveRun, closePanel, dismissRun, retryRun, setActive, setModal, stopRun, takeAnswer, useStore } from '../store'
import { focusTerminal } from '../terminal/terminalRegistry'

const STEP_HINTS: Record<RunStep['state'], string> = {
  pending: 'Not started yet',
  starting: 'Opening its session',
  running: 'Claude is working',
  gate: 'Finished; waits for your approval',
  done: 'Finished',
  paused: 'Stopped; needs a retry or a hand'
}

function showTab(sessionId: string | undefined): void {
  if (!sessionId || !useStore.getState().sessions.some((s) => s.id === sessionId)) return
  // the tab may be under a panel
  closePanel()
  setActive(sessionId)
  focusTerminal(sessionId)
}

/** What a run is doing right now, in a sentence. */
function statusOf(run: WorkflowRun): { text: string; tone: '' | 'warn' | 'bad' | 'ok' } {
  const step = run.steps[run.current]
  if (run.state === 'done') return { text: 'Finished', tone: 'ok' }
  if (run.state === 'stopped') return { text: `Stopped at ${step.name}`, tone: '' }
  if (run.state === 'paused') return { text: step.problem ?? `${step.name} cannot go on by itself`, tone: 'bad' }
  if (run.state === 'waiting') return { text: `${step.name} is finished. Read the result, then continue.`, tone: 'warn' }
  if (step.state === 'starting') return { text: `Opening ${step.name}…`, tone: '' }
  if (step.needsYou) return { text: `${step.name} waits for you in its tab`, tone: 'warn' }
  if (step.background) return { text: `${step.name} has answered; work it started is still running`, tone: '' }
  return { text: `${step.name} is working…`, tone: '' }
}

function RunBar({ run }: { run: WorkflowRun }) {
  const profiles = useStore((s) => s.profiles)
  const sessions = useStore((s) => s.sessions)
  const step = run.steps[run.current]
  const next = run.steps[run.current + 1]
  const active = isActiveRun(run)
  const status = statusOf(run)
  const hasTab = (s: RunStep): boolean => s.sessionId !== undefined && sessions.some((x) => x.id === s.sessionId)

  return (
    <div className={`run-bar ${run.state}`}>
      <span className="run-name" title={`${run.name}\n${run.cwd}`}>
        {run.name}
        <span className="muted"> · {splitPath(run.cwd).name}</span>
      </span>

      <div className="run-pills">
        {run.steps.map((s, index) => {
          const profile = profiles.find((p) => p.id === s.profileId)
          const current = index === run.current && active
          // what the dot of the step shows: only the step a live run is at has more to say than done or not
          const mark = s.state === 'done' ? 'done' : !current ? 'pending' : s.needsYou ? 'needs' : s.state
          return (
            <span key={s.id} className="run-pill-wrap">
              {index > 0 && <span className="run-arrow">→</span>}
              <span className={`run-pill ${mark}${current ? ' current' : ''}`} style={{ '--accent': profile?.color ?? '#888' } as CSSProperties}>
                <button
                  className="run-pill-main"
                  disabled={!hasTab(s)}
                  title={`${s.name} · ${profile?.name ?? 'deleted account'}\n${s.needsYou ? 'Claude waits for you in this tab' : STEP_HINTS[s.state]}${hasTab(s) ? '\nClick to show its tab' : ''}`}
                  onClick={() => showTab(s.sessionId)}
                >
                  <span className="run-mark" />
                  {s.name}
                </button>
                {s.hasOutput && (
                  <button className="run-pill-out" title={`Show what ${s.name} handed on`} onClick={() => setModal({ type: 'stepOutput', runId: run.id, index })}>
                    ≡
                  </button>
                )}
              </span>
            </span>
          )
        })}
      </div>

      <span className={`run-status ${status.tone}`} title={status.text}>
        {status.text}
      </span>

      {run.state === 'waiting' && next && (
        <button className="btn primary" title={`Start ${next.name} with this result`} onClick={() => approveRun(run.id)}>
          Continue to {next.name}
        </button>
      )}
      {run.state === 'paused' && (
        <button className="btn primary" title={`Run ${step.name} again from its start`} onClick={() => retryRun(run.id)}>
          Retry {step.name}
        </button>
      )}
      {active && (step.state === 'running' || step.state === 'paused') && hasTab(step) && (
        <button
          className="btn"
          title={`Finish ${step.name} with what Claude last said in its tab. For a step that is done and was not noticed, or one you finished by hand.`}
          onClick={() => void takeAnswer(run.id)}
        >
          Use last answer
        </button>
      )}
      {active ? (
        <button className="btn" title="End the run here; its tabs stay open" onClick={() => void stopRun(run.id)}>
          Stop
        </button>
      ) : (
        <button className="tab-close" title="Remove from this list" onClick={() => dismissRun(run.id)}>
          ×
        </button>
      )}
    </div>
  )
}

/** One line per workflow run, above the terminals: where it is, and what it waits for. */
export function RunBars() {
  const runs = useStore((s) => s.runs)
  if (runs.length === 0) return null
  return (
    <div className="run-bars">
      {runs.map((run) => (
        <RunBar key={run.id} run={run} />
      ))}
    </div>
  )
}
