import { create } from 'zustand'
import changelogText from '../CHANGELOG.md?raw'

/**
 * `CHANGELOG.md` as the What's new dialog shows it. The file follows Keep a Changelog, and only
 * what it uses is read: `## [version] - date` starts a release, `### Added` (Changed, Fixed …) a
 * part of it, `- ` an item; in the text of an item `**bold**` and `` `code` ``. Anything else
 * (the introduction above the first release, other markup) is left out or shown as it is written.
 */

export interface ChangelogPart {
  /** `Added`, `Changed`, `Fixed`, … */
  title: string
  items: string[]
}

export interface ChangelogRelease {
  version: string
  /** As written, e.g. `2026-11-08`; null for a release without one (`Unreleased`). */
  date: string | null
  parts: ChangelogPart[]
}

export type InlinePiece = { kind: 'text' | 'bold' | 'code'; text: string }

const RELEASE = /^##\s+\[?([^\]\s]+)\]?(?:\s+[-–—]\s+(.+?))?\s*$/
const PART = /^###\s+(.+?)\s*$/
const ITEM = /^\s*[-*]\s+(.*)$/

/** The releases of a Keep a Changelog file, in the order of the file (newest first). */
export function parseChangelog(markdown: string): ChangelogRelease[] {
  const releases: ChangelogRelease[] = []
  let part: ChangelogPart | null = null
  // an item continues on the lines that follow it, indented, until a blank line
  let item = false
  for (const raw of markdown.split(/\r?\n/)) {
    const line = raw.trimEnd()
    const release = RELEASE.exec(line)
    if (release) {
      releases.push({ version: release[1], date: release[2] ?? null, parts: [] })
      part = null
      item = false
      continue
    }
    const current = releases[releases.length - 1]
    if (!current) continue
    const heading = PART.exec(line)
    if (heading) {
      part = { title: heading[1], items: [] }
      current.parts.push(part)
      item = false
      continue
    }
    const entry = ITEM.exec(line)
    if (entry) {
      // items before any part heading get a part without a title
      if (!part) {
        part = { title: '', items: [] }
        current.parts.push(part)
      }
      part.items.push(entry[1].trim())
      item = true
      continue
    }
    if (item && part && line.trim() && /^\s/.test(line)) {
      part.items[part.items.length - 1] += ` ${line.trim()}`
      continue
    }
    item = false
  }
  return releases
}

/** The text of an item in pieces: plain, `**bold**` and `` `code` ``. Inside code nothing is markup. */
export function parseInline(text: string): InlinePiece[] {
  const pieces: InlinePiece[] = []
  const push = (kind: InlinePiece['kind'], value: string): void => {
    if (!value) return
    const last = pieces[pieces.length - 1]
    if (last && last.kind === kind && kind === 'text') last.text += value
    else pieces.push({ kind, text: value })
  }
  const token = /`([^`]+)`|\*\*(.+?)\*\*/g
  let at = 0
  for (let match = token.exec(text); match; match = token.exec(text)) {
    push('text', text.slice(at, match.index))
    if (match[1] !== undefined) push('code', match[1])
    else push('bold', match[2])
    at = match.index + match[0].length
  }
  push('text', text.slice(at))
  return pieces
}

/** The changelog that came with this build. */
export const CHANGELOG: readonly ChangelogRelease[] = parseChangelog(changelogText)

/** The newest release that has a version number; what the dot on the sidebar button is about. */
export const NEWEST_VERSION: string | null = CHANGELOG.find((r) => r.date !== null)?.version ?? CHANGELOG[0]?.version ?? null

const SEEN_KEY = 'mc.changelogSeen'

// a preference of this window only; without storage the dot shows until the dialog is opened
function readSeen(): string | null {
  try {
    return localStorage.getItem(SEEN_KEY)
  } catch {
    return null
  }
}

/** The newest version whose changes were shown once. */
export const useChangelogSeen = create<{ seen: string | null }>(() => ({ seen: readSeen() }))

export function markChangelogSeen(): void {
  if (!NEWEST_VERSION || useChangelogSeen.getState().seen === NEWEST_VERSION) return
  useChangelogSeen.setState({ seen: NEWEST_VERSION })
  try {
    localStorage.setItem(SEEN_KEY, NEWEST_VERSION)
  } catch {
    // shown again after a restart then
  }
}
