/** What a session's terminal title says about it. */
export interface TabStatus {
  /** The conversation's topic as Claude Code named it; null until it has one. */
  topic: string | null
  /** Claude is working, as opposed to waiting for input. */
  busy: boolean
}

// Claude Code writes its terminal title as "<mark> <text>": ✳ while it waits, and ◐ / ◑
// alternating about once a second while it works.
const IDLE_MARK = '\u2733'
const BUSY_MARKS = ['\u25D0', '\u25D1']
// the title before a conversation has a topic
const DEFAULT_TEXT = 'claude code'

/**
 * Null for a title that does not carry Claude Code's mark. Those come from elsewhere: the bare
 * process name before Claude Code has drawn anything, or a program it runs in the same console.
 */
export function parseTerminalTitle(raw: string): TabStatus | null {
  const trimmed = raw.trim()
  const mark = trimmed.charAt(0)
  const busy = BUSY_MARKS.includes(mark)
  if (!busy && mark !== IDLE_MARK) return null
  // U+FE0F may follow the mark when the terminal title is written with emoji presentation
  const text = trimmed.slice(1).replace(/^\uFE0F/, '').trim()
  return { topic: text && text.toLowerCase() !== DEFAULT_TEXT ? text : null, busy }
}
