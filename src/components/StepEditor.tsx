import { useRef, useState, type CSSProperties, type ReactNode } from 'react'
import type { ProfileView } from '@shared/types'
import {
  EFFORT_LEVELS,
  MODEL_ALIASES,
  PERMISSION_MODES,
  WORKFLOW_LIMITS,
  isModel,
  type Effort,
  type PermissionMode,
  type Workflow,
  type WorkflowIssue,
  type WorkflowStep
} from '@shared/workflows'

const MODE_LABELS: Record<PermissionMode, { label: string; hint: string }> = {
  plan: { label: 'Plan', hint: 'Claude explores and presents a plan; it changes nothing until the plan is approved' },
  manual: { label: 'Ask', hint: 'Claude asks before every change and command' },
  acceptEdits: { label: 'Accept edits', hint: 'File edits are accepted without asking; other actions still ask' },
  auto: { label: 'Auto', hint: 'Claude works on its own and only stops for what looks risky' },
  dontAsk: { label: "Don't ask", hint: 'What the permission rules do not allow is refused instead of asked about' }
}

/** Stands for "a model id that is typed in" among the model choices; no model is called this. */
const OTHER_MODEL = ' other'

export function modeLabel(mode: PermissionMode): string {
  return MODE_LABELS[mode].label.toLowerCase()
}

interface Option<T> {
  value: T
  label: string
  hint?: string
}

/** One of a few, as a row of buttons. */
function Choice<T extends string | undefined>({ options, value, onChange }: { options: readonly Option<T>[]; value: T; onChange(value: T): void }) {
  return (
    <div className="segmented wrap">
      {options.map((option) => (
        <button key={option.label} type="button" className={option.value === value ? 'selected' : ''} title={option.hint} onClick={() => onChange(option.value)}>
          {option.label}
        </button>
      ))}
    </div>
  )
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="field">
      <span>{label}</span>
      {children}
    </div>
  )
}

interface Props {
  step: WorkflowStep
  index: number
  workflow: Workflow
  profiles: readonly ProfileView[]
  issues: readonly WorkflowIssue[]
  onChange(step: WorkflowStep): void
  onDelete(): void
}

/** The settings of one step: who runs it, how, in which conversation, and what it is told. */
export function StepEditor({ step, index, workflow, profiles, issues, onChange, onDelete }: Props) {
  const prompt = useRef<HTMLTextAreaElement>(null)
  const earlier = workflow.steps.slice(0, index)
  const last = index === workflow.steps.length - 1
  const continued = earlier.find((s) => s.id === step.continues)
  const account = profiles.find((p) => p.id === step.profileId)
  // a model that is typed in rather than picked; the field stays while it is being retyped
  const [typed, setTyped] = useState(step.model !== undefined && !(MODEL_ALIASES as readonly string[]).includes(step.model))

  const set = (patch: Partial<WorkflowStep>): void => {
    const next = { ...step, ...patch }
    // a setting that is back at its default is not written down at all
    for (const key of ['model', 'effort', 'permissionMode', 'continues'] as const) if (next[key] === undefined) delete next[key]
    onChange(next)
  }

  /** Puts a variable where the cursor is in the prompt. */
  const insert = (token: string): void => {
    const el = prompt.current
    const start = el?.selectionStart ?? step.prompt.length
    const end = el?.selectionEnd ?? step.prompt.length
    set({ prompt: (step.prompt.slice(0, start) + token + step.prompt.slice(end)).slice(0, WORKFLOW_LIMITS.prompt) })
    requestAnimationFrame(() => {
      el?.focus()
      el?.setSelectionRange(start + token.length, start + token.length)
    })
  }

  const variables = [
    { token: '{{task}}', hint: 'What you type when you start the run' },
    ...(index > 0 ? [{ token: '{{previous.output}}', hint: `The answer of the step before this one (${earlier[index - 1].name})` }] : []),
    ...earlier.slice(0, -1).map((s) => ({ token: `{{steps.${s.name}.output}}`, hint: `The answer of "${s.name}"` })),
    { token: '{{folder}}', hint: 'The folder the run works in' }
  ]

  return (
    <div className="wf-editor">
      <div className="wf-settings">
        <Row label="Step name">
          <input value={step.name} maxLength={WORKFLOW_LIMITS.stepName} spellCheck={false} onChange={(e) => set({ name: e.target.value })} />
        </Row>

        <Row label="Account">
          <div className="chips">
            {profiles.map((p) => (
              <button
                key={p.id}
                type="button"
                className={`chip${p.id === step.profileId ? ' selected' : ''}`}
                style={{ '--accent': p.color } as CSSProperties}
                title={p.account?.loggedIn ? p.account.email : 'Not signed in'}
                onClick={() => set({ profileId: p.id })}
              >
                <span className="dot" />
                {p.name}
                {!p.account?.loggedIn && <span className="chip-usage red">signed out</span>}
              </button>
            ))}
          </div>
        </Row>

        <Row label="Model">
          <Choice<string | undefined>
            options={[
              { value: undefined, label: 'Default', hint: 'Whatever this account uses when nothing is chosen' },
              ...MODEL_ALIASES.map((m) => ({ value: m as string, label: m[0].toUpperCase() + m.slice(1), hint: `The latest ${m} model` })),
              { value: OTHER_MODEL, label: 'Other', hint: 'A full model id' }
            ]}
            value={typed ? OTHER_MODEL : step.model}
            onChange={(model) => {
              setTyped(model === OTHER_MODEL)
              if (model !== OTHER_MODEL) set({ model })
            }}
          />
          {typed && (
            <input
              value={step.model ?? ''}
              spellCheck={false}
              autoFocus
              placeholder="A model id, for example claude-opus-5-5"
              onChange={(e) => {
                const model = e.target.value.trim()
                // only what `--model` could take is kept
                if (model === '' || isModel(model)) set({ model: model === '' ? undefined : model })
              }}
            />
          )}
        </Row>

        <Row label="Effort">
          <Choice<Effort | undefined>
            options={[{ value: undefined, label: 'Default', hint: "The account's own setting" }, ...EFFORT_LEVELS.map((level) => ({ value: level, label: level }))]}
            value={step.effort}
            onChange={(effort) => set({ effort })}
          />
        </Row>

        <Row label="Permission mode">
          <Choice<PermissionMode | undefined>
            options={[
              {
                value: undefined,
                label: 'Default',
                hint: continued ? 'The mode the conversation was left in' : "The account's own setting"
              },
              ...PERMISSION_MODES.map((mode) => ({ value: mode, label: MODE_LABELS[mode].label, hint: MODE_LABELS[mode].hint }))
            ]}
            value={step.permissionMode}
            onChange={(permissionMode) => set({ permissionMode })}
          />
        </Row>

        <Row label="Conversation">
          <Choice<string | undefined>
            options={[
              { value: undefined, label: 'New conversation', hint: 'The step starts with an empty context and knows only what its prompt says' },
              ...earlier.map((s) => ({ value: s.id, label: `Go on with "${s.name}"`, hint: `The step continues the conversation of "${s.name}", with everything that was said there` })),
              // the step was moved in front of the one it went on with: shown, so that it can be chosen away
              ...(step.continues !== undefined && !continued ? [{ value: step.continues, label: 'A step that is not before this one', hint: 'Not possible. Choose another.' }] : [])
            ]}
            value={step.continues}
            onChange={(continues) => set({ continues })}
          />
          {continued && continued.profileId !== step.profileId && (
            <p className="note">
              The conversation is copied from {profiles.find((p) => p.id === continued.profileId)?.name ?? 'the other account'} to {account?.name ?? 'this account'} when
              the step starts. Its first message there sends the whole conversation without the cache.
            </p>
          )}
          {index === 0 && <p className="note">The first step always starts a new conversation.</p>}
        </Row>

        {!last && (
          <label className="check" title="The run waits after this step until you press Continue. Meanwhile you can read the result and go on talking in its tab.">
            <input type="checkbox" checked={step.gate} onChange={(e) => set({ gate: e.target.checked })} />
            Wait for my approval before the next step starts
          </label>
        )}

        <div className="wf-step-actions">
          <button className="btn" onClick={onDelete}>
            Delete step
          </button>
        </div>
      </div>

      <div className="wf-prompt">
        <div className="wf-prompt-head">
          <span>Prompt</span>
          <span className="spacer" />
          {variables.map((v) => (
            <button key={v.token} type="button" className="wf-var" title={`${v.hint}. Click to insert.`} onClick={() => insert(v.token)}>
              {v.token}
            </button>
          ))}
        </div>
        <textarea
          ref={prompt}
          value={step.prompt}
          maxLength={WORKFLOW_LIMITS.prompt}
          spellCheck={false}
          placeholder="What this step should do. Use the variables above for the task and for the answers of earlier steps."
          onChange={(e) => set({ prompt: e.target.value })}
        />
        {issues.map((issue) => (
          <p key={issue.message} className={`note ${issue.level === 'error' ? 'bad' : 'warn'}`}>
            {issue.message}
          </p>
        ))}
      </div>
    </div>
  )
}
