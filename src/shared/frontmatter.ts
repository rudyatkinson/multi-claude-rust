import { isMap, isScalar, isSeq, parseDocument, stringify, type YAMLMap } from 'yaml'

/**
 * The text of a Markdown file that opens with a YAML frontmatter between two `---` lines, as
 * Claude Code's skills, subagents, rules and memories are written.
 *
 * In an editor the whole text is the one thing that is true. A form is this text read
 * (`parseDoc`), and a change in the form is a change of the text (`setField`, `setBody`) that
 * touches only the lines of that one field: comments, the order of the fields and fields the
 * form does not know stay exactly as they were written.
 *
 * The text here always has plain line feeds and no byte order mark; what is on disk is the main
 * process's business.
 */

export interface DocParts {
  /** What stands between the two fences, with its last line break; null without a frontmatter. */
  frontmatter: string | null
  /** Where the frontmatter begins in the text: right after the opening fence. */
  start: number
  /** Where the body begins: right after the closing fence. */
  bodyStart: number
  /** The file opens a frontmatter on its first line and never closes it. */
  unclosed: boolean
}

const OPENING = /^---[ \t]*\n/
// the line break before it may be the opening fence's own: an empty frontmatter
const CLOSING = /\n---[ \t]*(?=\n|$)/

/** Claude Code only sees a frontmatter that opens on the very first line. */
export function splitDoc(text: string): DocParts {
  const open = OPENING.exec(text)
  if (!open) return { frontmatter: null, start: 0, bodyStart: 0, unclosed: /^---[ \t]*$/.test(text) }
  const start = open[0].length
  const close = CLOSING.exec(text.slice(start - 1))
  if (!close) return { frontmatter: null, start: 0, bodyStart: 0, unclosed: true }
  const end = start - 1 + close.index + 1
  const afterFence = start - 1 + close.index + close[0].length
  return { frontmatter: text.slice(start, end), start, bodyStart: Math.min(text.length, afterFence + 1), unclosed: false }
}

/** A value of the frontmatter, as it was written and as it reads. */
export interface FieldValue {
  /** The value as it stands in the file, without the key. */
  source: string
  /** Its text, when it is a plain piece of text. */
  string?: string
  /** Its entries, when it is a list of plain values. */
  items?: string[]
}

export interface ParsedDoc {
  parts: DocParts
  /** Why the fields cannot be read, when they cannot. Claude Code then loads none of them either. */
  error: { message: string; line?: number } | null
  /** The fields in the order they are written. */
  keys: string[]
  values: Record<string, FieldValue>
  /** The Markdown under the frontmatter, without the empty line that usually comes first. */
  body: string
  /** Lines of the whole file. */
  lines: number
}

interface Entry {
  key: string
  /** The lines of the field within the frontmatter: from its key up to the next field. */
  from: number
  to: number
  value: FieldValue
}

interface Frontmatter {
  error: ParsedDoc['error']
  entries: Entry[]
}

const lineStart = (text: string, offset: number): number => text.lastIndexOf('\n', offset - 1) + 1

/** The mistakes people make most, said without YAML's own words. */
const PLAIN_ERRORS: Record<string, string> = {
  BLOCK_AS_IMPLICIT_KEY: 'A value contains a colon followed by a space, and has to stand in quotes for that.',
  DUPLICATE_KEY: 'A field is written twice.',
  TAB_AS_INDENT: 'A line is indented with a tab; YAML wants spaces.'
}

/** Moves an end back over empty lines and comments that start a line: they go with what follows. */
function withoutTrailingNotes(text: string, from: number, to: number): number {
  let end = to
  while (end > from) {
    const start = lineStart(text, end - 1)
    const line = text.slice(start, end).trimEnd()
    if (line !== '' && !line.startsWith('#')) break
    // the field's own first line is never given away
    if (start <= from) break
    end = start
  }
  return end
}

function readFrontmatter(frontmatter: string): Frontmatter {
  const doc = parseDocument(frontmatter, { prettyErrors: true })
  const map = doc.contents
  const problem = doc.errors[0]
  let error: Frontmatter['error'] = problem
    ? // the first line of the message says it; the rest draws the place, which the line number gives
      { message: PLAIN_ERRORS[problem.code] ?? problem.message.split('\n')[0].replace(/ at line \d+, column \d+:?$/, ''), line: problem.linePos?.[0].line }
    : null
  if (!error && map !== null && !isMap(map)) error = { message: 'The frontmatter is not a list of "name: value" lines.' }
  if (!error && isMap(map) && map.flow) error = { message: 'The frontmatter is written in braces on one line, which the form cannot change.' }
  if (!isMap(map)) return { error, entries: [] }

  const pairs = (map as YAMLMap).items
  const starts = pairs.map((pair) => {
    const key = pair.key as { range?: [number, number, number] | null } | null
    return key?.range ? lineStart(frontmatter, key.range[0]) : -1
  })
  const entries: Entry[] = []
  pairs.forEach((pair, index) => {
    const from = starts[index]
    if (from < 0) return
    const next = starts.slice(index + 1).find((s) => s > from) ?? frontmatter.length
    const node = pair.value as unknown
    const range = (node as { range?: [number, number, number] | null } | null)?.range
    const value: FieldValue = { source: range ? frontmatter.slice(range[0], range[1]).trim() : '' }
    if (isScalar(node) && typeof node.value === 'string') value.string = node.value
    else if (isSeq(node) && node.items.every((item) => isScalar(item) && item.value !== null && typeof item.value !== 'object')) {
      value.items = node.items.map((item) => String((item as { value: unknown }).value))
    }
    const key = isScalar(pair.key) ? String(pair.key.value) : String(pair.key)
    entries.push({ key, from, to: withoutTrailingNotes(frontmatter, from, next), value })
  })
  return { error, entries }
}

// Typing in the body must not read the frontmatter again each time.
let last: { frontmatter: string; read: Frontmatter } | null = null

function frontmatterOf(text: string): Frontmatter {
  if (last?.frontmatter !== text) last = { frontmatter: text, read: readFrontmatter(text) }
  return last.read
}

/** The body as the editor shows it, and the empty line between the fence and it. */
function bodyOf(text: string, parts: DocParts): { gap: string; body: string } {
  const rest = text.slice(parts.bodyStart)
  if (parts.frontmatter === null) return { gap: '', body: rest }
  return rest.startsWith('\n') ? { gap: '\n', body: rest.slice(1) } : { gap: '', body: rest }
}

export function parseDoc(text: string): ParsedDoc {
  const parts = splitDoc(text)
  const read = parts.frontmatter === null ? { error: null, entries: [] } : frontmatterOf(parts.frontmatter)
  const error = parts.unclosed
    ? { message: 'The frontmatter that opens on the first line is never closed with a line of "---".', line: 1 }
    : // the frontmatter's first line is the file's second
      read.error && { ...read.error, line: read.error.line === undefined ? undefined : read.error.line + 1 }
  const values: Record<string, FieldValue> = {}
  // of a field that is written twice the first one counts here; the file is flagged anyway
  for (const entry of read.entries) values[entry.key] ??= entry.value
  return {
    parts,
    error,
    keys: [...new Set(read.entries.map((e) => e.key))],
    values,
    body: bodyOf(text, parts).body,
    lines: text === '' ? 0 : text.replace(/\n$/, '').split('\n').length
  }
}

/** `yes`, `on` and `1` are booleans to Claude Code as much as `true` is. Null for anything else. */
export function asBool(text: string): boolean | null {
  const lower = text.trim().toLowerCase()
  if (['true', 'yes', 'on', '1'].includes(lower)) return true
  if (['false', 'no', 'off', '0'].includes(lower)) return false
  return null
}

/** How a form shows a field: its text, the entries of a list joined by commas, or what is written. */
export function fieldText(parsed: ParsedDoc, key: string, list = false): string {
  const value = parsed.values[key]
  if (!value) return ''
  if (list && value.items) return value.items.join(', ')
  return value.string ?? value.source
}

/** A switch of the frontmatter; `fallback` when it is not set or not a yes or no. */
export function fieldBool(parsed: ParsedDoc, key: string, fallback: boolean): boolean {
  const value = parsed.values[key]
  return (value && asBool(value.string ?? value.source)) ?? fallback
}

/** The entries of a field that holds several names: a YAML list, or one text with commas. */
export function fieldItems(parsed: ParsedDoc, key: string): string[] {
  const value = parsed.values[key]
  if (!value) return []
  if (value.items) return value.items
  return splitItems(value.string ?? value.source)
}

/** Names typed with commas between them. */
export function splitItems(text: string): string[] {
  return text
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean)
}

/** The name the file gives itself, when it does. */
export function nameOf(parsed: ParsedDoc): string | null {
  const name = fieldText(parsed, 'name').trim()
  return name || null
}

/** The first line of the text that says something, without the marks of a heading. */
export function firstBodyLine(body: string): string {
  const line = body.split('\n').find((l) => l.trim() !== '') ?? ''
  return line.replace(/^#+\s*/, '').trim()
}

/**
 * What a field is set to. A text stands as typed where YAML reads it the same way and in quotes
 * otherwise; a number stands bare; several names become a YAML list. Null, an empty text and an
 * empty list take the field out.
 */
export type FieldInput = string | boolean | number | readonly string[] | null

const KEY = /^[A-Za-z_][A-Za-z0-9_-]*$/
const FIX_FIRST = 'The frontmatter has a problem. Fix it in the Source tab first.'

const sameItems = (a: readonly string[] | undefined, b: readonly string[]): boolean => a !== undefined && a.length === b.length && a.every((item, i) => item === b[i])

/**
 * The text with one field set or taken out. Only the lines of that field change. The result is
 * read again before it is given back: when the frontmatter would not say exactly what was asked,
 * with every other field as it was, nothing is changed and `failed` says so.
 */
export function setField(text: string, key: string, input: FieldInput): { text: string } | { failed: string } {
  if (!KEY.test(key)) return { failed: `"${key}" cannot be set from the form.` }
  if (typeof input === 'number' && !Number.isFinite(input)) return { failed: `"${key}" needs a number.` }
  const remove = input === null || input === '' || (typeof input === 'object' && input.length === 0)
  const parts = splitDoc(text)
  if (parts.unclosed) return { failed: FIX_FIRST }
  if (parts.frontmatter === null) {
    if (remove) return { text }
    // a file without a frontmatter gets one
    return setField(`---\n---\n${text === '' || text.startsWith('\n') ? '' : '\n'}${text}`, key, input)
  }
  const frontmatter = parts.frontmatter
  const before = frontmatterOf(frontmatter)
  if (before.error) return { failed: FIX_FIRST }
  const existing = before.entries.find((e) => e.key === key)
  if (remove && !existing) return { text }

  const candidates: string[] = []
  if (remove) candidates.push('')
  else if (typeof input === 'boolean' || typeof input === 'number') candidates.push(`${key}: ${input}\n`)
  else if (typeof input === 'string') {
    // as it was typed when YAML reads that the same way, else in quotes or as a block
    if (!input.includes('\n') && input === input.trim()) candidates.push(`${key}: ${input}\n`)
    candidates.push(stringify({ [key]: input }, { lineWidth: 0 }))
  } else candidates.push(stringify({ [key]: [...input] }, { lineWidth: 0 }))

  const from = existing ? existing.from : frontmatter.length
  const to = existing ? existing.to : frontmatter.length
  // a last line without a line break would run into what is added after it
  const lead = !existing && frontmatter !== '' && !frontmatter.endsWith('\n') ? '\n' : ''
  const others = before.entries.filter((e) => e.key !== key)
  for (const candidate of candidates) {
    const next = frontmatter.slice(0, from) + lead + candidate + frontmatter.slice(to)
    const after = readFrontmatter(next)
    if (after.error) continue
    const rest = after.entries.filter((e) => e.key !== key)
    const untouched = rest.length === others.length && rest.every((e, i) => e.key === others[i].key && next.slice(e.from, e.to) === frontmatter.slice(others[i].from, others[i].to))
    if (!untouched) continue
    const written = after.entries.filter((e) => e.key === key)
    if (remove ? written.length !== 0 : written.length !== 1) continue
    if (!remove) {
      const value = written[0].value
      const says =
        typeof input === 'boolean'
          ? asBool(value.string ?? value.source) === input
          : typeof input === 'number'
            ? value.string === undefined && Number(value.source) === input
            : typeof input === 'string'
              ? // a text must read as that text; only a list in brackets may stand as it was typed
                value.string === input || (value.string === undefined && value.items !== undefined && value.source === input)
              : sameItems(value.items, input)
      if (!says) continue
    }
    return { text: text.slice(0, parts.start) + next + text.slice(parts.start + frontmatter.length) }
  }
  return { failed: `This value cannot be written into the frontmatter from the form. Change "${key}" in the Source tab.` }
}

/** The text with another body under the same frontmatter. */
export function setBody(text: string, body: string): string {
  const parts = splitDoc(text)
  if (parts.frontmatter === null) return body
  const current = bodyOf(text, parts)
  // an empty line after the fence, as files have by habit, when a body appears where none was
  const gap = current.gap || (current.body === '' && body !== '' ? '\n' : '')
  const head = text.slice(0, parts.bodyStart)
  return (head.endsWith('\n') || body === '' ? head : `${head}\n`) + (body === '' ? current.gap : gap) + body
}

/**
 * How a form shows one field. `text` is one line and `long` several; `list` is names separated
 * by commas, kept as one text (a file may hold them as a YAML list, which then stays one until
 * the field is typed in); `names` is the same to the eye and always written as a YAML list;
 * `choice` is one of a few; `number` a whole number; `switch` a yes or no.
 */
export type FormFieldKind = 'text' | 'long' | 'list' | 'names' | 'choice' | 'number' | 'switch'

export interface FormField {
  key: string
  label: string
  kind: FormFieldKind
  hint: string
  placeholder?: string
  /** For `choice`; the empty value stands for "not set". */
  choices?: readonly { value: string; label: string }[]
  /** For `choice`: values Claude Code takes as well, though the form does not offer them. */
  accepts?: readonly string[]
}

/** The frontmatter of a new file: the given fields, each written as YAML reads it back. */
export function frontmatterBlock(fields: Record<string, unknown>): string {
  return `---\n${stringify(fields, { lineWidth: 0 })}---\n`
}
