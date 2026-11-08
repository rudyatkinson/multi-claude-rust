import { fieldText, frontmatterBlock, nameOf, type FormField, type ParsedDoc } from './frontmatter'
import { kebabProblem, type SkillRoot } from './skills'
import { EFFORT_LEVELS } from './workflows'

/**
 * Claude Code subagents as the Library sees them: a Markdown file with a frontmatter in an
 * account's `agents` folder ("personal") or in a project's `.claude/agents` ("project"), at any
 * depth. What names an agent is the `name` in its file, not the name of the file. Agents that
 * are built in or come from plugins are not its business.
 */

export const AGENT_LIMITS = {
  /** Name length Claude Code allows. */
  name: 256,
  /** Name length for an agent made or renamed here, where the name names the file as well. */
  newName: 64,
  /** Lines of a system prompt after which the list says so. */
  lines: 500
} as const

/** A folder that holds subagents: the same thing as a folder that holds skills. */
export type AgentRoot = SkillRoot

export interface AgentRef {
  rootId: string
  /** The file within the agents folder, with forward slashes: `reviewer.md`, `team/qa.md`. */
  file: string
}

export interface AgentIssue {
  level: 'error' | 'warning' | 'info'
  message: string
  /** It is about another agent of the same name, which only the whole list can tell. */
  clash?: true
}

export interface AgentSummary extends AgentRef {
  /** The name in the file; empty when the file gives none. */
  name: string
  description: string
  model: string
  effort: string
  /** Claude Code can load the file as an agent. */
  loads: boolean
  issues: AgentIssue[]
}

export interface AgentDetail extends AgentRef {
  /** The whole file, with plain line feeds. Empty when it cannot be edited here. */
  text: string
  /** Stands for the bytes on disk; a save names the one it started from. */
  hash: string
  /** The file, for showing. */
  path: string
  /** Why the file cannot be edited in the app, when it cannot. */
  readOnly?: string
}

export interface AgentListing {
  roots: AgentRoot[]
  agents: AgentSummary[]
}

export type AgentSaveResult = { saved: AgentDetail } | { conflict: AgentDetail }

/** Where an agent is after it was renamed, copied or moved. */
export interface AgentMoved {
  ref: AgentRef
  /** What did not go fully as asked, though the agent is where it should be. */
  warning?: string
}

export const agentKey = (ref: AgentRef): string => `${ref.rootId}/${ref.file}`

/** The file's name without its folders and its ending: what an agent is usually named after. */
export function agentStem(file: string): string {
  const base = file.slice(file.lastIndexOf('/') + 1)
  return base.replace(/\.md$/i, '')
}

/** Why a name cannot be given to an agent here, or null when it can. */
export function agentNameProblem(name: string): string | null {
  if (!name) return 'Give the agent a name.'
  return kebabProblem(name, AGENT_LIMITS.newName)
}

// ---------- the fields of an agent file ----------

const PERMISSION_MODES = ['default', 'acceptEdits', 'auto', 'dontAsk', 'bypassPermissions', 'plan'] as const
const COLORS = ['red', 'blue', 'green', 'yellow', 'purple', 'orange', 'pink', 'cyan'] as const

const choices = (unset: string, values: readonly string[]): { value: string; label: string }[] => [{ value: '', label: unset }, ...values.map((value) => ({ value, label: value }))]

/** The fields the form shows, in order. Everything else in a file is left to the Source tab. */
export const AGENT_FIELDS: readonly FormField[] = [
  {
    key: 'description',
    label: 'Description',
    kind: 'long',
    hint: 'What the agent is for. Claude goes by this text when it decides whom to hand a task to.',
    placeholder: 'Reviews a diff for … Use after …'
  },
  { key: 'tools', label: 'Tools', kind: 'list', hint: 'The tools the agent may use. Empty: every tool of the session.', placeholder: 'Read, Grep, Glob, Bash' },
  { key: 'disallowedTools', label: 'Disallowed tools', kind: 'list', hint: 'Tools taken away from what the agent would have.', placeholder: 'Write, Edit' },
  {
    key: 'model',
    label: 'Model',
    kind: 'text',
    hint: 'An alias (sonnet, opus, haiku, fable), a full model id, or inherit for the model of the session. Empty: Claude Code chooses.',
    placeholder: 'inherit'
  },
  { key: 'effort', label: 'Effort', kind: 'choice', hint: 'The effort the agent works with.', choices: choices('As the session', EFFORT_LEVELS) },
  {
    key: 'permissionMode',
    label: 'Permission mode',
    kind: 'choice',
    hint: 'How the agent asks before it acts.',
    choices: choices('As the session', PERMISSION_MODES),
    accepts: ['manual']
  },
  {
    key: 'memory',
    label: 'Memory',
    kind: 'choice',
    hint: 'A memory of its own that the agent keeps from one session to the next. user: in the account, for every project. project: in the project, shared with it. local: in the project, kept out of git.',
    choices: choices('None', ['user', 'project', 'local'])
  },
  {
    key: 'isolation',
    label: 'Isolation',
    kind: 'choice',
    hint: 'In a worktree the agent works on a copy of the repository of its own.',
    choices: choices('None', ['worktree'])
  },
  { key: 'color', label: 'Color', kind: 'choice', hint: 'The colour the agent shows in.', choices: choices('Default', COLORS) },
  { key: 'maxTurns', label: 'Max turns', kind: 'number', hint: 'The agent stops after this many turns.', placeholder: 'No limit' },
  { key: 'skills', label: 'Skills', kind: 'names', hint: 'Skills whose text the agent starts with.', placeholder: 'arch-review, unity-cli' },
  { key: 'background', label: 'Always in the background', kind: 'switch', hint: 'The agent runs as a background task whenever it is used.' }
]

/** Keys the form or the manager handles itself, so they are not listed as "also in the file". */
export const AGENT_HANDLED_KEYS: readonly string[] = ['name', ...AGENT_FIELDS.map((f) => f.key)]

// ---------- what is wrong with an agent ----------

/** The name of an agent as Claude Code takes it: at most 256 characters, without a colon. */
function takenNameProblem(name: string): string | null {
  if (name.length > AGENT_LIMITS.name) return `The name has ${name.length} characters; Claude Code takes at most ${AGENT_LIMITS.name}.`
  if (name.includes(':')) return `The name "${name}" contains a colon, which Claude Code keeps for agents of plugins.`
  return null
}

/** What is wrong with an agent file, or worth knowing. An `error` keeps Claude Code from loading it. */
export function inspectAgent(parsed: ParsedDoc): AgentIssue[] {
  const issues: AgentIssue[] = []
  const add = (level: AgentIssue['level'], message: string): void => void issues.push({ level, message })

  if (parsed.error) {
    const where = parsed.error.line === undefined ? '' : ` (line ${parsed.error.line})`
    const said = /[.!?]$/.test(parsed.error.message) ? parsed.error.message : `${parsed.error.message}.`
    add('error', `The frontmatter cannot be read${where}: ${said} Claude Code does not load the agent.`)
    return issues
  }

  const name = nameOf(parsed)
  if (name === null) add('error', 'No name. Claude Code knows an agent by the "name" in its file, not by the name of the file, and does not load one without it.')
  else {
    const problem = takenNameProblem(name)
    if (problem) add('error', problem)
  }
  if (!fieldText(parsed, 'description').trim()) add('error', 'No description. Claude Code does not load an agent without one.')

  for (const field of AGENT_FIELDS) {
    const value = fieldText(parsed, field.key).trim()
    if (!value) continue
    if (field.kind === 'choice' && !field.choices?.some((c) => c.value === value) && !field.accepts?.includes(value)) {
      const known = (field.choices ?? []).map((c) => c.value).filter(Boolean)
      add('warning', `"${field.key}: ${value}" is not a value Claude Code knows. It knows ${known.join(', ')}.`)
    }
    if (field.kind === 'number' && !/^[1-9]\d*$/.test(value)) add('warning', `"${field.key}: ${value}" is not a whole number above zero.`)
  }

  if (parsed.body.trim() === '') add('warning', 'No system prompt. The text under the frontmatter is all the agent is told about its job.')
  else if (parsed.lines > AGENT_LIMITS.lines) add('info', `The file has ${parsed.lines} lines, all of which the agent reads every time it starts.`)
  return issues
}

/** Whether Claude Code can load the file, as far as its own text tells. */
export const agentLoads = (issues: readonly AgentIssue[]): boolean => !issues.some((issue) => issue.level === 'error' && !issue.clash)

// ---------- names that get in each other's way ----------

interface Named extends AgentRef {
  name: string
}

/**
 * Notes about agents that carry the same name. Within one folder only one of them is used; of a
 * project's agent and a personal one Claude Code takes the project's (the other way round than
 * with skills). `accountsOf` names the accounts of a personal root, for the message.
 */
export function agentClashes(agents: readonly Named[], roots: readonly AgentRoot[], accountsOf: (root: AgentRoot) => string): Map<string, AgentIssue[]> {
  const result = new Map<string, AgentIssue[]>()
  const add = (agent: Named, level: AgentIssue['level'], message: string): void => {
    const key = agentKey(agent)
    result.set(key, [...(result.get(key) ?? []), { level, message, clash: true }])
  }
  const rootOf = new Map(roots.map((r) => [r.id, r]))
  const named = agents.filter((agent) => agent.name !== '')

  for (const agent of named) {
    const twin = named.find((other) => other !== agent && other.rootId === agent.rootId && other.name === agent.name)
    if (twin) add(agent, 'warning', `"${twin.file}" in the same folder has the same name. Only one of the two is used.`)

    const root = rootOf.get(agent.rootId)
    if (root?.kind === 'personal') {
      const winner = named.find((other) => rootOf.get(other.rootId)?.kind === 'project' && other.name === agent.name)
      const folder = winner && rootOf.get(winner.rootId)?.folder
      if (winner) add(agent, 'warning', `The project${folder ? ` ${folder}` : ''} has an agent of the same name ("${winner.file}"). In that project it is used instead of this one.`)
    } else if (root?.kind === 'project') {
      const loser = named.find((other) => rootOf.get(other.rootId)?.kind === 'personal' && other.name === agent.name)
      if (loser) {
        const accounts = accountsOf(rootOf.get(loser.rootId)!)
        add(agent, 'info', `Takes the place of the personal agent of the same name${accounts ? ` (${accounts})` : ''} in this project.`)
      }
    }
  }
  return result
}

// ---------- new agents ----------

/** The text of an agent that was just made. */
export function newAgentText(name: string, description: string): string {
  const head = frontmatterBlock({ name, description: description.trim() || 'What this agent is for, and when Claude should hand work to it.' })
  return `${head}\nDescribe here how this agent works: what it looks at, what it may change, and what it hands back when it is done.\n`
}
