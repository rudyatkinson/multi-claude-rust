import { describe, expect, it } from 'vitest'
import { AGENT_FIELDS, agentClashes, agentKey, agentLoads, agentNameProblem, agentStem, inspectAgent, newAgentText, type AgentRoot } from '../src/shared/agents'
import { fieldText, nameOf, parseDoc } from '../src/shared/frontmatter'

const lines = (...l: string[]): string => l.join('\n') + '\n'

// as the user's own agents are written
const REAL = lines(
  '---',
  'name: arch-reviewer',
  'description: Runs the arch-review skill and returns the report. Report-only, never edits code.',
  'model: opus',
  'effort: xhigh',
  'skills:',
  '  - arch-review',
  'tools: Read, Grep, Glob, Bash',
  '---',
  '',
  'You review the architecture.'
)

const messages = (text: string): string[] => inspectAgent(parseDoc(text)).map((issue) => `${issue.level}: ${issue.message}`)

describe('inspectAgent', () => {
  it('has nothing to say about a real agent', () => {
    expect(messages(REAL)).toEqual([])
    expect(agentLoads(inspectAgent(parseDoc(REAL)))).toBe(true)
  })

  it('says that Claude Code does not load an agent without a name or a description', () => {
    const issues = inspectAgent(parseDoc(lines('---', 'model: opus', '---', '', 'Text.')))
    expect(issues.map((i) => i.level)).toEqual(['error', 'error'])
    expect(issues[0].message).toContain('No name.')
    expect(issues[1].message).toContain('No description.')
    expect(agentLoads(issues)).toBe(false)
    // a file without any frontmatter is the same case
    expect(messages('Just some notes.\n').filter((m) => m.startsWith('error'))).toHaveLength(2)
  })

  it('reports a frontmatter that cannot be read as the one error', () => {
    const issues = inspectAgent(parseDoc(lines('---', 'name: a', 'description: Does: this', '---', 'x')))
    expect(issues).toHaveLength(1)
    expect(issues[0]).toMatchObject({ level: 'error' })
    expect(issues[0].message).toMatch(/^The frontmatter cannot be read \(line 3\): .* Claude Code does not load the agent\.$/)
  })

  it('refuses a name Claude Code would not take', () => {
    const named = (name: string): string => lines('---', `name: "${name}"`, 'description: d', '---', 'x')
    expect(messages(named('plugin:agent'))).toEqual(['error: The name "plugin:agent" contains a colon, which Claude Code keeps for agents of plugins.'])
    expect(messages(named('a'.repeat(257)))[0]).toBe('error: The name has 257 characters; Claude Code takes at most 256.')
    // a name with capitals and spaces is unusual, and loads
    expect(messages(named('Code Reviewer'))).toEqual([])
  })

  it('names a value that is none of the choices, and takes the ones Claude Code also knows', () => {
    const withField = (line: string): string[] => messages(lines('---', 'name: a', 'description: d', line, '---', 'x'))
    expect(withField('permissionMode: yolo')).toEqual(['warning: "permissionMode: yolo" is not a value Claude Code knows. It knows default, acceptEdits, auto, dontAsk, bypassPermissions, plan.'])
    expect(withField('permissionMode: manual')).toEqual([])
    expect(withField('memory: project')).toEqual([])
    expect(withField('color: teal')[0]).toContain('"color: teal" is not a value Claude Code knows.')
    expect(withField('maxTurns: many')).toEqual(['warning: "maxTurns: many" is not a whole number above zero.'])
    expect(withField('maxTurns: 0')).toHaveLength(1)
    expect(withField('maxTurns: 25')).toEqual([])
    // a model may be any id
    expect(withField('model: claude-opus-5-5')).toEqual([])
  })

  it('warns about an agent that is told nothing', () => {
    expect(messages(lines('---', 'name: a', 'description: d', '---'))).toEqual(['warning: No system prompt. The text under the frontmatter is all the agent is told about its job.'])
  })
})

describe('agent names', () => {
  it('asks for kebab-case when the app gives the name', () => {
    expect(agentNameProblem('code-reviewer')).toBeNull()
    expect(agentNameProblem('')).toBe('Give the agent a name.')
    expect(agentNameProblem('Code Reviewer')).toBe('Use lower-case letters, digits and hyphens only.')
    expect(agentNameProblem('a'.repeat(65))).toBe('A name has at most 64 characters.')
    expect(agentNameProblem('nul')).toBe('"nul" is a name Windows keeps for a device.')
    // a skills folder keeps this one for itself; an agent may have it
    expect(agentNameProblem('synced')).toBeNull()
  })

  it('knows what a file is named after', () => {
    expect(agentStem('reviewer.md')).toBe('reviewer')
    expect(agentStem('team/qa/Tester.MD')).toBe('Tester')
  })

  it('writes a first text that loads', () => {
    const text = newAgentText('log-reader', 'Reads logs: errors first.')
    const parsed = parseDoc(text)
    expect(nameOf(parsed)).toBe('log-reader')
    expect(fieldText(parsed, 'description')).toBe('Reads logs: errors first.')
    expect(inspectAgent(parsed)).toEqual([])
    expect(inspectAgent(parseDoc(newAgentText('x', '')))).toEqual([])
  })

  it('offers every field once, and only choices that are spelled as Claude Code spells them', () => {
    const keys = AGENT_FIELDS.map((f) => f.key)
    expect(new Set(keys).size).toBe(keys.length)
    expect(AGENT_FIELDS.find((f) => f.key === 'permissionMode')?.choices?.map((c) => c.value)).toEqual(['', 'default', 'acceptEdits', 'auto', 'dontAsk', 'bypassPermissions', 'plan'])
    expect(AGENT_FIELDS.find((f) => f.key === 'skills')?.kind).toBe('names')
    expect(AGENT_FIELDS.find((f) => f.key === 'maxTurns')?.kind).toBe('number')
  })
})

describe('agentClashes', () => {
  const roots: AgentRoot[] = [
    { id: 'p-1', kind: 'personal', dir: 'C:\\u\\.claude\\agents', profileIds: ['main', 'elif'], exists: true },
    { id: 'j-1', kind: 'project', dir: 'C:\\proj\\.claude\\agents', profileIds: [], folder: 'C:\\proj', exists: true }
  ]
  const accounts = (): string => 'Bilal, Elif'
  const agent = (rootId: string, file: string, name: string): { rootId: string; file: string; name: string } => ({ rootId, file, name })

  it('says that the project agent is the one that is used', () => {
    const personal = agent('p-1', 'reviewer.md', 'reviewer')
    const project = agent('j-1', 'team/reviewer.md', 'reviewer')
    const clashes = agentClashes([personal, project, agent('p-1', 'other.md', 'other')], roots, accounts)
    expect(clashes.get(agentKey(personal))).toEqual([
      { level: 'warning', clash: true, message: 'The project C:\\proj has an agent of the same name ("team/reviewer.md"). In that project it is used instead of this one.' }
    ])
    expect(clashes.get(agentKey(project))).toEqual([{ level: 'info', clash: true, message: 'Takes the place of the personal agent of the same name (Bilal, Elif) in this project.' }])
    expect(clashes.size).toBe(2)
  })

  it('goes by the name in the file, not by the file', () => {
    const clashes = agentClashes([agent('p-1', 'reviewer.md', 'one'), agent('j-1', 'reviewer.md', 'two')], roots, accounts)
    expect(clashes.size).toBe(0)
  })

  it('warns on both of two agents in one folder that carry the same name', () => {
    const a = agent('p-1', 'a.md', 'same')
    const b = agent('p-1', 'sub/b.md', 'same')
    const clashes = agentClashes([a, b], roots, accounts)
    expect(clashes.get(agentKey(a))?.[0].message).toBe('"sub/b.md" in the same folder has the same name. Only one of the two is used.')
    expect(clashes.get(agentKey(b))?.[0].message).toBe('"a.md" in the same folder has the same name. Only one of the two is used.')
  })

  it('leaves files without a name out of it', () => {
    expect(agentClashes([agent('p-1', 'README.md', ''), agent('j-1', 'notes.md', '')], roots, accounts).size).toBe(0)
  })
})
