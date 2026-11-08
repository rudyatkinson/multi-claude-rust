import type { Turn } from './turn'
import type { AuthKind, UsageState } from './types'
import { suggestAccount, usageView, usedUp } from './usage'

/** The limits within which a conversation is moved to another account without the user. */
export const SWITCH_RULES = {
  /** A failed turn older than this is not acted on any more. */
  freshMs: 2 * 60_000,
  /** How far back the moves of a conversation count: the length of a 5-hour limit. */
  memoryMs: 5 * 3600_000,
  /** How often a conversation is moved within that time. */
  maxMoves: 2,
  /** How long after a move the same conversation is not moved again. */
  gapMs: 10 * 60_000,
  /** How long an account that just took a conversation takes no other: its numbers do not show the first one yet. */
  arrivalGapMs: 2 * 60_000,
  /** How full the account to move to may be, in percent. */
  fiveHourPct: 80,
  weekPct: 94
} as const

/** A move the app made by itself. */
export interface SwitchMove {
  conversationId: string
  from: string
  to: string
  at: number
}

export interface SwitchInput {
  /** The setting is on, and Claude can be handed a first message. */
  enabled: boolean
  now: number
  session: {
    profileId: string
    model?: string
    conversationId?: string
    turn?: Turn
    moving?: boolean
    /** A step of a workflow run: the run decides what becomes of it. */
    workflow: boolean
  }
  /** When the failed turn that asked for this decision got there. */
  failedAt: number
  /** Claude was at work when the turn failed. */
  wasBusy: boolean
  /** Something was typed into the tab since. */
  typed: boolean
  profiles: readonly { id: string; auth?: AuthKind; account?: { loggedIn: boolean } }[]
  usage: Readonly<Record<string, UsageState>>
  /** The accounts whose usage was asked for anew for this decision, and whether numbers came. */
  probed: Readonly<Record<string, boolean>>
  moves: readonly SwitchMove[]
}

export type SwitchDecision =
  | { action: 'move'; to: string; limit: string }
  /** Ask for this account's usage, then decide again. */
  | { action: 'probe'; profileId: string }
  /** The limit is real, but the conversation stays: say so. */
  | { action: 'stay'; why: 'moved' | 'full'; limit: string }
  | { action: 'none'; why: string }

const none = (why: string): SwitchDecision => ({ action: 'none', why })

/** Whether an account's fresh numbers leave room for a conversation that arrives with all it holds. */
function hasRoom(state: UsageState | undefined, now: number, model: string | undefined): boolean {
  if (!state || usedUp(state, now, model)) return false
  const pct = (window: { usedPct: number; resetsAt?: number } | undefined): number => (window ? usageView(window, now).pct : 0)
  const running = model?.toLowerCase()
  const own = running ? state.models?.find((m) => running.includes(m.name.toLowerCase())) : undefined
  return pct(state.fiveHour) <= SWITCH_RULES.fiveHourPct && pct(state.sevenDay) <= SWITCH_RULES.weekPct && pct(own) <= SWITCH_RULES.weekPct
}

/**
 * What to do about a turn that a usage limit seems to have cut off, when the app may move the
 * conversation by itself. Two things have to say that the limit is real: the session's hooks, and
 * numbers that were fetched after it failed. A rate limit error alone can be the server slowing
 * everyone down; 100% alone can be an account that goes on with extra usage.
 */
export function decideSwitch(input: SwitchInput): SwitchDecision {
  const { session, now, usage, probed } = input
  if (!input.enabled) return none('switched off')
  const { turn } = session
  if (turn?.kind !== 'failed' || !turn.limit || turn.at !== input.failedAt) return none('the turn went on')
  if (!input.wasBusy) return none('Claude was not at work')
  if (session.workflow) return none('a workflow step')
  if (session.moving) return none('already on its way')
  if (input.typed) return none('the user is at it')
  const conversation = session.conversationId
  if (!conversation) return none('no conversation')
  if (now - turn.at > SWITCH_RULES.freshMs) return none('too long ago')

  const source = session.profileId
  if (!(source in probed)) return { action: 'probe', profileId: source }
  // a token account reports no usage, so nothing can confirm its limit
  if (!probed[source]) return none('no fresh numbers')
  const limit = usedUp(usage[source], now, session.model)
  if (!limit) return none('the numbers show no full limit')

  const mine = input.moves.filter((m) => m.conversationId === conversation && now - m.at < SWITCH_RULES.memoryMs)
  if (mine.length >= SWITCH_RULES.maxMoves || mine.some((m) => now - m.at < SWITCH_RULES.gapMs)) return { action: 'stay', why: 'moved', limit: limit.label }

  // not back to where it came from, and not onto an account that is still taking another one in
  const left = new Set(mine.map((m) => m.from))
  const taking = new Set(input.moves.filter((m) => now - m.at < SWITCH_RULES.arrivalGapMs).map((m) => m.to))
  const candidates = input.profiles.filter((p) => {
    if (p.id === source || !p.account?.loggedIn || p.auth === 'token' || left.has(p.id) || taking.has(p.id)) return false
    // numbers from before are good enough to choose whom to ask; the answer decides
    return p.id in probed ? probed[p.id] && hasRoom(usage[p.id], now, session.model) : !usage[p.id] || hasRoom(usage[p.id], now, session.model)
  })
  const best = suggestAccount(candidates, usage, now, source, session.model)
  if (!best) return { action: 'stay', why: 'full', limit: limit.label }
  if (!(best.profileId in probed)) return { action: 'probe', profileId: best.profileId }
  return { action: 'move', to: best.profileId, limit: limit.label }
}
