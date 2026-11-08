import { useEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent } from 'react'
import { dropTarget, type DropTarget } from '@shared/tabOrder'
import type { ProfileView } from '@shared/types'
import {
  WORKFLOW_LIMITS,
  moveStep,
  removeStep,
  renameStep,
  validateWorkflow,
  type Workflow,
  type WorkflowIssue,
  type WorkflowStep
} from '@shared/workflows'
import { followDrag } from '../pointerDrag'
import {
  closePanel,
  createWorkflow,
  deleteWorkflow,
  duplicateWorkflow,
  flushWorkflows,
  newStep,
  selectWorkflow,
  setModal,
  updateWorkflow,
  useStore
} from '../store'
import { StepEditor, modeLabel } from './StepEditor'

// the chain is laid out by these numbers, so that the lines under it can be drawn without measuring
const CARD_W = 184
const LINK_W = 52
const ARC_BASE = 18
const ARC_STEP = 16

const centerOf = (index: number): number => index * (CARD_W + LINK_W) + CARD_W / 2

interface Arc {
  from: number
  to: number
  /** How far under the cards its line runs. */
  depth: number
}

/**
 * A line from each step that goes on with an earlier conversation to the step it goes on from.
 * Lines that would run over each other get a level of their own; the short ones stay on top.
 */
function conversationArcs(steps: readonly WorkflowStep[]): Arc[] {
  const links = steps
    .flatMap((step, to) => {
      const from = step.continues === undefined ? -1 : steps.findIndex((s, i) => i < to && s.id === step.continues)
      return from < 0 ? [] : [{ from, to }]
    })
    .sort((a, b) => a.to - a.from - (b.to - b.from) || a.from - b.from)
  const placed: (Arc & { level: number })[] = []
  for (const link of links) {
    let level = 0
    while (placed.some((p) => p.level === level && p.from < link.to && link.from < p.to)) level++
    placed.push({ ...link, level, depth: ARC_BASE + ARC_STEP * level })
  }
  return placed
}

interface ChainProps {
  workflow: Workflow
  profiles: readonly ProfileView[]
  issues: readonly WorkflowIssue[]
  selected: string | null
  onSelect(stepId: string): void
  onChange(workflow: Workflow): void
}

/** The steps from left to right, with an arrow from each to the next and a line under steps that share a conversation. */
function StepChain({ workflow, profiles, issues, selected, onSelect, onChange }: ChainProps) {
  const row = useRef<HTMLDivElement>(null)
  const [dragged, setDragged] = useState<string | null>(null)
  const [mark, setMark] = useState<DropTarget | null>(null)
  const { steps } = workflow

  /** Where a step dragged to a point (client coordinates) would land; null outside the row of steps. */
  const targetAt = (clientX: number, clientY: number): DropTarget | null => {
    const el = row.current
    if (!el) return null
    const box = el.getBoundingClientRect()
    if (clientX < box.left || clientX >= box.right || clientY < box.top || clientY >= box.bottom) return null
    const cards = [...el.querySelectorAll<HTMLElement>('.wf-card')]
    return dropTarget(
      cards.map((card) => {
        const { left, right } = card.getBoundingClientRect()
        return { id: card.dataset.id!, left, right }
      }),
      clientX
    )
  }

  // the drag outlives the render it started in; it moves the step in the workflow as it is by then
  const latest = useRef({ workflow, onChange })
  latest.current = { workflow, onChange }

  /** A button went down on a step: a few pixels on, it drags the step to another place in the chain. */
  const onCardPointerDown = (e: PointerEvent<HTMLElement>, stepId: string): void => {
    const finish = (): void => {
      setDragged(null)
      setMark(null)
    }
    followDrag(e, {
      start: () => setDragged(stepId),
      move(x, y) {
        const next = targetAt(x, y)
        setMark((prev) => (prev?.id === next?.id && prev?.side === next?.side ? prev : next))
        return next !== null
      },
      drop(x, y) {
        const next = targetAt(x, y)
        finish()
        const { workflow: now, onChange: change } = latest.current
        if (next && now.steps.some((s) => s.id === stepId)) change(moveStep(now, stepId, next.beforeId))
      },
      cancel: finish
    })
  }

  const arcs = conversationArcs(steps)
  const arcHeight = arcs.reduce((max, arc) => Math.max(max, arc.depth), 0) + 12
  const width = steps.length * (CARD_W + LINK_W)
  const canAdd = steps.length < WORKFLOW_LIMITS.steps

  const add = (): void => {
    const step = newStep(steps)
    onChange({ ...workflow, steps: [...steps, step] })
    onSelect(step.id)
  }

  return (
    <div className="wf-chain">
      <div className="wf-row" ref={row}>
        {steps.map((step, index) => {
          const profile = profiles.find((p) => p.id === step.profileId)
          const broken = issues.some((i) => i.stepId === step.id && i.level === 'error')
          const how = [step.model ?? 'default model', step.effort, step.permissionMode && modeLabel(step.permissionMode)].filter(Boolean).join(' · ')
          const classes = ['wf-card', step.id === selected && 'selected', broken && 'broken', dragged === step.id && 'dragged', mark?.id === step.id && `drop-${mark.side}`]
            .filter(Boolean)
            .join(' ')
          return (
            <div className="wf-cell" key={step.id}>
              <div
                className={classes}
                data-id={step.id}
                role="button"
                tabIndex={0}
                style={{ '--accent': profile?.color ?? '#888' } as CSSProperties}
                title={broken ? 'This step has a problem; select it to see what.' : 'Drag to move the step'}
                onPointerDown={(e) => onCardPointerDown(e, step.id)}
                onClick={() => onSelect(step.id)}
                onKeyDown={(e) => {
                  if (e.key !== 'Enter' && e.key !== ' ') return
                  e.preventDefault()
                  onSelect(step.id)
                }}
              >
                <div className="wf-card-head">
                  <span className="wf-number">{index + 1}</span>
                  <span className="wf-card-name">{step.name || 'Unnamed'}</span>
                  {broken && <span className="wf-flag">!</span>}
                </div>
                <div className="wf-card-account">
                  <span className="dot" />
                  {profile?.name ?? 'No account'}
                </div>
                <div className="wf-card-how">{how}</div>
              </div>
              {index < steps.length - 1 ? (
                <div className={`wf-link${step.gate ? ' gated' : ''}`} title={step.gate ? 'Waits for your approval before the next step' : 'Goes on by itself'}>
                  {step.gate && <span className="wf-gate">approve</span>}
                </div>
              ) : (
                <div className="wf-link last">
                  <button className="wf-add" disabled={!canAdd} title={canAdd ? 'Add a step' : `A workflow has at most ${WORKFLOW_LIMITS.steps} steps`} onClick={add}>
                    +
                  </button>
                </div>
              )}
            </div>
          )
        })}
        {steps.length === 0 && (
          <button className="btn" onClick={add}>
            + Add the first step
          </button>
        )}
      </div>
      {arcs.length > 0 && (
        <svg className="wf-arcs" width={width} height={arcHeight} aria-hidden="true">
          {arcs.map((arc) => {
            const x1 = centerOf(arc.from)
            const x2 = centerOf(arc.to)
            return (
              <g key={`${arc.from}-${arc.to}`}>
                <path d={`M ${x1} 0 V ${arc.depth - 8} Q ${x1} ${arc.depth} ${x1 + 8} ${arc.depth} H ${x2 - 8} Q ${x2} ${arc.depth} ${x2} ${arc.depth - 8} V 5`} />
                <path className="head" d={`M ${x2 - 4} 7 L ${x2} 0 L ${x2 + 4} 7 Z`} />
                <text x={(x1 + x2) / 2} y={arc.depth + 3} textAnchor="middle">
                  same conversation
                </text>
              </g>
            )
          })}
        </svg>
      )}
    </div>
  )
}

/**
 * The workflow designer: the list of workflows, the chain of steps of the one that is chosen, and
 * the settings of the step that is selected. It is shown in its own tab (`panel:workflows`).
 */
export function WorkflowsPanel() {
  const workflows = useStore((s) => s.workflows)
  const workflowId = useStore((s) => s.workflowId)
  const profiles = useStore((s) => s.profiles)
  const claudeFound = useStore((s) => s.claude.path !== null)
  const [stepId, setStepId] = useState<string | null>(null)
  const root = useRef<HTMLDivElement>(null)

  // a tab restored from the last run has chosen nothing yet
  const workflow = workflows.find((w) => w.id === workflowId) ?? workflows[0] ?? null
  const step = workflow?.steps.find((s) => s.id === stepId) ?? workflow?.steps[0] ?? null
  const issues = useMemo(() => (workflow ? validateWorkflow(workflow, new Set(profiles.map((p) => p.id))) : []), [workflow, profiles])
  const general = issues.filter((i) => i.stepId === undefined)
  const errors = issues.filter((i) => i.level === 'error').length

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const { modal, confirm } = useStore.getState()
      // a dialog on top takes the key for itself; a terminal beside the designer keeps its Escape
      if (e.key === 'Escape' && !modal && !confirm && e.target instanceof Node && root.current?.contains(e.target)) closePanel('workflows')
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])

  // what was typed is saved when the designer goes out of sight: another tab in front, or its tab closed
  useEffect(() => flushWorkflows, [])

  const changeStep = (next: WorkflowStep): void => {
    if (!workflow || !step) return
    // a new name is also written into the prompts that use this step's output
    if (next.name !== step.name) updateWorkflow(renameStep(workflow, next.id, next.name))
    else updateWorkflow({ ...workflow, steps: workflow.steps.map((s) => (s.id === next.id ? next : s)) })
  }

  return (
    <div className="wf-panel" ref={root} tabIndex={-1}>
      <header className="wf-head">
        <span className="wf-title">Workflows</span>
        <span className="muted">A chain of sessions, each on the account and model you choose. Every step runs in a tab of its own.</span>
        <span className="spacer" />
        <button className="icon-btn" title="Close (Esc)" onClick={() => closePanel('workflows')}>
          ×
        </button>
      </header>
      <div className="wf-body">
        <nav className="wf-list">
          {workflows.map((w) => (
            <button key={w.id} className={`wf-item${w.id === workflow?.id ? ' selected' : ''}`} onClick={() => selectWorkflow(w.id)}>
              <span className="wf-item-name">{w.name || 'Untitled'}</span>
              <span className="wf-item-steps">{w.steps.map((s) => s.name).join(' → ') || 'No steps'}</span>
            </button>
          ))}
          <div className="wf-list-actions">
            <button className="btn" onClick={createWorkflow}>
              + New workflow
            </button>
          </div>
        </nav>

        {workflow ? (
          <section className="wf-design">
            <div className="wf-toolbar">
              <input
                className="wf-name"
                value={workflow.name}
                maxLength={WORKFLOW_LIMITS.name}
                placeholder="Name of the workflow"
                spellCheck={false}
                onChange={(e) => updateWorkflow({ ...workflow, name: e.target.value })}
              />
              <button className="btn" onClick={() => duplicateWorkflow(workflow.id)}>
                Duplicate
              </button>
              <button className="btn" onClick={() => void deleteWorkflow(workflow.id)}>
                Delete
              </button>
              <button
                className="btn primary"
                disabled={errors > 0 || !claudeFound}
                title={errors > 0 ? 'Fix the marked steps first' : !claudeFound ? 'claude was not found. Set its path in Settings.' : 'Choose a folder and a task, and start'}
                onClick={() => setModal({ type: 'runWorkflow', workflowId: workflow.id })}
              >
                Run…
              </button>
            </div>
            {general.map((issue) => (
              <p key={issue.message} className={`note ${issue.level === 'error' ? 'bad' : 'warn'}`}>
                {issue.message}
              </p>
            ))}

            <StepChain workflow={workflow} profiles={profiles} issues={issues} selected={step?.id ?? null} onSelect={setStepId} onChange={updateWorkflow} />

            {step && (
              <StepEditor
                key={step.id}
                step={step}
                index={workflow.steps.indexOf(step)}
                workflow={workflow}
                profiles={profiles}
                issues={issues.filter((i) => i.stepId === step.id)}
                onChange={changeStep}
                onDelete={() => {
                  updateWorkflow(removeStep(workflow, step.id))
                  setStepId(null)
                }}
              />
            )}
          </section>
        ) : (
          <section className="wf-design wf-empty">
            <div className="empty-title">No workflow yet</div>
            <div className="muted">
              A workflow runs sessions one after the other and hands each one's answer to the next: plan on one account, implement on another, review where the plan
              was made.
            </div>
            <button className="btn primary" onClick={createWorkflow}>
              New workflow
            </button>
          </section>
        )}
      </div>
    </div>
  )
}
