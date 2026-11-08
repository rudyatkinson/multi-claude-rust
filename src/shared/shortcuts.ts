/** What a key combination of the app does. Everything else belongs to the program in the terminal. */
export type ShortcutAction =
  | { type: 'new' }
  | { type: 'close' }
  | { type: 'next' }
  | { type: 'prev' }
  | { type: 'sidebar' }
  /** Go to a tab by position; `last` is Ctrl+9, as in browsers. */
  | { type: 'tab'; index: number | 'last' }
  | { type: 'search' }
  /** Terminal font size: one step up or down, or back to the default. */
  | { type: 'zoom'; step: 1 | -1 | 0 }
  /** Give the active tab an area of its own, to the right. */
  | { type: 'split' }
  /** Move the keyboard to the next or the previous area. */
  | { type: 'area'; step: 1 | -1 }
  /** Search the past conversations of all accounts. */
  | { type: 'history' }
  /** Put the active tab to sleep, or wake it. */
  | { type: 'sleep' }

/** The parts of a KeyboardEvent the table looks at. */
export interface KeyLike {
  key: string
  code: string
  ctrlKey: boolean
  shiftKey: boolean
  altKey: boolean
  metaKey: boolean
  isComposing?: boolean
}

const WITH_SHIFT: Record<string, ShortcutAction> = {
  t: { type: 'new' },
  w: { type: 'close' },
  b: { type: 'sidebar' },
  f: { type: 'search' },
  d: { type: 'split' },
  h: { type: 'history' },
  z: { type: 'sleep' }
}

/**
 * The app's own shortcut for a key press, or null when the key goes to the terminal.
 *
 * Only keys the terminal could not deliver to Claude Code anyway are taken, with one exception
 * shared with Windows Terminal: Ctrl+- zooms out. Claude Code's undo stays on Ctrl+Shift+- (Ctrl+_).
 */
export function matchShortcut(e: KeyLike): ShortcutAction | null {
  if (e.isComposing || e.metaKey) return null
  if (e.key === 'F6' && !e.ctrlKey && !e.altKey) return { type: 'area', step: e.shiftKey ? -1 : 1 }
  // AltGr is Ctrl+Alt on Windows: on a Turkish keyboard { [ ] } are AltGr+7, 8, 9, 0
  if (!e.ctrlKey || e.altKey) return null

  if (e.key === 'Tab') return { type: e.shiftKey ? 'prev' : 'next' }
  if (e.shiftKey) {
    // "+" needs Shift on most layouts ("=" on a Turkish one)
    if (e.key === '+' || e.key === '=') return { type: 'zoom', step: 1 }
    return WITH_SHIFT[e.key.toLowerCase()] ?? null
  }

  // by position, so that layouts with shifted digits (AZERTY) work as well
  const digit = /^Digit([1-9])$/.exec(e.code)
  if (digit) return { type: 'tab', index: digit[1] === '9' ? 'last' : Number(digit[1]) - 1 }
  // by character, since "=" and "-" sit on different keys from layout to layout
  if (e.key === '=' || e.key === '+') return { type: 'zoom', step: 1 }
  if (e.key === '-') return { type: 'zoom', step: -1 }
  if (e.key === '0') return { type: 'zoom', step: 0 }
  return null
}
