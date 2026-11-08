/** A past conversation of an account, as much of it as a list needs. */
export interface ConversationEntry {
  profileId: string
  /** Claude Code's id of the conversation. The same id can exist in two accounts after a move. */
  id: string
  /** The folder it took place in. */
  cwd: string
  /** Claude Code's title for it, else the start of its first message. */
  title: string
  firstPrompt?: string
  lastPrompt?: string
  /** When it last changed, epoch ms. */
  updatedAt: number
}

// The dotless i is a letter of its own, not an i with a mark, so it is the one letter that
// taking the marks off does not reach.
const DOTLESS_I = String.fromCharCode(0x131)

/**
 * Text as it is compared: lower case and without diacritics, so that "isik" finds "Işık" and
 * "cagri" finds "Çağrı", as people type when the keyboard layout is not at hand.
 */
export function foldText(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .toLowerCase()
    .split(DOTLESS_I)
    .join('i')
}

/**
 * The conversations in which every word of the query occurs: in the title, the first or the
 * last message, the folder or the account's name. Newest first. An empty query matches all.
 */
export function searchConversations(
  entries: readonly ConversationEntry[],
  query: string,
  accountNames: Readonly<Record<string, string>>
): ConversationEntry[] {
  const words = foldText(query).split(/\s+/).filter(Boolean)
  const matches = entries.filter((entry) => {
    if (words.length === 0) return true
    const haystack = foldText([entry.title, entry.firstPrompt, entry.lastPrompt, entry.cwd, accountNames[entry.profileId]].filter(Boolean).join('\n'))
    return words.every((word) => haystack.includes(word))
  })
  return matches.sort((a, b) => b.updatedAt - a.updatedAt)
}
