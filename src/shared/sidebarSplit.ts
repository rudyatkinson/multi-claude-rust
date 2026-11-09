/**
 * With the tabs in the sidebar (Settings → Window → Tab strip: Sidebar), the account cards and
 * the list of tabs below them share the sidebar's height. The line between them is dragged to
 * give one of them more room; dragged up far enough, the accounts fold to a single card. This
 * window keeps the choice in its local storage.
 */
export type AccountsSplit = { height: number } | { mode: 'one' }

/** What a new window starts with, and what a double-click on the line goes back to. */
export const DEFAULT_SPLIT: AccountsSplit = { height: 260 }

/** About one card: dragged shorter than this, the accounts fold to the card of the active tab. */
export const ONE_CARD_PX = 120

/** The room the tab list keeps at least while the line is dragged down. */
export const TABS_MIN_PX = 200

/** More than any screen is tall; a stored height beyond it is a broken one. */
const MAX_PX = 10_000

export const isOneCard = (split: AccountsSplit): split is { mode: 'one' } => 'mode' in split

/**
 * The height the accounts get for a wish of `h` pixels, out of the `space` they share with the
 * tab list: at least one card, and no more than leaves the list its room.
 */
export function clampAccountsHeight(h: number, space: number): number {
  const most = Math.max(ONE_CARD_PX, space - TABS_MIN_PX)
  return Math.round(Math.min(most, Math.max(ONE_CARD_PX, Number.isFinite(h) ? h : ONE_CARD_PX)))
}

/** Where the line ends up when it is let go with the accounts `h` pixels tall: shorter than one card folds them to one. */
export function snapSplit(h: number, space: number): AccountsSplit {
  return h < ONE_CARD_PX ? { mode: 'one' } : { height: clampAccountsHeight(h, space) }
}

/** The split as it was stored; anything that is not one is the default. */
export function parseSplit(raw: string | null): AccountsSplit {
  if (!raw) return DEFAULT_SPLIT
  let value: unknown
  try {
    value = JSON.parse(raw)
  } catch {
    return DEFAULT_SPLIT
  }
  if (typeof value !== 'object' || value === null) return DEFAULT_SPLIT
  const { mode, height } = value as { mode?: unknown; height?: unknown }
  if (mode === 'one') return { mode: 'one' }
  if (typeof height !== 'number' || !Number.isFinite(height) || height <= 0) return DEFAULT_SPLIT
  return { height: Math.round(Math.min(MAX_PX, Math.max(ONE_CARD_PX, height))) }
}

export function serializeSplit(split: AccountsSplit): string {
  return JSON.stringify(isOneCard(split) ? { mode: 'one' } : { height: Math.round(split.height) })
}
