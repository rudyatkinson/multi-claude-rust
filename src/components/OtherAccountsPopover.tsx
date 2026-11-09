import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { createPortal } from 'react-dom'
import type { ProfileView } from '@shared/types'
import { mc } from '../api'
import { ago, planLabel } from '../format'
import { useDismiss } from '../hooks'
import { attempt, setModal, setPeekAccount, useStore } from '../store'
import { MiniBars, usageLimits } from './AccountMini'
import { RefreshIcon } from './icons'
import { loginProfile, ProfileMenu } from './ProfileMenu'

const MARGIN = 6
/** The button that opens the list, which toggles it itself. */
export const OTHERS_BUTTON = '.icon-btn.others'

/** One account in the list: a click shows it in the single card; refresh and the menu work as on its own card. */
function OtherAccount({ profile, onPick }: { profile: ProfileView; onPick(): void }) {
  const usage = useStore((s) => s.usage[profile.id])
  const now = useStore((s) => s.now)
  const claudeFound = useStore((s) => s.claude.path !== null)
  const running = useStore((s) => s.sessions.filter((x) => x.profileId === profile.id && x.status === 'running').length)
  const [menuOpen, setMenuOpen] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const closeMenu = useCallback(() => setMenuOpen(false), [])

  const account = profile.account
  const loggedIn = account?.loggedIn === true
  const tokenAccount = profile.auth === 'token'
  const plan = tokenAccount ? 'Token' : loggedIn ? planLabel(account.subscriptionType) : null
  const limits = usageLimits(profile, usage)
  const updatedAt = usage?.observedAt ?? account?.checkedAt
  const status = !account ? 'Checking…' : loggedIn ? (limits.length === 0 ? 'No usage data' : null) : tokenAccount ? 'Token unavailable' : 'Not logged in'


  const refresh = async (): Promise<void> => {
    setRefreshing(true)
    await attempt(mc.invoke('profiles:refreshStatus', profile.id))
    setRefreshing(false)
  }

  return (
    <div className={`other-account${loggedIn ? '' : ' off'}`} style={{ '--accent': profile.color } as CSSProperties}>
      <button className="other-pick" title={`Show ${profile.name} in the card`} onClick={onPick}>
        <span className="other-line">
          <span className="dot" />
          <span className="other-name">{profile.name}</span>
          {plan && <span className="badge">{plan}</span>}
          {running > 0 && (
            <span className="badge live" title="Running sessions">
              {running}
            </span>
          )}
        </span>
        {status ? (
          <span className="other-status muted">{status}</span>
        ) : (
          <span className="other-bars">
            <MiniBars limits={limits} now={now} labelled />
          </span>
        )}
      </button>
      {account && !loggedIn && (
        <button
          className="link-btn other-login"
          disabled={tokenAccount ? false : !claudeFound}
          onClick={() => (tokenAccount ? setModal({ type: 'replaceToken', profileId: profile.id }) : void loginProfile(profile))}
        >
          {tokenAccount ? 'Replace token' : 'Log in'}
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
      {menuOpen && <ProfileMenu profile={profile} updatedAt={updatedAt} onClose={closeMenu} />}
    </div>
  )
}

interface Props {
  /** The account the single card shows; the list has the rest. */
  shownId: string
  onClose(): void
}

/**
 * The accounts the single card does not show, opened from its "Other accounts" button. It opens
 * beside the button, over the terminals if need be, and stays inside the window.
 */
export function OtherAccountsPopover({ shownId, onClose }: Props) {
  const profiles = useStore((s) => s.profiles)
  // a dialog opened from an account's menu takes over
  const modal = useStore((s) => s.modal)
  const others = profiles.filter((p) => p.id !== shownId)
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  useDismiss(ref, onClose, OTHERS_BUTTON)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const anchor = document.querySelector(OTHERS_BUTTON)?.getBoundingClientRect()
    const { width, height } = el.getBoundingClientRect()
    const a = anchor ?? { right: MARGIN, top: MARGIN }
    setPos({
      left: Math.max(MARGIN, Math.min(a.right + 8, window.innerWidth - width - MARGIN)),
      top: Math.max(MARGIN, Math.min(a.top - 6, window.innerHeight - height - MARGIN))
    })
  }, [others.length])

  useEffect(() => {
    if (modal || others.length === 0) onClose()
  }, [modal, others.length, onClose])

  return createPortal(
    <div className="popover other-accounts" ref={ref} style={pos ?? { left: 0, top: 0, visibility: 'hidden' }} onContextMenu={(e) => e.preventDefault()}>
      <div className="popover-title">Other accounts</div>
      {others.map((profile) => (
        <OtherAccount
          key={profile.id}
          profile={profile}
          onPick={() => {
            setPeekAccount(profile.id)
            onClose()
          }}
        />
      ))}
    </div>,
    document.body
  )
}
