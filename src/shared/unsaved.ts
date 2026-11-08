/**
 * Drafts of the Library that are not saved, as the window reports them and a quit question
 * names them. Each item names one draft and starts a sentence: `The agent "reviewer"`.
 */

const MAX_ITEMS = 8
const MAX_CHARS = 120

/** What the window sent, cut down to a few short texts. */
export function cleanUnsaved(raw: unknown): string[] {
  if (!Array.isArray(raw)) return []
  return raw
    .filter((item): item is string => typeof item === 'string' && item.trim() !== '')
    .slice(0, MAX_ITEMS)
    .map((item) => item.replace(/\s+/g, ' ').slice(0, MAX_CHARS))
}

/** The paragraph of the quit question about them; empty when nothing is unsaved. */
export function unsavedNote(items: readonly string[]): string {
  if (items.length === 0) return ''
  if (items.length === 1) return `${items[0]} has changes that are not saved. They are lost when the app quits.`
  return `These have changes that are not saved, which are lost when the app quits:\n${items.map((item) => `  • ${item}`).join('\n')}`
}
