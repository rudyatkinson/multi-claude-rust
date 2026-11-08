import { describe, expect, it } from 'vitest'
import {
  freeStepName,
  isModel,
  moveStep,
  parseStep,
  parseWorkflow,
  parseWorkflows,
  preflight,
  promptRefs,
  removeStep,
  renameStep,
  renameStepRefs,
  renderPrompt,
  sampleWorkflow,
  threadOf,
  validateWorkflow,
  type Workflow,
  type WorkflowStep
} from '../src/shared/workflows'

const step = (id: string, patch: Partial<WorkflowStep> = {}): WorkflowStep => ({
  id,
  name: id,
  profileId: 'main',
  prompt: 'Do {{task}}',
  gate: true,
  ...patch
})
const flow = (...steps: WorkflowStep[]): Workflow => ({ id: 'w1', name: 'Flow', steps })
const accounts = new Set(['main', 'work'])
const messages = (workflow: Workflow, level?: 'error' | 'warning'): string[] =>
  validateWorkflow(workflow, accounts)
    .filter((i) => !level || i.level === level)
    .map((i) => i.message)

describe('parseStep', () => {
  it('keeps what is valid and drops what is not', () => {
    const parsed = parseStep({ id: 's1', name: '  Plan  ', profileId: 'main', model: 'opus', effort: 'ultra', permissionMode: 'plan', prompt: 'x', gate: false })
    expect(parsed).toEqual({ id: 's1', name: 'Plan', profileId: 'main', model: 'opus', permissionMode: 'plan', prompt: 'x', gate: false })
    expect(parsed && 'effort' in parsed).toBe(false)
  })

  it('never lets a value through that would read as a flag', () => {
    expect(parseStep({ id: 's1', model: '--dangerously-skip-permissions' })?.model).toBeUndefined()
    expect(parseStep({ id: 's1', model: '-p' })?.model).toBeUndefined()
    expect(parseStep({ id: 's1', permissionMode: 'bypassPermissions' })?.permissionMode).toBeUndefined()
    expect(parseStep({ id: 's1', effort: '--max' })?.effort).toBeUndefined()
    expect(isModel('claude-opus-5-5')).toBe(true)
    expect(isModel('opus[1m]')).toBe(true)
    expect(isModel('opus plan')).toBe(false)
    expect(isModel('')).toBe(false)
  })

  it('has a gate unless it is switched off, and refuses a step without an id', () => {
    expect(parseStep({ id: 's1' })).toEqual({ id: 's1', name: '', profileId: '', prompt: '', gate: true })
    expect(parseStep({ name: 'no id' })).toBeNull()
    expect(parseStep({ id: 'bad id' })).toBeNull()
    expect(parseStep('text')).toBeNull()
  })

  it('keeps fields a later version may have added', () => {
    expect(parseStep({ id: 's1', timeoutMinutes: 5 })).toMatchObject({ id: 's1', timeoutMinutes: 5 })
    expect(parseWorkflow({ id: 'w', name: 'N', steps: [], color: 'red' })).toMatchObject({ color: 'red' })
  })
})

describe('parseWorkflows', () => {
  it('reads a file and leaves out what it cannot use', () => {
    const list = parseWorkflows({
      version: 1,
      workflows: [{ id: 'a', name: 'A', steps: [{ id: 's1' }, { id: 's1' }, { nope: true }, { id: 's2' }] }, { id: 'a', name: 'twice' }, 'junk', { name: 'no id' }]
    })
    expect(list.map((w) => w.id)).toEqual(['a'])
    expect(list[0].steps.map((s) => s.id)).toEqual(['s1', 's2'])
  })

  it('gives an empty list for anything else', () => {
    expect(parseWorkflows(null)).toEqual([])
    expect(parseWorkflows([])).toEqual([])
    expect(parseWorkflows({ workflows: 'x' })).toEqual([])
  })
})

describe('prompt variables', () => {
  const values = {
    task: 'fix the login',
    folder: 'C:\\code\\app',
    previous: 'the plan',
    step: (name: string) => (name.toLowerCase() === 'plan' ? 'PLAN TEXT' : undefined)
  }

  it('fills in the task, the folder, the previous output and a named step', () => {
    expect(renderPrompt('{{task}} in {{folder}}: {{previous.output}} / {{steps.Plan.output}}', values)).toBe('fix the login in C:\\code\\app: the plan / PLAN TEXT')
  })

  it('allows spaces inside the braces and any case', () => {
    expect(renderPrompt('{{ Task }} {{STEPS.plan.OUTPUT}} {{ previous.output }}', values)).toBe('fix the login PLAN TEXT the plan')
  })

  it('reads text that came in through a variable only as text', () => {
    const tricky = { ...values, task: '{{previous.output}} and {{steps.Plan.output}}', previous: '{{task}}' }
    expect(renderPrompt('{{task}} | {{previous.output}}', tricky)).toBe('{{previous.output}} and {{steps.Plan.output}} | {{task}}')
  })

  it('leaves alone what is not a variable or has no value', () => {
    const text = 'Vue {{ user.name }}, Jinja {{ item|upper }}, {{steps.Nope.output}}, {{}} and { {task} }'
    expect(renderPrompt(text, values)).toBe(text)
    expect(renderPrompt('{{previous.output}}', { ...values, previous: undefined })).toBe('{{previous.output}}')
    // a replacement pattern in the value is not a pattern
    expect(renderPrompt('{{task}}', { ...values, task: "$& $1 $'" })).toBe("$& $1 $'")
  })

  it('lists what a prompt refers to', () => {
    expect(promptRefs('{{task}} {{folder}} {{previous.output}} {{steps.My Plan.output}} {{nope}} text')).toEqual([
      { kind: 'task' },
      { kind: 'folder' },
      { kind: 'previous' },
      { kind: 'step', name: 'My Plan' },
      { kind: 'unknown', text: 'nope' }
    ])
    expect(promptRefs('no variables, {single} braces')).toEqual([])
  })

  it('renames references to a step', () => {
    expect(renameStepRefs('a {{steps.plan.output}} b {{ steps.Plan.output }} c {{steps.Other.output}}', 'Plan', 'Design')).toBe(
      'a {{steps.Design.output}} b {{steps.Design.output}} c {{steps.Other.output}}'
    )
  })
})

describe('editing', () => {
  const three = flow(step('a', { name: 'Plan' }), step('b', { name: 'Build', prompt: 'Use {{steps.Plan.output}}' }), step('c', { name: 'Review', continues: 'a' }))

  it('renames a step together with the prompts that name it', () => {
    const renamed = renameStep(three, 'a', 'Design')
    expect(renamed.steps.map((s) => s.name)).toEqual(['Design', 'Build', 'Review'])
    expect(renamed.steps[1].prompt).toBe('Use {{steps.Design.output}}')
    expect(renameStep(three, 'a', 'Plan')).toBe(three)
    expect(renameStep(three, 'zz', 'X')).toBe(three)
  })

  it('does not touch the prompts while the name is being cleared or is shared', () => {
    expect(renameStep(three, 'a', '').steps[1].prompt).toBe('Use {{steps.Plan.output}}')
    const twins = flow(step('a', { name: 'Plan' }), step('b', { name: 'Plan' }), step('c', { name: 'C', prompt: '{{steps.Plan.output}}' }))
    expect(renameStep(twins, 'b', 'Other').steps[2].prompt).toBe('{{steps.Plan.output}}')
  })

  it('removes a step and the links to its conversation', () => {
    const left = removeStep(three, 'a')
    expect(left.steps.map((s) => s.id)).toEqual(['b', 'c'])
    expect('continues' in left.steps[1]).toBe(false)
  })

  it('moves a step', () => {
    expect(moveStep(three, 'c', 'a').steps.map((s) => s.id)).toEqual(['c', 'a', 'b'])
    expect(moveStep(three, 'a', null).steps.map((s) => s.id)).toEqual(['b', 'c', 'a'])
    expect(moveStep(three, 'a', 'b')).toBe(three)
  })

  it('finds a free step name', () => {
    expect(freeStepName(three.steps)).toBe('Step 4')
    expect(freeStepName([step('a', { name: 'step 2' })])).toBe('Step 3')
    expect(freeStepName([step('a', { name: 'Step 2' }), step('b', { name: 'Step 3' })])).toBe('Step 4')
  })

  it('follows a conversation back to the step that started it', () => {
    const chain = [step('a'), step('b'), step('c', { continues: 'a' }), step('d', { continues: 'c' }), step('e', { continues: 'zz' })]
    expect(chain.map((_, i) => threadOf(chain, i))).toEqual([0, 1, 0, 0, 4])
    // a link that points forward or at itself is no link
    const odd = [step('a', { continues: 'b' }), step('b', { continues: 'b' })]
    expect(odd.map((_, i) => threadOf(odd, i))).toEqual([0, 1])
  })

  it('builds the sample: plan and review on one account, implement on the other', () => {
    let n = 0
    const sample = sampleWorkflow(() => `id${++n}`, 'main', 'work')
    expect(sample.steps.map((s) => [s.name, s.profileId, s.model])).toEqual([
      ['Plan', 'main', 'fable'],
      ['Implement', 'work', 'opus'],
      ['Review', 'main', 'fable']
    ])
    expect(sample.steps[2].continues).toBe(sample.steps[0].id)
    // the review goes on with the plan's conversation, and leaves its plan mode
    expect(sample.steps.map((s) => s.permissionMode)).toEqual(['plan', undefined, 'manual'])
    expect(threadOf(sample.steps, 2)).toBe(0)
    expect(validateWorkflow(sample, accounts)).toEqual([])
    expect(parseWorkflow(JSON.parse(JSON.stringify(sample)))).toEqual(sample)
  })
})

describe('validateWorkflow', () => {
  it('accepts a plain chain', () => {
    expect(messages(flow(step('a'), step('b', { prompt: '{{previous.output}}' })))).toEqual([])
  })

  it('wants a name, steps, and for every step a name, an account and a prompt', () => {
    expect(messages({ id: 'w', name: ' ', steps: [] })).toEqual(['The workflow needs a name.', 'The workflow has no steps.'])
    const issues = validateWorkflow(flow(step('a', { name: '', profileId: '', prompt: '  ' })), accounts)
    expect(issues.map((i) => [i.stepId, i.level])).toEqual([
      ['a', 'error'],
      ['a', 'error'],
      ['a', 'error']
    ])
  })

  it('refuses names that cannot be told apart or cannot be written in a variable', () => {
    expect(messages(flow(step('a', { name: 'Plan' }), step('b', { name: 'plan ' })))).toEqual(['Another step is already named "plan".'])
    expect(messages(flow(step('a', { name: 'Plan.v2' })))[0]).toContain('A step name can have')
    expect(messages(flow(step('a', { name: 'Tasarım 2_son-hali' })))).toEqual([])
  })

  it('reports an account that is gone', () => {
    expect(messages(flow(step('a', { profileId: 'deleted' })))).toEqual(['The account of this step no longer exists. Choose another one.'])
  })

  it('lets a step continue only a step before it', () => {
    expect(messages(flow(step('a'), step('b', { continues: 'a' })))).toEqual([])
    expect(messages(flow(step('a', { continues: 'b' }), step('b')))[0]).toContain('comes later')
    expect(messages(flow(step('a', { continues: 'a' })))[0]).toContain('comes later')
    expect(messages(flow(step('a', { continues: 'gone' })))[0]).toContain('no longer exists')
  })

  it('warns when a step would stay in the plan mode of the conversation it continues', () => {
    const planned = step('a', { permissionMode: 'plan' })
    expect(messages(flow(planned, step('b', { continues: 'a' })), 'warning')[0]).toContain('leaves the conversation in plan mode')
    expect(messages(flow(planned, step('b', { continues: 'a' })), 'error')).toEqual([])
    expect(messages(flow(planned, step('b', { continues: 'a', permissionMode: 'auto' })))).toEqual([])
    expect(messages(flow(planned, step('b', { continues: 'a', permissionMode: 'plan' })))).toEqual([])
    // a conversation of its own starts in the account's mode
    expect(messages(flow(planned, step('b')))).toEqual([])
    // what counts is the step that had the conversation last
    expect(messages(flow(planned, step('b', { continues: 'a', permissionMode: 'auto' }), step('c', { continues: 'a' })))).toEqual([])
    expect(messages(flow(step('a'), step('b', { continues: 'a', permissionMode: 'plan' }), step('c', { continues: 'a' })), 'warning')).toHaveLength(1)
  })

  it('checks what the prompts refer to', () => {
    expect(messages(flow(step('a', { prompt: '{{previous.output}}' })))[0]).toContain('The first step has no previous step')
    expect(messages(flow(step('a', { prompt: '{{steps.b.output}}' }), step('b')))[0]).toContain('has not run yet')
    expect(messages(flow(step('a', { prompt: '{{steps.a.output}}' })))[0]).toContain('has not run yet')
    expect(messages(flow(step('a', { prompt: '{{steps.Nobody.output}}' })))[0]).toContain('no step has that name')
    expect(messages(flow(step('a'), step('b', { prompt: '{{steps.A.output}} {{task}} {{folder}}' })))).toEqual([])
  })

  it('only warns about braces that are not variables', () => {
    const issues = validateWorkflow(flow(step('a', { prompt: 'Render {{ user.name }}' })), accounts)
    expect(issues).toEqual([{ stepId: 'a', level: 'warning', message: '{{user.name}} is not a variable; it is sent as it is written.' }])
  })
})

describe('preflight', () => {
  const now = 1_000_000
  const profiles = [
    { id: 'main', name: 'Main', account: { loggedIn: true } },
    { id: 'work', name: 'Work', account: { loggedIn: false } }
  ]
  const both = flow(step('a'), step('b', { profileId: 'work' }), step('c', { profileId: 'work' }), step('d'))

  it('stops for an account that is not signed in, once', () => {
    expect(preflight(both, profiles, {}, now)).toEqual([{ stepId: 'b', level: 'error', message: '"Work" is not signed in.' }])
  })

  it('warns about a limit that looks used up, once per limit', () => {
    const usage = { main: { fiveHour: { usedPct: 100, resetsAt: now + 60_000 } } }
    const issues = preflight(flow(step('a', { name: 'Plan' }), step('d')), profiles, usage, now)
    expect(issues).toEqual([{ stepId: 'a', level: 'warning', message: '"Main" looks out of its 5-hour limit; step "Plan" may stop there.' }])
    // the limit has started over
    expect(preflight(flow(step('a')), profiles, usage, now + 120_000)).toEqual([])
  })

  it("looks at the weekly limit of the step's own model", () => {
    const usage = { main: { models: [{ name: 'Fable', usedPct: 100, resetsAt: now + 60_000 }] } }
    expect(preflight(flow(step('a', { model: 'opus' })), profiles, usage, now)).toEqual([])
    expect(preflight(flow(step('a', { model: 'fable' })), profiles, usage, now)[0].message).toContain('Fable weekly limit')
  })
})
