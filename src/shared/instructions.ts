import { fieldItems, parseDoc } from './frontmatter'
import { kebabProblem } from './skills'

/**
 * The instruction files Claude Code reads when a session starts, as the Library sees them: an
 * account's `CLAUDE.md` and `rules`, and a project's `CLAUDE.md`, `.claude/CLAUDE.md`,
 * `CLAUDE.local.md` and `.claude/rules`.
 */

export const INSTRUCTION_LIMITS = {
  /** Anthropic's advice for the length of one CLAUDE.md, in lines. */
  lines: 200,
  /** Name length of a rule made here. */
  ruleName: 64,
  /** How far `@path` imports are followed, as Claude Code follows them. */
  importHops: 4
} as const

/** Whose files: an account's (its config folder), or a project folder's. */
export interface InstructionScope {
  /** Stands for the folder; the window never names a path itself. */
  id: string
  kind: 'account' | 'project'
  /** Account: whose config folder it is. */
  profileId?: string
  /** The config folder, or the project folder; for showing. */
  dir: string
  /** Account: its CLAUDE.md is a copy that syncing from the main account overwrites. */
  syncedClaudeMd?: boolean
}

export type InstructionKind = 'claude' | 'local' | 'rule'

export interface InstructionFile {
  /** Within the scope's folder, with forward slashes: `CLAUDE.md`, `.claude/rules/tests.md`. */
  file: string
  kind: InstructionKind
  /** False for a file Claude Code would read if it were there; it can be made. */
  exists: boolean
  lines: number
  /** A rule's `paths`: it is read only when Claude works on a matching file. */
  paths: string[]
  /** The `@path` imports in the file. */
  imports: string[]
}

export interface InstructionRef {
  scopeId: string
  file: string
}

export interface InstructionDetail extends InstructionRef {
  text: string
  hash: string
  /** The file, for showing. */
  path: string
  readOnly?: string
}

export type InstructionSaveResult = { saved: InstructionDetail } | { conflict: InstructionDetail }

/** One file a session reads, in the order of reading. */
export interface LoadStep {
  /** The file on disk. */
  path: string
  /** What it is: `Account`, `Project`, `Rule`, `Local`, `Import`, `Memory`, `Managed policy`. */
  role: string
  /**
   * `start`: read when the session starts. `demand`: a rule with `paths`, read when a matching
   * file comes up. `missing`: an import that leads nowhere.
   */
  when: 'start' | 'demand' | 'missing'
  lines: number
  /** How deep it was imported: 0 for a file Claude Code finds by itself. */
  depth: number
  /** Something worth knowing about this step. */
  note?: string
}

export const instructionKey = (ref: InstructionRef): string => `${ref.scopeId}/${ref.file}`

/** The fixed files of a scope, in the order they are listed; rules come after them. */
export function fixedFiles(kind: InstructionScope['kind']): { file: string; kind: InstructionKind; about: string }[] {
  if (kind === 'account') return [{ file: 'CLAUDE.md', kind: 'claude', about: 'Read in every project this account works in' }]
  return [
    { file: 'CLAUDE.md', kind: 'claude', about: 'The project\'s instructions, shared through git' },
    { file: '.claude/CLAUDE.md', kind: 'claude', about: 'The same, kept in the .claude folder' },
    { file: 'CLAUDE.local.md', kind: 'local', about: 'Yours alone for this project; keep it out of git' }
  ]
}

/** The folder of a scope's rules, within the scope's folder. */
export const rulesDir = (kind: InstructionScope['kind']): string => (kind === 'account' ? 'rules' : '.claude/rules')

/** Why a name cannot be given to a new rule, or null when it can. */
export function ruleNameProblem(name: string): string | null {
  if (!name) return 'Give the rule a name.'
  return kebabProblem(name, INSTRUCTION_LIMITS.ruleName)
}

/** The text a file starts with when it is made here. */
export function starterText(kind: InstructionKind, scope: InstructionScope['kind'], title: string): string {
  if (kind === 'rule') return `# ${title}\n\nOne topic per rule. To have it read only for some files, put a frontmatter with a "paths" list of globs above this line.\n`
  if (kind === 'local') return '# Local notes\n\nInstructions for this project that stay on this machine. Keep this file out of git.\n'
  if (scope === 'account') return '# My instructions\n\nWhat Claude should know in every project: how you like to work, what to always or never do.\n'
  return `# ${title}\n\nWhat Claude should know about this project: how to build and test it, its conventions, what to stay away from.\n`
}

// ---------- reading a file ----------

const FENCE = /^(\s*)(```|~~~)/

/** The text without what stands in code: fenced blocks and `spans`, where an @ is just an @. */
function withoutCode(text: string): string {
  const kept: string[] = []
  let fence: string | null = null
  for (const line of text.split('\n')) {
    const mark = FENCE.exec(line)?.[2]
    if (fence) {
      if (mark === fence) fence = null
      continue
    }
    if (mark) {
      fence = mark
      continue
    }
    kept.push(line.replace(/`[^`\n]*`/g, ' '))
  }
  return kept.join('\n')
}

/**
 * The files a text pulls in with `@path`. Claude Code takes any `@word` outside code for one;
 * listed here are those that look like a path (a slash, a dot or a leading `~`), so that a
 * person's handle in a sentence is not shown as a missing file.
 */
export function findImports(text: string): string[] {
  const body = withoutCode(text)
  const found: string[] = []
  for (const match of body.matchAll(/(?:^|[\s(])@((?:[^\s\\)]|\\ )+)/g)) {
    const target = match[1].replace(/\\ /g, ' ').replace(/[.,;:!?]+$/, '')
    if (!/[./]/.test(target) && !target.startsWith('~')) continue
    if (/^[\w.+-]+@|^https?:/i.test(target)) continue
    if (!found.includes(target)) found.push(target)
  }
  return found
}

/** The globs of a rule's `paths`; empty for a rule that is always read. */
export function rulePaths(text: string): string[] {
  const parsed = parseDoc(text)
  return parsed.error ? [] : fieldItems(parsed, 'paths')
}

export const lineCount = (text: string): number => (text === '' ? 0 : text.replace(/\n$/, '').split('\n').length)
