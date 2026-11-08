import { parse } from 'yaml'
import { foldText } from './conversations'
import { frontmatterBlock, splitDoc } from './frontmatter'
import { kebabProblem } from './skills'

/**
 * Claude Code's auto memory as the Library sees it: per account and project a folder
 * `<config>/projects/<project>/memory` with a `MEMORY.md` (the index every session reads) and one
 * Markdown file per thing that was written down.
 */

export const MEMORY_INDEX = 'MEMORY.md'

export const MEMORY_LIMITS = {
  /** What a session reads of MEMORY.md: this many lines … */
  indexLines: 200,
  /** … or this many bytes, whichever comes first. */
  indexBytes: 25_000,
  /** The name of a memory made here, which names its file. */
  name: 64,
  title: 120,
  description: 300,
  /** Hits a search hands back. */
  hits: 60
} as const

export const MEMORY_TYPES = ['user', 'feedback', 'project', 'reference'] as const
export type MemoryType = (typeof MEMORY_TYPES)[number]

/** A memory folder: what one account (or several that share it) remembers of one project. */
export interface MemoryFolder {
  /** Stands for the folder; the window never names a path itself. */
  id: string
  /** The accounts whose own folder this is. */
  profileIds: string[]
  /** The accounts that use it through sharing instead of their own. */
  sharedWith: string[]
  /** The memory folder itself, for showing. */
  dir: string
  /** Memories in it, MEMORY.md not counted. */
  count: number
  /** When a file in it last changed. */
  updatedAt: number
}

/** A project that has a memory in at least one account. */
export interface MemoryProject {
  /** Claude Code's name for the project: its path with a hyphen for every other character. */
  key: string
  /** The project's folder, when a conversation or a known folder tells it. */
  cwd: string | null
  folders: MemoryFolder[]
}

export interface MemoryListing {
  projects: MemoryProject[]
  /** Why no account can use the main account's memory right now, when that is so. */
  shareProblem: string | null
}

export interface MemoryRef {
  folderId: string
  /** The file within the memory folder. */
  file: string
}

export interface MemoryEntry {
  file: string
  /** What the index calls it, else the name in the file, else the file's name. */
  title: string
  /** The line of the index about it, else the description in the file. */
  description: string
  /** `user`, `feedback`, `project` or `reference`, when the file says. */
  type: string
  updatedAt: number
  size: number
  /** MEMORY.md has a line for it. */
  indexed: boolean
}

export interface MemoryIssue {
  level: 'warning' | 'info'
  message: string
}

/** One memory folder, opened. */
export interface MemoryView {
  folderId: string
  dir: string
  /** MEMORY.md is there. */
  hasIndex: boolean
  indexLines: number
  indexBytes: number
  entries: MemoryEntry[]
  issues: MemoryIssue[]
}

export interface MemoryDetail extends MemoryRef {
  text: string
  hash: string
  /** The file, for showing. */
  path: string
  readOnly?: string
}

export type MemorySaveResult = { saved: MemoryDetail } | { conflict: MemoryDetail }

export interface MemoryHit extends MemoryRef {
  /** The project's key, for showing where the hit is. */
  key: string
  cwd: string | null
  profileIds: string[]
  title: string
  /** The text around the first word that was found. */
  snippet: string
}

export const memoryKey = (ref: MemoryRef): string => `${ref.folderId}/${ref.file}`

// ---------- the index ----------

export interface IndexLine {
  /** The line in MEMORY.md, counted from 0. */
  line: number
  title: string
  file: string
  /** What stands after the link: when the memory is worth reading. */
  hook: string
}

// - [Title](file.md) — hook
const INDEX_LINE = /^\s*[-*+]\s+\[([^\]]*)\]\(([^)\s]+)\)\s*(?:[—–:-]+\s*)?(.*)$/

/** The file a link of the index points to, when it is a file of the memory folder itself. */
function targetOf(link: string): string | null {
  const file = link.replace(/^\.\//, '')
  return /^[^/\\:?#]+\.md$/i.test(file) ? file : null
}

/** The lines of MEMORY.md that point to a memory. */
export function parseIndex(text: string): IndexLine[] {
  const found: IndexLine[] = []
  text.split('\n').forEach((raw, line) => {
    const match = INDEX_LINE.exec(raw)
    const file = match && targetOf(match[2])
    if (match && file) found.push({ line, title: match[1].trim(), file, hook: match[3].trim() })
  })
  return found
}

const oneLine = (text: string): string => text.replace(/\s+/g, ' ').trim()

/** A line of the index as Claude Code writes them. */
export function indexLine(entry: { title: string; file: string; hook: string }): string {
  const title = oneLine(entry.title).replace(/[[\]]/g, '') || entry.file.replace(/\.md$/i, '')
  const hook = oneLine(entry.hook)
  return `- [${title}](${entry.file})${hook ? ` — ${hook}` : ''}`
}

/** MEMORY.md with a line for another memory: after the last such line, or at the end. */
export function addIndexLine(text: string, entry: { title: string; file: string; hook: string }): string {
  const added = indexLine(entry)
  const lines = text === '' ? [] : text.replace(/\n$/, '').split('\n')
  const last = parseIndex(text).at(-1)
  if (last) lines.splice(last.line + 1, 0, added)
  else {
    // a first entry under whatever the file says so far
    if (lines.length > 0 && lines[lines.length - 1].trim() !== '') lines.push('')
    lines.push(added)
  }
  return `${lines.join('\n')}\n`
}

/** MEMORY.md without the lines that point to a file. */
export function removeIndexLine(text: string, file: string): string {
  const gone = new Set(
    parseIndex(text)
      .filter((entry) => entry.file.toLowerCase() === file.toLowerCase())
      .map((entry) => entry.line)
  )
  if (gone.size === 0) return text
  return text
    .split('\n')
    .filter((_, line) => !gone.has(line))
    .join('\n')
}

/** A few names, and how many more there are. */
function some(names: readonly string[], shown = 5): string {
  return names.length <= shown ? names.join(', ') : `${names.slice(0, shown).join(', ')} and ${names.length - shown} more`
}

/** What a session does not get to read of MEMORY.md, and what the index and the folder disagree on. */
export function inspectMemory(index: { text: string; bytes: number } | null, files: readonly string[]): MemoryIssue[] {
  const issues: MemoryIssue[] = []
  if (index === null) {
    if (files.length > 0) issues.push({ level: 'warning', message: 'There is no MEMORY.md. A session only knows of memories that are listed there.' })
    return issues
  }
  const lines = index.text === '' ? 0 : index.text.replace(/\n$/, '').split('\n').length
  if (lines > MEMORY_LIMITS.indexLines) issues.push({ level: 'warning', message: `MEMORY.md has ${lines} lines. A session reads the first ${MEMORY_LIMITS.indexLines}; what comes after is not seen.` })
  else if (index.bytes > MEMORY_LIMITS.indexBytes) issues.push({ level: 'warning', message: `MEMORY.md has ${Math.round(index.bytes / 1000)} KB. A session reads the first 25 KB; what comes after is not seen.` })
  const listed = parseIndex(index.text)
  const has = (file: string): boolean => files.some((f) => f.toLowerCase() === file.toLowerCase())
  const missing = [...new Set(listed.filter((entry) => !has(entry.file)).map((entry) => entry.file))]
  if (missing.length > 0) issues.push({ level: 'warning', message: `MEMORY.md points to ${missing.length === 1 ? 'a file that is' : 'files that are'} not there: ${some(missing)}.` })
  const unlisted = files.filter((file) => !listed.some((entry) => entry.file.toLowerCase() === file.toLowerCase()))
  if (unlisted.length > 0) issues.push({ level: 'info', message: `Not in MEMORY.md, so a session does not know of ${unlisted.length === 1 ? 'it' : 'them'}: ${some(unlisted)}.` })
  return issues
}

// ---------- one memory ----------

/** What the frontmatter of a memory says about it; empty texts for what it does not say. */
export function memoryMeta(text: string): { name: string; description: string; type: string } {
  const none = { name: '', description: '', type: '' }
  const { frontmatter } = splitDoc(text)
  if (frontmatter === null) return none
  let data: unknown
  try {
    data = parse(frontmatter)
  } catch {
    return none
  }
  if (typeof data !== 'object' || data === null || Array.isArray(data)) return none
  const record = data as Record<string, unknown>
  const metadata = typeof record.metadata === 'object' && record.metadata !== null ? (record.metadata as Record<string, unknown>) : {}
  const str = (value: unknown): string => (typeof value === 'string' ? oneLine(value) : '')
  return { name: str(record.name), description: str(record.description), type: str(metadata.type) || str(record.type) }
}

/** Why a name cannot be given to a new memory, or null when it can. */
export function memoryNameProblem(name: string): string | null {
  if (!name) return 'Give the memory a name.'
  if (name.toLowerCase() === 'memory') return 'MEMORY.md is the index itself.'
  return kebabProblem(name, MEMORY_LIMITS.name)
}

/** The text of a memory that was just made, as Claude Code writes them. */
export function newMemoryText(input: { name: string; description: string; type: MemoryType; modified: string }): string {
  const head = frontmatterBlock({ name: input.name, description: oneLine(input.description), metadata: { node_type: 'memory', type: input.type, modified: input.modified } })
  return `${head}\nWrite here what Claude should remember.\n`
}

// ---------- searching ----------

/** The text around the first of the words, for a list of hits; null when a word is missing. */
export function memorySnippet(text: string, words: readonly string[], around = 70): string | null {
  const folded = foldText(text)
  if (words.length === 0 || !words.every((word) => folded.includes(word))) return null
  // folding keeps the length of ordinary text; a stray difference only moves the cut a little
  const at = Math.min(folded.indexOf(words[0]), text.length)
  const from = Math.max(0, at - around)
  const to = Math.min(text.length, at + words[0].length + around)
  return `${from > 0 ? '…' : ''}${oneLine(text.slice(from, to))}${to < text.length ? '…' : ''}`
}

// ---------- bringing one account's memories into another's folder ----------

export interface AdoptFile {
  name: string
  /** Stands for the file's content. */
  hash: string
}

export interface AdoptPlan {
  /** Files that go over, under the name they get there. */
  copy: { from: string; to: string }[]
  /** Files the target has already, with the same content. */
  same: string[]
}

/** A plan, and whether it was carried out. */
export interface AdoptResult extends AdoptPlan {
  /** The folder the files go to, for showing. */
  target: string
  /** False for a look at what would happen. */
  applied: boolean
}

/**
 * What it takes to bring the memories of one folder into another without losing anything on
 * either side: a file the target does not have goes over as it is; one it has with the same
 * content is left; one it has with other content goes over under a name that says where it came
 * from. MEMORY.md is never among them: the index of the target gets lines for what arrived.
 */
export function planAdopt(source: readonly AdoptFile[], target: readonly AdoptFile[], slug: string): AdoptPlan {
  const plan: AdoptPlan = { copy: [], same: [] }
  const taken = new Map(target.map((file) => [file.name.toLowerCase(), file.hash]))
  for (const file of source) {
    if (file.name.toLowerCase() === MEMORY_INDEX.toLowerCase()) continue
    const stem = file.name.replace(/\.md$/i, '')
    for (let n = 0; ; n++) {
      const name = n === 0 ? file.name : `${stem}.from-${slug}${n === 1 ? '' : `-${n}`}.md`
      const there = taken.get(name.toLowerCase())
      if (there === undefined) {
        plan.copy.push({ from: file.name, to: name })
        taken.set(name.toLowerCase(), file.hash)
        break
      }
      if (there === file.hash) {
        plan.same.push(file.name)
        break
      }
    }
  }
  return plan
}
