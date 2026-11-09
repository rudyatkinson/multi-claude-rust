import { useCallback, useState, type CSSProperties, type ReactNode } from 'react'
import type { ProfileView } from '@shared/types'
import { mc } from '../api'
import { ago, planLabel } from '../format'
import { attempt, mainProfileName, setModal, useStore } from '../store'
import { RefreshIcon, SyncIcon } from './icons'
import { ProfileMenu, loginProfile, syncProfile } from './ProfileMenu'
import { UsageRows } from './UsageBar'

/** An account's card: its name, plan and running sessions, its usage, and its buttons. `extra` comes last in its head. */
export function AccountCard({ profile, extra }: { profile: ProfileView; extra?: ReactNode }) {
  const usage = useStore((s) => s.usage[profile.id])
  const now = useStore((s) => s.now)
  const claudeFound = useStore((s) => s.claude.path !== null)
  const running = useStore((s) => s.sessions.filter((x) => x.profileId === profile.id && x.status === 'running').length)
  const [menuOpen, setMenuOpen] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const closeMenu = useCallback(() => setMenuOpen(false), [])

  const account = profile.account
  const loggedIn = account?.loggedIn === true
  // Claude Code reports neither plan nor email for a token account
  const tokenAccount = profile.auth === 'token'
  const plan = tokenAccount ? 'Token' : loggedIn ? planLabel(account) : null
  const override = !tokenAccount && loggedIn && account.authMethod && account.authMethod !== 'claude.ai' ? account.authMethod : null
  const hasUsage = Boolean(usage?.fiveHour || usage?.sevenDay || usage?.models?.length)
  const syncPending = profile.sync?.mainChanged ?? []
  const identity = tokenAccount ? 'Signed in with an OAuth token' : [account?.email, account?.orgName].filter(Boolean).join(' · ')
  // when the numbers on the card were last confirmed
  const updatedAt = usage?.observedAt ?? account?.checkedAt

  const refresh = async (): Promise<void> => {
    setRefreshing(true)
    await attempt(mc.invoke('profiles:refreshStatus', profile.id))
    setRefreshing(false)
  }

  return (
    <div className="card" style={{ '--accent': profile.color } as CSSProperties}>
      <div className="card-head">
        <span className="dot" />
        <span className="card-name" title={identity || profile.name}>
          {profile.name}
        </span>
        {override && (
          <span className="badge warn" title={`Sessions of this account authenticate with "${override}" instead of the subscription login.`}>
            !
          </span>
        )}
        {plan && <span className="badge">{plan}</span>}
        {running > 0 && (
          <span className="badge live" title="Running sessions">
            {running}
          </span>
        )}
        {syncPending.length > 0 && (
          <button
            className="icon-btn sync"
            title={`Changed in ${mainProfileName()}: ${syncPending.join(', ')}. Click to sync.`}
            onClick={() => void syncProfile(profile)}
          >
            <SyncIcon />
          </button>
        )}
        <button
          className={`icon-btn${refreshing ? ' spinning' : ''}`}
          disabled={refreshing || !claudeFound}
          title={`Refresh usage and account status${updatedAt ? ` (updated ${ago(now - updatedAt)})` : ''}`}
          onClick={() => void refresh()}
        >
          <RefreshIcon />
        </button>
        <button className="icon-btn" data-trigger title="Account menu" onClick={() => setMenuOpen(!menuOpen)}>
          ⋯
        </button>
        {extra}
      </div>

      {loggedIn && (hasUsage || !tokenAccount) ? (
        <UsageRows usage={usage} now={now} />
      ) : loggedIn ? (
        <div className="card-status">
          <span className="muted" title="Claude Code does not report plan usage for accounts that sign in with a token.">
            No usage data for token accounts
          </span>
        </div>
      ) : (
        <div className="card-status">
          <span className="muted">{!account ? 'Checking…' : tokenAccount ? 'Token unavailable' : 'Not logged in'}</span>
          {account && tokenAccount && (
            <button className="link-btn" onClick={() => setModal({ type: 'replaceToken', profileId: profile.id })}>
              Replace token
            </button>
          )}
          {account && !tokenAccount && (
            <button className="link-btn" disabled={!claudeFound} onClick={() => void loginProfile(profile)}>
              Log in
            </button>
          )}
        </div>
      )}

      {menuOpen && <ProfileMenu profile={profile} updatedAt={updatedAt} onClose={closeMenu} />}
    </div>
  )
}
