import { foldText } from './conversations'
import type { FormField } from './frontmatter'
import { EFFORT_LEVELS } from './workflows'

/**
 * Claude Code skills as the skill manager sees them: a folder with a SKILL.md in an account's
 * `skills` folder ("personal") or in a project's `.claude/skills` ("project"). Skills that come
 * from plugins or are synced from claude.ai are not its business.
 */

export const SKILL_LIMITS = {
  /** Name length the Agent Skills format allows. */
  name: 64,
  /** Description length claude.ai accepts on upload. */
  description: 1024,
  /** What Claude Code keeps of description plus when_to_use in its listing. */
  listing: 1536,
  compatibility: 500,
  /** Anthropic's advice for the length of SKILL.md, in lines. */
  lines: 500,
  /** A SKILL.md larger than this is not opened in the editor. */
  fileBytes: 1_000_000,
  /** Length of a request to Claude about a skill. */
  request: 20_000
} as const

export type SkillRootKind = 'personal' | 'project'

/** A folder that holds skills. */
export interface SkillRoot {
  id: string
  kind: SkillRootKind
  /** The skills folder itself, for showing. */
  dir: string
  /** Personal: the accounts whose skills folder this is (several when they share it). */
  profileIds: string[]
  /** Project: the folder whose `.claude/skills` this is. */
  folder?: string
  /** The skills folder is there. A project the user added may not have one yet. */
  exists: boolean
  /** Project: the user added the folder to the list by hand. */
  pinned?: boolean
}

export interface SkillRef {
  rootId: string
  /** The name of the skill's folder. */
  dirName: string
}

export interface SkillIssue {
  level: 'error' | 'warning' | 'info'
  /** `claude`: it matters to Claude Code. `upload`: only when the skill is uploaded to claude.ai. */
  scope: 'claude' | 'upload'
  message: string
  /** It is about another skill of the same name, which only the whole list can tell. */
  clash?: true
}

export interface SkillSummary extends SkillRef {
  /** The name in the file, or the folder's name when the file gives none. */
  name: string
  description: string
  /** Claude may pick the skill up by itself. */
  modelInvocable: boolean
  /** The skill shows in the / menu and runs as /name. */
  userInvocable: boolean
  /** The folder is a link to a folder somewhere else. */
  linked: boolean
  /** What Claude Code would stumble over. Upload rules are not counted here. */
  issues: SkillIssue[]
}

export interface SkillFile {
  /** Relative to the skill's folder, with forward slashes. */
  path: string
  size: number
}

export interface SkillDetail extends SkillRef {
  /** The whole SKILL.md, with plain line feeds. Empty when it cannot be edited here. */
  text: string
  /** Stands for the bytes on disk; a save names the one it started from. */
  hash: string
  files: SkillFile[]
  /** There are more files than are listed. */
  moreFiles: boolean
  /** The skill's folder, for showing. */
  path: string
  linked: boolean
  /** Why the file cannot be edited in the app, when it cannot. */
  readOnly?: string
}

export interface SkillListing {
  roots: SkillRoot[]
  skills: SkillSummary[]
}

/** What an archive holds, before it is brought in. */
export interface ImportPreview {
  /** Names this archive for `skills:import`; the window never sees its path. */
  token: string
  /** The name the archive suggests. */
  name: string
  description: string
  files: SkillFile[]
  moreFiles: boolean
  issues: SkillIssue[]
}

export type SaveResult = { saved: SkillDetail } | { conflict: SkillDetail }

/** Where a skill is after it was renamed, copied or moved. */
export interface SkillMoved {
  ref: SkillRef
  /** What did not go fully as asked, though the skill is where it should be. */
  warning?: string
}

export const skillKey = (ref: SkillRef): string => `${ref.rootId}/${ref.dirName}`

// ---------- names ----------

/** Names Windows keeps for devices, whatever the extension. */
const DEVICE_NAME = /^(con|prn|aux|nul|com[0-9]|lpt[0-9])$/i

/** Folder names Claude Code keeps for itself inside a skills folder. */
export function isReservedSkillDir(name: string): boolean {
  const lower = name.toLowerCase()
  return lower === 'synced' || lower === 'anthropic-skills' || name.startsWith('.')
}

/**
 * Why a name cannot be given to a new skill, or null when it can. The rule is the one of the
 * Agent Skills format, which also keeps the folder name harmless on every file system.
 */
export function newNameProblem(name: string): string | null {
  if (!name) return 'Give the skill a name.'
  return kebabProblem(name, SKILL_LIMITS.name) ?? (isReservedSkillDir(name) ? `"${name}" is a name Claude Code keeps for itself.` : null)
}

/** Why a text that is not empty cannot be a name in kebab-case that also names a file, or null. */
export function kebabProblem(name: string, max: number): string | null {
  if (name.length > max) return `A name has at most ${max} characters.`
  if (!/^[a-z0-9-]+$/.test(name)) return 'Use lower-case letters, digits and hyphens only.'
  if (name.startsWith('-') || name.endsWith('-') || name.includes('--')) return 'A name does not start or end with a hyphen, and has no two hyphens in a row.'
  if (DEVICE_NAME.test(name)) return `"${name}" is a name Windows keeps for a device.`
  return null
}

/** A name made from any text: what is typed as a title becomes a name that passes. */
export function slugifySkillName(text: string): string {
  return foldText(text)
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, SKILL_LIMITS.name)
    .replace(/-+$/, '')
}

/** A name that is free among `taken`: the wish itself, or with a number after it. */
export function freeSkillName(wish: string, taken: readonly string[]): string {
  const used = new Set(taken.map((n) => n.toLowerCase()))
  if (!used.has(wish.toLowerCase())) return wish
  for (let n = 2; ; n++) {
    const suffix = `-${n}`
    const name = wish.slice(0, SKILL_LIMITS.name - suffix.length).replace(/-+$/, '') + suffix
    if (!used.has(name.toLowerCase())) return name
  }
}

// ---------- the fields of a SKILL.md ----------

export { asBool } from './frontmatter'

/** The two switches: who may start the skill. */
export const SKILL_SWITCHES = {
  /** `true` keeps Claude from using the skill by itself. */
  hideFromModel: 'disable-model-invocation',
  /** `false` takes the skill out of the / menu. */
  userInvocable: 'user-invocable'
} as const

export type SkillField = FormField

/** The fields the form shows, in order. Everything else in a file is left to the Source tab. */
export const SKILL_FIELDS: readonly SkillField[] = [
  {
    key: 'description',
    label: 'Description',
    kind: 'long',
    hint: 'What the skill does and when to use it. Claude goes by this text when it decides whether the skill fits.',
    placeholder: 'Reviews a pull request for … Use when …'
  },
  {
    key: 'when_to_use',
    label: 'When to use',
    kind: 'long',
    hint: 'More about when the skill applies. Claude reads it together with the description.'
  },
  { key: 'argument-hint', label: 'Argument hint', kind: 'text', hint: 'Shown after the name in the / menu.', placeholder: '[issue-number]' },
  {
    key: 'allowed-tools',
    label: 'Allowed tools',
    kind: 'list',
    hint: 'Tools Claude may use without asking while the skill is active.',
    placeholder: 'Read, Grep, Bash(git *)'
  },
  { key: 'disallowed-tools', label: 'Disallowed tools', kind: 'list', hint: 'Tools that are taken away while the skill is active.', placeholder: 'Write, Edit' },
  { key: 'model', label: 'Model', kind: 'text', hint: 'The model while the skill is active: an alias, a full id, or inherit.', placeholder: 'inherit' },
  {
    key: 'effort',
    label: 'Effort',
    kind: 'choice',
    hint: 'The effort while the skill is active.',
    choices: [{ value: '', label: 'As the session' }, ...EFFORT_LEVELS.map((level) => ({ value: level as string, label: level as string }))]
  },
  {
    key: 'context',
    label: 'Runs',
    kind: 'choice',
    hint: 'In a subagent the skill works without the conversation so far and hands back its result.',
    choices: [
      { value: '', label: 'In the conversation' },
      { value: 'fork', label: 'In a subagent' }
    ]
  },
  { key: 'agent', label: 'Subagent type', kind: 'text', hint: 'Which kind of agent runs the skill when it runs in a subagent.', placeholder: 'general-purpose' },
  { key: 'paths', label: 'Paths', kind: 'list', hint: 'Globs that limit where Claude picks the skill up by itself.', placeholder: 'src/**/*.ts, docs/**' }
]

/** The keys claude.ai accepts in an uploaded skill; any other key makes it refuse the upload. */
export const UPLOAD_KEYS: readonly string[] = ['name', 'description', 'license', 'compatibility', 'metadata', 'allowed-tools']

/** Keys the form or the manager handles itself, so they are not listed as "also in the file". */
export const HANDLED_KEYS: readonly string[] = ['name', ...SKILL_FIELDS.map((f) => f.key), ...Object.values(SKILL_SWITCHES)]

// ---------- names that get in each other's way ----------

interface Named extends SkillRef {
  name: string
}

/**
 * Warnings about skills that answer to the same name. Claude Code runs a skill by its folder
 * name and by the name in its file, and lets a personal skill win over a project one.
 * `accountsOf` names the accounts of a personal root, for the message.
 */
export function nameClashes(skills: readonly Named[], roots: readonly SkillRoot[], accountsOf: (root: SkillRoot) => string): Map<string, SkillIssue[]> {
  const result = new Map<string, SkillIssue[]>()
  const add = (skill: Named, message: string): void => {
    const key = skillKey(skill)
    result.set(key, [...(result.get(key) ?? []), { level: 'warning', scope: 'claude', message, clash: true }])
  }
  const namesOf = (skill: Named): string[] => [...new Set([skill.dirName.toLowerCase(), skill.name.toLowerCase()])]
  const rootOf = new Map(roots.map((r) => [r.id, r]))

  for (const skill of skills) {
    const mine = namesOf(skill)
    const twin = skills.find((other) => other !== skill && other.rootId === skill.rootId && namesOf(other).some((n) => mine.includes(n)))
    if (twin) add(skill, `"${twin.dirName}" in the same folder answers to the same name. Only one of the two is used.`)

    if (rootOf.get(skill.rootId)?.kind !== 'project') continue
    const personal = skills.find((other) => rootOf.get(other.rootId)?.kind === 'personal' && namesOf(other).some((n) => mine.includes(n)))
    if (personal) {
      const accounts = accountsOf(rootOf.get(personal.rootId)!)
      add(skill, `The personal skill "${personal.dirName}"${accounts ? ` (${accounts})` : ''} has the same name and is used instead of this one.`)
    }
  }
  return result
}
