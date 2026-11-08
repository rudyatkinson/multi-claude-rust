import type { CSSProperties } from 'react'
import type { ProfileView, UsageWindow } from '@shared/types'
import { accountInitials, countdown, usageView } from '../format'
import { toggleSidebar, useStore } from '../store'

/** An account in the collapsed sidebar: its initial and one thin bar per usage limit. */
export function AccountMini({ profile }: { profile: ProfileView }) {
  const usage = useStore((s) => s.usage[profile.id])
  const initials = useStore((s) => accountInitials(profile.name, s.profiles.map((p) => p.name)))
  const now = useStore((s) => s.now)
  const running = useStore((s) => s.sessions.filter((x) => x.profileId === profile.id && x.status === 'running').length)

  const account = profile.account
  const loggedIn = account?.loggedIn === true
  const hasUsage = Boolean(usage?.fiveHour || usage?.sevenDay || usage?.models?.length)
  // same rows as the full card; a token account without data has none
  const limits: [string, UsageWindow | undefined][] =
    loggedIn && (hasUsage || profile.auth !== 'token')
      ? [['5h', usage?.fiveHour], ['Week', usage?.sevenDay], ...(usage?.models ?? []).map((m): [string, UsageWindow] => [m.name, m])]
      : []

  const lines = limits.map(([label, window]) => {
    if (!window) return `${label}: no data`
    const view = usageView(window, now)
    if (view.reset) return `${label}: reset`
    return `${label}: ${view.pct}%${window.resetsAt !== undefined ? ` · resets in ${countdown(window.resetsAt - now)}` : ''}`
  })
  const status = !account ? 'Checking…' : !loggedIn ? 'Not logged in' : null
  const tip = [profile.name, status, ...lines].filter(Boolean).join('\n')

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
      {limits.map(([label, window]) => {
        const view = window ? usageView(window, now) : null
        return (
          <span key={label} className="mini-bar">
            {view && <span className={`fill ${view.tone}`} style={{ width: `${view.pct}%` }} />}
          </span>
        )
      })}
    </button>
  )
}
