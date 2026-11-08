import { create } from 'zustand'
import type { SkillRoot } from '@shared/skills'
import type { ProfileView } from '@shared/types'
import { mc } from './api'
import { splitPath } from './format'

/**
 * The Library panel: what Claude Code reads from disk and the app lets you look after. Each
 * section keeps its own store; what they share is told here.
 */

export const LIBRARY_SECTIONS = [
  { id: 'skills', label: 'Skills', blurb: 'What Claude Code can do on request: one folder per skill, for every account that sees the folder or for one project.' },
  { id: 'agents', label: 'Agents', blurb: 'Subagents Claude hands tasks to. Each is one file: who it is, what it may use, and how it works.' },
  { id: 'memory', label: 'Memory', blurb: 'What Claude Code wrote down by itself about your projects and about how you work, per account and project.' },
  { id: 'instructions', label: 'Instructions', blurb: 'CLAUDE.md files and rules: what you tell every session, for an account or for a project.' }
] as const

export type LibrarySection = (typeof LIBRARY_SECTIONS)[number]['id']

export interface SectionHooks {
  /** Saves what is open in the section's editor (Ctrl+S). */
  save(): void
  /** Reads the section's lists again and looks whether the open file changed on disk. */
  refresh(): Promise<void>
}

const hooks = new Map<LibrarySection, SectionHooks>()

export function registerSection(id: LibrarySection, section: SectionHooks): void {
  hooks.set(id, section)
}

export const sectionHooks = (id: LibrarySection): SectionHooks | undefined => hooks.get(id)

/** What a folder of skills or agents is called in lists: whose personal ones, or which project. */
export function rootLabel(root: SkillRoot, profiles: readonly ProfileView[]): string {
  if (root.kind === 'project') return splitPath(root.folder ?? root.dir).name
  if (profiles.length <= 1) return 'Personal'
  if (root.profileIds.length === profiles.length) return 'Personal · all accounts'
  const names = root.profileIds.map((id) => profiles.find((p) => p.id === id)?.name).filter(Boolean)
  return `Personal · ${names.join(', ')}`
}

interface UnsavedState {
  /** By section: what has changes that are not saved, as a quit question names it. */
  items: Partial<Record<LibrarySection, string>>
}

export const useUnsaved = create<UnsavedState>(() => ({ items: {} }))

/** A section says what in it is unsaved ("The agent "reviewer""), or null for nothing. */
export function reportUnsaved(section: LibrarySection, item: string | null): void {
  const { items } = useUnsaved.getState()
  if ((items[section] ?? null) === item) return
  const next = { ...items }
  if (item === null) delete next[section]
  else next[section] = item
  useUnsaved.setState({ items: next })
}

// the main process asks before quitting while something is unsaved
useUnsaved.subscribe((state) => mc.send('ui:unsaved', { items: Object.values(state.items) }))
