import { reorder } from './tabOrder'
import type { UsageState } from './types'
import { usedUp } from './usage'

/** The model aliases offered for a step. A full model id may be typed instead. */
export const MODEL_ALIASES = ['fable', 'opus', 'sonnet', 'haiku'] as const
export const EFFORT_LEVELS = ['low', 'medium', 'high', 'xhigh', 'max'] as const
/** The permission modes a step may start in. `bypassPermissions` is left out on purpose. */
export const PERMISSION_MODES = ['plan', 'manual', 'acceptEdits', 'auto', 'dontAsk'] as const

export type Effort = (typeof EFFORT_LEVELS)[number]
export type PermissionMode = (typeof PERMISSION_MODES)[number]

export const WORKFLOW_LIMITS = { name: 60, stepName: 40, steps: 12, prompt: 100_000, task: 100_000, workflows: 100 } as const

/** One session of a workflow: an account, how Claude runs there, and what it is told. */
export interface WorkflowStep {
  id: string
  /** Shown on the step's card and tab, and used in `{{steps.<name>.output}}`. */
  name: string
  profileId: string
  /** An alias or a full model id. Missing: whatever the account uses by default. */
  model?: string
  effort?: Effort
  permissionMode?: PermissionMode
  /** The id of an earlier step whose conversation this step goes on with, instead of starting a new one. */
  continues?: string
  /** What Claude is told, with `{{...}}` variables. */
  prompt: string
  /** Wait for the user's approval before the next step starts. */
  gate: boolean
}

export interface Workflow {
  id: string
  name: string
  steps: WorkflowStep[]
}

// ---------- reading what comes from a file or from the window ----------

const ID = /^[A-Za-z0-9_-]{1,64}$/
/** A model alias or id as `--model` takes it. It cannot start with a dash, which would make it a flag. */
const MODEL = /^[A-Za-z0-9][A-Za-z0-9._:[\]-]{0,79}$/
const STEP_NAME = /^[\p{L}\p{N}][\p{L}\p{N} _-]*$/u

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value)
const text = (value: unknown, max: number): string => (typeof value === 'string' ? value.slice(0, max) : '')
const oneOf = <T extends string>(value: unknown, choices: readonly T[]): T | undefined =>
  choices.includes(value as T) ? (value as T) : undefined

export function isModel(value: unknown): value is string {
  return typeof value === 'string' && MODEL.test(value)
}

/**
 * A step with every field of the right type; fields this version does not know are kept. Null
 * when it has no usable id. What the fields say is judged by `validateWorkflow`, not here.
 */
export function parseStep(raw: unknown): WorkflowStep | null {
  if (!isRecord(raw) || typeof raw.id !== 'string' || !ID.test(raw.id)) return null
  const step: WorkflowStep = {
    ...raw,
    id: raw.id,
    name: text(raw.name, WORKFLOW_LIMITS.stepName).trim(),
    profileId: text(raw.profileId, 64),
    model: isModel(raw.model) ? raw.model : undefined,
    effort: oneOf(raw.effort, EFFORT_LEVELS),
    permissionMode: oneOf(raw.permissionMode, PERMISSION_MODES),
    continues: typeof raw.continues === 'string' && ID.test(raw.continues) ? raw.continues : undefined,
    prompt: text(raw.prompt, WORKFLOW_LIMITS.prompt),
    gate: raw.gate !== false
  }
  // a missing value is a missing key, also after a trip through the window
  for (const key of ['model', 'effort', 'permissionMode', 'continues'] as const) if (step[key] === undefined) delete step[key]
  return step
}

export function parseWorkflow(raw: unknown): Workflow | null {
  if (!isRecord(raw) || typeof raw.id !== 'string' || !ID.test(raw.id)) return null
  const seen = new Set<string>()
  const steps: WorkflowStep[] = []
  for (const item of Array.isArray(raw.steps) ? raw.steps.slice(0, WORKFLOW_LIMITS.steps) : []) {
    const step = parseStep(item)
    if (!step || seen.has(step.id)) continue
    seen.add(step.id)
    steps.push(step)
  }
  return { ...raw, id: raw.id, name: text(raw.name, WORKFLOW_LIMITS.name).trim(), steps }
}

/** The workflows of a saved file; entries that cannot be read are left out. */
export function parseWorkflows(raw: unknown): Workflow[] {
  const list = isRecord(raw) && Array.isArray(raw.workflows) ? raw.workflows : []
  const seen = new Set<string>()
  const workflows: Workflow[] = []
  for (const item of list.slice(0, WORKFLOW_LIMITS.workflows)) {
    const workflow = parseWorkflow(item)
    if (!workflow || seen.has(workflow.id)) continue
    seen.add(workflow.id)
    workflows.push(workflow)
  }
  return workflows
}

// ---------- prompt variables ----------

/** `{{ ... }}` on one line, without braces inside. */
const TOKEN = /\{\{([^{}\r\n]*)\}\}/g

export type PromptRef =
  | { kind: 'task' }
  | { kind: 'folder' }
  | { kind: 'previous' }
  | { kind: 'step'; name: string }
  /** Looks like a variable and is none; it stays in the prompt as written. */
  | { kind: 'unknown'; text: string }

function refOf(inner: string): PromptRef {
  const key = inner.trim()
  const lower = key.toLowerCase()
  if (lower === 'task') return { kind: 'task' }
  if (lower === 'folder') return { kind: 'folder' }
  if (lower === 'previous.output') return { kind: 'previous' }
  const step = /^steps\.(.+)\.output$/i.exec(key)
  if (step && step[1].trim()) return { kind: 'step', name: step[1].trim() }
  return { kind: 'unknown', text: key }
}

const sameName = (a: string, b: string): boolean => a.trim().toLowerCase() === b.trim().toLowerCase()

/** Everything in a prompt that is written like a variable, in order. */
export function promptRefs(prompt: string): PromptRef[] {
  return [...prompt.matchAll(TOKEN)].map((match) => refOf(match[1]))
}

export interface PromptValues {
  task: string
  folder: string
  /** The output of the step before this one. */
  previous?: string
  /** The output of an earlier step, by its name. */
  step(name: string): string | undefined
}

/**
 * The prompt with its variables filled in. It is one pass: text that arrives through a variable is
 * never read for variables itself. A variable without a value stays as it is written.
 */
export function renderPrompt(prompt: string, values: PromptValues): string {
  return prompt.replace(TOKEN, (whole, inner: string) => {
    const ref = refOf(inner)
    if (ref.kind === 'task') return values.task
    if (ref.kind === 'folder') return values.folder
    if (ref.kind === 'previous') return values.previous ?? whole
    if (ref.kind === 'step') return values.step(ref.name) ?? whole
    return whole
  })
}

/** The prompt with `{{steps.<from>.output}}` pointing at the step's new name. */
export function renameStepRefs(prompt: string, from: string, to: string): string {
  return prompt.replace(TOKEN, (whole, inner: string) => {
    const ref = refOf(inner)
    return ref.kind === 'step' && sameName(ref.name, from) ? `{{steps.${to}.output}}` : whole
  })
}

// ---------- editing ----------

/** "Step 3", or the first such name that is free. */
export function freeStepName(steps: readonly WorkflowStep[], base = 'Step'): string {
  for (let n = steps.length + 1; ; n++) {
    const name = `${base} ${n}`
    if (!steps.some((s) => sameName(s.name, name))) return name
  }
}

/** Renames a step and every `{{steps.<name>.output}}` that names it. */
export function renameStep(workflow: Workflow, stepId: string, name: string): Workflow {
  const old = workflow.steps.find((s) => s.id === stepId)
  if (!old || old.name === name) return workflow
  // a name that is shared with another step cannot be told apart in the prompts
  const rewrite = old.name.trim() !== '' && name.trim() !== '' && !workflow.steps.some((s) => s.id !== stepId && sameName(s.name, old.name))
  return {
    ...workflow,
    steps: workflow.steps.map((s) => ({
      ...s,
      name: s.id === stepId ? name : s.name,
      prompt: rewrite ? renameStepRefs(s.prompt, old.name, name.trim()) : s.prompt
    }))
  }
}

/** Removes a step; steps that went on with its conversation start their own instead. */
export function removeStep(workflow: Workflow, stepId: string): Workflow {
  return {
    ...workflow,
    steps: workflow.steps
      .filter((s) => s.id !== stepId)
      .map((s) => {
        if (s.continues !== stepId) return s
        const { continues: _gone, ...rest } = s
        return rest
      })
  }
}

/** Moves a step in front of another one, or to the end with `beforeId` null. */
export function moveStep(workflow: Workflow, stepId: string, beforeId: string | null): Workflow {
  const order = reorder(
    workflow.steps.map((s) => s.id),
    stepId,
    beforeId
  )
  const steps = order.map((id) => workflow.steps.find((s) => s.id === id)!)
  return steps.every((s, i) => s === workflow.steps[i]) ? workflow : { ...workflow, steps }
}

/**
 * The index of the step whose conversation the step at `index` belongs to: itself when it starts
 * a new one, else the start of the chain of `continues`. A link that does not point at an earlier
 * step counts as none.
 */
export function threadOf(steps: readonly WorkflowStep[], index: number): number {
  let at = index
  for (;;) {
    const target = steps[at]?.continues
    const before = target === undefined ? -1 : steps.findIndex((s, i) => i < at && s.id === target)
    if (before < 0) return at
    at = before
  }
}

/** A workflow to start from: plan on one account, implement on another, review where the plan was made. */
export function sampleWorkflow(newId: () => string, planProfileId: string, implementProfileId: string): Workflow {
  const plan = newId()
  return {
    id: newId(),
    name: 'Plan, implement, review',
    steps: [
      {
        id: plan,
        name: 'Plan',
        profileId: planProfileId,
        model: 'fable',
        permissionMode: 'plan',
        prompt:
          'Plan this task. Do not change any files.\n\nTask:\n{{task}}\n\nExplore the code as far as you need, then present a concrete, step-by-step implementation plan: the files to change, what changes in each, and how to verify the result.',
        gate: true
      },
      {
        id: newId(),
        name: 'Implement',
        profileId: implementProfileId,
        model: 'opus',
        prompt:
          'Implement the following plan in this folder.\n\nTask:\n{{task}}\n\nPlan:\n{{previous.output}}\n\nWhen you are done, summarize what you changed and how you verified it.',
        gate: true
      },
      {
        id: newId(),
        name: 'Review',
        profileId: planProfileId,
        model: 'fable',
        // out of the plan mode the conversation was left in; edits would still ask first
        permissionMode: 'manual',
        continues: plan,
        prompt:
          'Another session has implemented your plan. Review the result against the plan. Do not change any files.\n\nIts summary:\n{{previous.output}}\n\nLook at the actual changes in the folder (for example with git diff). Report what is wrong or missing, most important first, and say clearly whether the work is ready.',
        gate: true
      }
    ]
  }
}

// ---------- checks ----------

export interface WorkflowIssue {
  /** The step it is about; missing for the workflow as a whole. */
  stepId?: string
  /** An error keeps the workflow from running. */
  level: 'error' | 'warning'
  message: string
}

/** What is wrong with a workflow as it is designed. `profileIds` are the accounts that exist. */
export function validateWorkflow(workflow: Workflow, profileIds: ReadonlySet<string>): WorkflowIssue[] {
  const issues: WorkflowIssue[] = []
  const error = (message: string, stepId?: string): void => void issues.push({ stepId, level: 'error', message })
  const warning = (message: string, stepId?: string): void => void issues.push({ stepId, level: 'warning', message })

  if (!workflow.name.trim()) error('The workflow needs a name.')
  if (workflow.steps.length === 0) error('The workflow has no steps.')

  workflow.steps.forEach((step, index) => {
    const name = step.name.trim()
    if (!name) error('The step needs a name.', step.id)
    else if (!STEP_NAME.test(name)) error('A step name can have letters, digits, spaces, "-" and "_", and starts with a letter or digit.', step.id)
    else if (workflow.steps.some((other, i) => i < index && sameName(other.name, name))) error(`Another step is already named "${name}".`, step.id)

    if (!step.profileId) error('Choose an account for this step.', step.id)
    else if (!profileIds.has(step.profileId)) error('The account of this step no longer exists. Choose another one.', step.id)

    if (!step.prompt.trim()) error('The step has no prompt.', step.id)

    if (step.continues !== undefined) {
      const target = workflow.steps.findIndex((s) => s.id === step.continues)
      if (target < 0) error('The step whose conversation this one continues no longer exists.', step.id)
      else if (target >= index) error(`A step can only continue the conversation of a step before it, and "${workflow.steps[target].name}" comes later.`, step.id)
      else {
        // Claude Code resumes a conversation in the mode it was left in
        const thread = threadOf(workflow.steps, index)
        const before = workflow.steps.findLast((_, i) => i < index && threadOf(workflow.steps, i) === thread)
        if (before?.permissionMode === 'plan' && step.permissionMode === undefined) {
          warning(`"${before.name}" leaves the conversation in plan mode, and this step stays in it. Choose a permission mode if it should not.`, step.id)
        }
      }
    }

    for (const ref of promptRefs(step.prompt)) {
      if (ref.kind === 'previous' && index === 0) error('The first step has no previous step: {{previous.output}} cannot be used in it.', step.id)
      else if (ref.kind === 'step') {
        const target = workflow.steps.findIndex((s) => sameName(s.name, ref.name))
        if (target < 0) error(`The prompt uses the output of "${ref.name}", and no step has that name.`, step.id)
        else if (target >= index) error(`The prompt uses the output of "${ref.name}", which has not run yet at this point.`, step.id)
      } else if (ref.kind === 'unknown') {
        warning(`{{${ref.text}}} is not a variable; it is sent as it is written.`, step.id)
      }
    }
  })
  return issues
}

interface AccountRef {
  id: string
  name: string
  account?: { loggedIn: boolean }
}

/**
 * What stands in the way of running a workflow right now: an account that is not signed in is an
 * error, an account that looks out of its limit is a warning (it may have extra usage enabled).
 */
export function preflight(
  workflow: Workflow,
  profiles: readonly AccountRef[],
  usage: Readonly<Record<string, UsageState>>,
  now: number
): WorkflowIssue[] {
  const issues: WorkflowIssue[] = []
  const told = new Set<string>()
  for (const step of workflow.steps) {
    const profile = profiles.find((p) => p.id === step.profileId)
    if (!profile) continue
    if (!profile.account?.loggedIn) {
      if (!told.has(profile.id)) issues.push({ stepId: step.id, level: 'error', message: `"${profile.name}" is not signed in.` })
      told.add(profile.id)
      continue
    }
    const limit = usedUp(usage[profile.id], now, step.model)
    const key = `${profile.id} ${limit?.label}`
    if (limit && !told.has(key)) {
      told.add(key)
      issues.push({ stepId: step.id, level: 'warning', message: `"${profile.name}" looks out of its ${limit.label}; step "${step.name}" may stop there.` })
    }
  }
  return issues
}

// ---------- a run, as the window sees it ----------

/**
 * `starting`: its session is being opened. `running`: Claude works on it, or waits in its tab.
 * `gate`: it is finished and the run waits for approval. `paused`: it cannot go on by itself.
 */
export type StepState = 'pending' | 'starting' | 'running' | 'gate' | 'done' | 'paused'
/** `waiting`: at a gate. `paused`: the current step needs a retry or a hand. */
export type RunState = 'running' | 'waiting' | 'paused' | 'done' | 'stopped'

export interface RunStep {
  id: string
  name: string
  profileId: string
  state: StepState
  /** The tab the step runs in, while that tab exists. */
  sessionId?: string
  /** Claude waits for the user in that tab: a permission, a question, a dialog. */
  needsYou?: boolean
  /** Claude gave an answer, and work it started in the background is still running. */
  background?: boolean
  hasOutput: boolean
  /** Why the step paused. */
  problem?: string
}

export interface WorkflowRun {
  id: string
  workflowId: string
  /** The workflow's name when the run began. */
  name: string
  cwd: string
  state: RunState
  /** Index of the step the run is at. */
  current: number
  steps: RunStep[]
  startedAt: number
  endedAt?: number
}

export interface StartRunInput {
  workflowId: string
  cwd: string
  task: string
}

/** A run that still has something to do or to wait for. */
export function isActiveRun(run: Pick<WorkflowRun, 'state'>): boolean {
  return run.state === 'running' || run.state === 'waiting' || run.state === 'paused'
}
