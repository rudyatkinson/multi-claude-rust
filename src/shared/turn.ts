import { sleepLabel } from './sleep'

/** The API errors a turn can end with, as Claude Code names them in its `StopFailure` hook. */
export const STOP_ERRORS = [
  'rate_limit',
  'overloaded',
  'authentication_failed',
  'oauth_org_not_allowed',
  'account_on_hold',
  'billing_error',
  'invalid_request',
  'model_not_found',
  'server_error',
  'max_output_tokens',
  'cloud_credential_error',
  'unknown'
] as const
export type StopError = (typeof STOP_ERRORS)[number]

export function isStopError(value: string): value is StopError {
  return (STOP_ERRORS as readonly string[]).includes(value)
}

// a server that throttles everyone says so in these words; it is a rate limit, but not the account's
const NOT_THE_LIMIT = /not your usage limit/i
// "You've hit your session limit · resets 3:45pm", also for the week, a model and a spend limit
const LIMIT_TEXT = /^(?:api error:\s*)?(?:you['’]ve hit your\b.{0,60}\blimit\b|usage limit reached)/i

/**
 * Whether a turn was cut off by a usage limit of the account, going by the error's name and by
 * the text Claude Code showed for it. A hint for what to offer; before anything is done by itself
 * the account's usage numbers have to say the same.
 */
export function limitCut(error: StopError, text: string): boolean {
  if (NOT_THE_LIMIT.test(text)) return false
  return error === 'rate_limit' || LIMIT_TEXT.test(text.trim())
}

const SENTENCES: Record<Exclude<StopError, 'unknown'>, string> = {
  rate_limit: 'The account reached a usage limit.',
  overloaded: 'The Claude servers are overloaded.',
  authentication_failed: 'The account is no longer signed in.',
  oauth_org_not_allowed: 'The organisation of the account is not allowed here.',
  account_on_hold: 'The account is on hold.',
  billing_error: 'The account has a billing problem.',
  invalid_request: 'The request was refused as invalid.',
  model_not_found: 'The model is not available to the account.',
  server_error: 'The Claude servers reported an error.',
  max_output_tokens: 'The answer was cut off at the output limit.',
  cloud_credential_error: 'The cloud credentials could not be loaded.'
}

/** What to say about a turn that ended with an API error. `text` is what Claude Code showed for it. */
export function stopSentence(error: StopError, text: string): string {
  if (limitCut(error, text)) return SENTENCES.rate_limit
  if (error === 'rate_limit') return 'The Claude servers are limiting requests for the moment.'
  if (error !== 'unknown') return SENTENCES[error]
  return text ? `Claude stopped with an error: ${text.slice(0, 200)}` : 'Claude stopped with an error.'
}

/**
 * What a session's turn has come to, as its hooks report it. `at` is when it got there. A session
 * without hooks (they are switched off, or the app could not listen) has none.
 */
export type Turn = { at: number } & (
  | { kind: 'working' }
  /** Claude finished its answer. `background`: work it started still runs and will wake it. */
  | { kind: 'done'; background?: true }
  /** Claude waits for an answer in the tab. */
  | { kind: 'permission' | 'question' | 'plan' }
  /**
   * The turn ended with an API error. `limit`: a usage limit of the account cut it off. `wait`:
   * how Claude Code's own wait for that limit ended, when it did not simply go on.
   */
  | { kind: 'failed'; error: StopError; text?: string; limit?: true; wait?: 'needsEnter' | 'ended' }
)

/** What can change a turn: the signals of the session's hooks, and the user typing into its tab. */
export type TurnEvent =
  | { kind: 'prompt' }
  | { kind: 'stop'; background: boolean }
  | { kind: 'plan' }
  | { kind: 'needsYou'; what: 'permission' | 'question' }
  | { kind: 'failure'; error: StopError; text: string }
  | { kind: 'limitWait'; stage: 'resumed' | 'needsEnter' | 'ended' }
  | { kind: 'typed' }

const asks = (turn: Turn | undefined): boolean => turn?.kind === 'permission' || turn?.kind === 'question' || turn?.kind === 'plan'

/** The turn after an event. Gives back the very same object when the event changes nothing. */
export function nextTurn(prev: Turn | undefined, event: TurnEvent, now: number): Turn | undefined {
  switch (event.kind) {
    case 'prompt':
      return prev?.kind === 'working' ? prev : { kind: 'working', at: now }
    case 'stop':
      return { kind: 'done', at: now, ...(event.background ? { background: true as const } : {}) }
    case 'plan':
      // a plan is reported twice, before the tool runs and when it asks for approval
      return prev?.kind === 'plan' ? prev : { kind: 'plan', at: now }
    case 'needsYou':
      // a prompt that goes unanswered is reported again a few seconds later, less exactly
      return asks(prev) ? prev : { kind: event.what, at: now }
    case 'failure':
      return {
        kind: 'failed',
        at: now,
        error: event.error,
        ...(event.text ? { text: event.text } : {}),
        ...(limitCut(event.error, event.text) ? { limit: true as const } : {})
      }
    case 'limitWait':
      if (event.stage === 'resumed') return { kind: 'working', at: now }
      // only a usage limit is waited for, whatever the failure was called
      if (prev?.kind === 'failed') return prev.limit && prev.wait === event.stage ? prev : { ...prev, limit: true, wait: event.stage }
      return { kind: 'failed', at: now, error: 'rate_limit', limit: true, wait: event.stage }
    case 'typed':
      // no hook says that a question was answered; what is typed there is taken for the answer
      return asks(prev) ? { kind: 'working', at: now } : prev
  }
}

/** Tells one state of a turn from another: what it is, when it got there, how a limit wait ended. */
export function turnKey(turn: Turn): string {
  return [turn.kind, turn.at, turn.kind === 'failed' ? (turn.wait ?? '') : ''].join(':')
}

/** Whether a turn has come to something the user should hear of however briefly Claude worked. */
export function callsForYou(turn: Turn | undefined): turn is Turn {
  return asks(turn) || turn?.kind === 'failed'
}

export type TabMark = 'busy' | 'idle' | 'ask' | 'failed' | 'limit' | 'ended' | 'asleep'

/**
 * The mark of a tab and what it means. The terminal title says whether Claude is working; the
 * turn says why it is not. Without a turn the title is all there is, as it was before tabs had
 * hooks. A sleeping tab says since when it sleeps, whatever its title said last.
 */
export function tabMark(o: { exited: boolean; asleep?: boolean; sleptAt?: number; turn?: Turn; busy: boolean; busySince?: number }): {
  state: TabMark
  label: string
} {
  if (o.asleep) return { state: 'asleep', label: sleepLabel(o.sleptAt) }
  if (o.exited) return { state: 'ended', label: 'Ended' }
  const { turn } = o
  const working = { state: 'busy' as const, label: 'Working' }
  if (!turn || turn.kind === 'working' || turn.kind === 'done') {
    if (o.busy) return working
    return { state: 'idle', label: turn?.kind === 'done' && turn.background ? 'Waiting for work it started in the background' : 'Waiting for input' }
  }
  // the title shows work that began after the turn got here, and no hook has told of it yet
  if (o.busy && o.busySince !== undefined && o.busySince > turn.at) return working
  switch (turn.kind) {
    case 'permission':
      return { state: 'ask', label: 'Asks for a permission' }
    case 'question':
      return { state: 'ask', label: 'Asks a question' }
    case 'plan':
      return { state: 'ask', label: 'Presents a plan and waits for your approval' }
    case 'failed':
      if (!turn.limit) return { state: 'failed', label: stopSentence(turn.error, turn.text ?? '') }
      if (turn.wait === 'needsEnter') return { state: 'limit', label: 'The usage limit has reset. Press Enter in the tab.' }
      return { state: 'limit', label: turn.wait === 'ended' ? 'Stopped at a usage limit. Claude Code no longer waits for it.' : 'Stopped at a usage limit' }
  }
}

/** Whether Claude was at work in a turn, or about to be woken by work it had started. */
export function inProgress(turn: Turn | undefined): boolean {
  return turn?.kind === 'working' || asks(turn) || (turn?.kind === 'done' && turn.background === true)
}
