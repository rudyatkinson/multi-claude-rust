import { tabMark } from '@shared/turn'
import type { SessionInfo } from '@shared/types'
import { suggestAccount, usedUp } from '@shared/usage'
import { clock, countdown } from '../format'
import { dismissLimit, setModal, transferSession, useStore } from '../store'

/**
 * Shown under a session whose account has run out: offers to carry the conversation over to the
 * account with the most room. Two things can say so. The session's hooks, when a usage limit cut
 * its last turn off; and the usage numbers, which are a hint only: an account at 100% may still
 * work.
 */
export function LimitBar({ session }: { session: SessionInfo }) {
  const usage = useStore((s) => s.usage)
  const profiles = useStore((s) => s.profiles)
  const now = useStore((s) => s.now)
  const status = useStore((s) => s.tabStatus[session.id])
  const dismissed = useStore((s) => s.dismissedLimits)

  if (session.kind !== 'claude' || session.status !== 'running') return null
  // while Claude is answering, the account evidently still works
  if (tabMark({ exited: false, turn: session.turn, busy: status?.busy === true, busySince: status?.busySince }).state === 'busy') return null
  const cut = session.turn?.kind === 'failed' && session.turn.limit ? session.turn : null
  const limit = usedUp(usage[session.profileId], now, session.model)
  if (!limit && !cut) return null
  // dismissed for this limit until it starts over, or for this one failed turn
  const key = limit ? `${session.id}:${limit.label}:${limit.resetsAt ?? 0}` : `${session.id}:cut:${cut?.at}`
  if (dismissed[key]) return null

  const name = profiles.find((p) => p.id === session.profileId)?.name ?? 'This account'
  const dismiss = (
    <button className="tab-close" title={limit ? 'Dismiss until the limit resets' : 'Dismiss'} onClick={() => dismissLimit(key)}>
      ×
    </button>
  )

  // Claude Code waited for the reset itself, and the computer slept through it
  if (cut?.wait === 'needsEnter' && !limit) {
    return (
      <div className="limit-bar">
        <span>{name}: the usage limit has reset. Press Enter in the tab.</span>
        <span className="spacer" />
        {dismiss}
      </div>
    )
  }

  const suggestion = suggestAccount(profiles, usage, now, session.profileId, session.model)
  const other = suggestion && profiles.find((p) => p.id === suggestion.profileId)
  const resets = limit?.resetsAt !== undefined ? ` · resets in ${countdown(limit.resetsAt - now)}` : ''
  const said = cut && limit
    ? `${name} reached its ${limit.label}: the last turn was cut off${resets}`
    : cut
      ? `${name}: the last turn ended with a rate limit error`
      : `${name}: ${limit?.label} looks used up${resets}`

  return (
    <div className="limit-bar">
      <span>
        {said}
        {cut?.wait === 'ended' && ' · Claude Code no longer waits for it'}
      </span>
      <span className="spacer" />
      {other && (
        <button className="btn primary" onClick={() => void transferSession(session.id, other.id)}>
          Continue on {other.name}
          {suggestion.fiveHourPct !== undefined && ` (5h ${suggestion.fiveHourPct}%)`}
        </button>
      )}
      <button className="btn" onClick={() => setModal({ type: 'transfer', sessionId: session.id })}>
        {other ? 'Other account…' : 'Choose account…'}
      </button>
      {dismiss}
    </div>
  )
}

/**
 * Shown under a session whose conversation was moved here because a usage limit cut it off, so
 * that a tab which changed its account while nobody looked says why.
 */
export function ArrivedBar({ session }: { session: SessionInfo }) {
  const profiles = useStore((s) => s.profiles)
  const dismissed = useStore((s) => s.dismissedLimits)
  const { arrived } = session
  const key = `${session.id}:arrived`
  if (!arrived || session.status !== 'running' || dismissed[key]) return null
  const from = profiles.find((p) => p.id === arrived.from)?.name ?? 'another account'

  return (
    <div className="note-bar">
      <span>
        {arrived.auto ? 'Moved here by itself' : 'Continued here'} from {from} at {clock(arrived.at)}: {from} reached its {arrived.limit}.{' '}
        {arrived.pickedUp ? 'Claude was asked to pick the task up.' : 'Send a message to go on.'}
      </span>
      <span className="spacer" />
      <button className="tab-close" title="Dismiss" onClick={() => dismissLimit(key)}>
        ×
      </button>
    </div>
  )
}
