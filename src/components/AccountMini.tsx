import type { CSSProperties } from 'react'
import { hasFiveHourRow } from './UsageBar'
import type { ProfileView, UsageState, UsageWindow } from '@shared/types'
import { accountInitials, countdown, usageView } from '../format'
import { toggleSidebar, useStore } from '../store'

/** A usage limit as the small bars show it: its short name and its window, missing while there is no data. */
export type Limit = [label: string, window: UsageWindow | undefined]

/**
 * The limits an account shows, the same rows as its full card: 5h (when its plan has one), the
 * week, and one per model-specific weekly limit. None for an account that is not logged in, or
 * a token account without data.
 */
export function usageLimits(profile: ProfileView, usage: UsageState | undefined): Limit[] {
  const loggedIn = profile.account?.loggedIn === true
  const hasUsage = Boolean(usage?.fiveHour || usage?.sevenDay || usage?.models?.length)
  if (!loggedIn || (!hasUsage && profile.auth === 'token')) return []
  return [
    ...(hasFiveHourRow(usage) ? [['5h', usage?.fiveHour] as Limit] : []),
    ['Week', usage?.sevenDay] as Limit,
    ...(usage?.models ?? []).map((m): Limit => [m.name, m])
  ]
}

/** One line of text per limit: `5h: 42% · resets in 2h 10m`. */
export function limitLine([label, window]: Limit, now: number): string {
  if (!window) return `${label}: no data`
  const view = usageView(window, now)
  if (view.reset) return `${label}: reset`
  return `${label}: ${view.pct}%${window.resetsAt !== undefined ? ` · resets in ${countdown(window.resetsAt - now)}` : ''}`
}

/** One thin bar per usage limit; `labelled` puts each limit's short name in front of its bar, with its line as tooltip. */
export function MiniBars({ limits, now, labelled = false }: { limits: readonly Limit[]; now: number; labelled?: boolean }) {
  return (
    <>
      {limits.map((limit) => {
        const [label, window] = limit
        const view = window ? usageView(window, now) : null
        const fill = view && <span className={`fill ${view.tone}`} style={{ width: `${view.pct}%` }} />
        // a flex item either way: the bar's size holds
        return labelled ? (
          <span key={label} className="mini-limit" title={limitLine(limit, now)}>
            <span className="mini-limit-label">{label}</span>
            <span className="mini-bar">{fill}</span>
          </span>
        ) : (
          <span key={label} className="mini-bar">
            {fill}
          </span>
        )
      })}
    </>
  )
}

/** An account in the collapsed sidebar: its initial and one thin bar per usage limit. */
export function AccountMini({ profile }: { profile: ProfileView }) {
  const usage = useStore((s) => s.usage[profile.id])
  const initials = useStore((s) => accountInitials(profile.name, s.profiles.map((p) => p.name)))
  const now = useStore((s) => s.now)
  const running = useStore((s) => s.sessions.filter((x) => x.profileId === profile.id && x.status === 'running').length)

  const account = profile.account
  const loggedIn = account?.loggedIn === true
  const limits = usageLimits(profile, usage)
  const status = !account ? 'Checking…' : !loggedIn ? 'Not logged in' : null
  const tip = [profile.name, status, ...limits.map((limit) => limitLine(limit, now))].filter(Boolean).join('\n')

  return (
    <button
      className={`mini-account${loggedIn ? '' : ' off'}`}
      style={{ '--accent': profile.color } as CSSProperties}
      title={tip}
      onClick={toggleSidebar}
    >
      <span className={`mini-avatar${initials.length > 1 ? ' two' : ''}`}>
        {initials}
        {running > 0 && <span className="mini-count">{running}</span>}
      </span>
      <MiniBars limits={limits} now={now} />
    </button>
  )
}
