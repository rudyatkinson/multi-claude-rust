import type { UsageState, UsageWindow } from './types'

export interface UsageView {
  /** Whole percent to show; 0 once the window has reset. */
  pct: number
  tone: 'ok' | 'amber' | 'red'
  /** The reset time has passed, so this is a fresh window whatever was last reported. */
  reset: boolean
}

/** How a usage window is shown at `now`: on the account card, in the collapsed sidebar and in the tray menu alike. */
export function usageView(window: { usedPct: number; resetsAt?: number }, now: number): UsageView {
  const reset = window.resetsAt !== undefined && now >= window.resetsAt
  const pct = reset ? 0 : Math.round(window.usedPct)
  return { pct, tone: pct >= 85 ? 'red' : pct >= 60 ? 'amber' : 'ok', reset }
}

/** A limit that looks used up. */
export interface UsedUp {
  /** "5-hour limit", "weekly limit", "Fable weekly limit". */
  label: string
  resetsAt?: number
}

const full = (window: UsageWindow | undefined, now: number): boolean => window !== undefined && usageView(window, now).pct >= 100

/**
 * The limit of an account that is at 100% and has not reset, if any: the 5-hour one, the weekly
 * one, or the weekly limit of the model a session runs on (`model` is the session's model as
 * Claude Code names it, e.g. "Fable 5 (1M context)").
 *
 * It is a strong hint, not proof: an account with extra usage enabled keeps working at 100%.
 */
export function usedUp(usage: UsageState | undefined, now: number, model?: string): UsedUp | null {
  if (!usage) return null
  if (full(usage.fiveHour, now)) return { label: '5-hour limit', resetsAt: usage.fiveHour!.resetsAt }
  if (full(usage.sevenDay, now)) return { label: 'weekly limit', resetsAt: usage.sevenDay!.resetsAt }
  const running = model?.toLowerCase()
  const own = running ? usage.models?.find((m) => running.includes(m.name.toLowerCase()) && full(m, now)) : undefined
  return own ? { label: `${own.name} weekly limit`, resetsAt: own.resetsAt } : null
}

interface AccountRef {
  id: string
  account?: { loggedIn: boolean }
}

export interface Suggestion {
  profileId: string
  /** Its 5-hour use in whole percent, when known. */
  fiveHourPct?: number
}

/**
 * The account to continue on when `excludeId` has run out: logged in, not used up itself, and
 * with the most room in its 5-hour limit. Accounts without usage numbers come last. `model` is
 * the model the session runs on: an account whose weekly limit for that model is full is no help.
 */
export function suggestAccount(
  profiles: readonly AccountRef[],
  usage: Readonly<Record<string, UsageState>>,
  now: number,
  excludeId: string,
  model?: string
): Suggestion | null {
  const candidates = profiles
    .filter((p) => p.id !== excludeId && p.account?.loggedIn && !usedUp(usage[p.id], now, model))
    .map((p) => {
      const state = usage[p.id]
      const fiveHourPct = state?.fiveHour ? usageView(state.fiveHour, now).pct : undefined
      // a plan without a 5-hour limit (Ultra) reports weekly numbers only: no limit counts as all the room
      const noFiveHourLimit = fiveHourPct === undefined && Boolean(state?.sevenDay || state?.models?.length)
      return {
        profileId: p.id,
        fiveHourPct,
        fiveRank: noFiveHourLimit ? 0 : fiveHourPct,
        weekPct: state?.sevenDay ? usageView(state.sevenDay, now).pct : undefined
      }
    })
  // unknown counts as worse than any known number; the week breaks ties
  const rank = (pct: number | undefined): number => pct ?? 101
  candidates.sort((a, b) => rank(a.fiveRank) - rank(b.fiveRank) || rank(a.weekPct) - rank(b.weekPct))
  const best = candidates[0]
  return best ? { profileId: best.profileId, fiveHourPct: best.fiveHourPct } : null
}
