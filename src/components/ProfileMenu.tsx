import { useLayoutEffect, useRef, useState } from 'react'
import type { ProfileView } from '@shared/types'
import { mc } from '../api'
import { ago } from '../format'
import { useDismiss } from '../hooks'
import { ask, attempt, mainProfileName, setActive, setModal, toast, useStore } from '../store'

const MARGIN = 6

interface Props {
  profile: ProfileView
  /** When the account's usage and status were last confirmed. */
  updatedAt: number | undefined
  onClose(): void
}

export async function syncProfile(profile: ProfileView): Promise<void> {
  let result = await attempt(mc.invoke('profiles:syncFromMain', profile.id, false))
  if (result?.needsConfirm) {
    const ok = await ask({
      title: 'Local changes',
      message: `${result.localChanged.join(', ')} was changed in "${profile.name}". The version from ${mainProfileName()} will replace it; the current one is kept as ".local-backup".`,
      confirmLabel: 'Overwrite'
    })
    if (!ok) return
    result = await attempt(mc.invoke('profiles:syncFromMain', profile.id, true))
  }
  if (!result) return
  for (const warning of result.warnings) toast('warn', warning)
  const done = [...result.linked, ...result.copied]
  toast('info', done.length > 0 ? `"${profile.name}" synced: ${done.join(', ')}` : `"${profile.name}" is already up to date.`)
}

export async function loginProfile(profile: ProfileView): Promise<void> {
  const session = await attempt(mc.invoke('profiles:login', profile.id))
  if (session) setActive(session.id)
}

async function logout(profile: ProfileView): Promise<void> {
  const ok = await ask({
    title: 'Log out',
    message: `This logs "${profile.name}" out and closes its open sessions.`,
    confirmLabel: 'Log out',
    danger: true
  })
  if (ok) await attempt(mc.invoke('profiles:logout', profile.id))
}

async function remove(profile: ProfileView): Promise<void> {
  const ok = await ask({
    title: 'Delete account',
    message: `"${profile.name}" will be deleted: it is logged out, its open sessions are closed, and its profile folder (including this account's conversation history) is moved to the Recycle Bin. The files of ${mainProfileName()} are not touched.`,
    confirmLabel: 'Delete',
    danger: true
  })
  if (ok) await attempt(mc.invoke('profiles:delete', profile.id))
}

/**
 * An account's menu. It opens under the `data-trigger` button next to it, flush with its right
 * edge, or above it where the window ends; placed on the window, so that a list that scrolls
 * (the cards, the other accounts) does not cut it off.
 */
export function ProfileMenu({ profile, updatedAt, onClose }: Props) {
  // ticks, so the "updated" line stays current while the menu is open
  const now = useStore((s) => s.now)
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)

  useDismiss(ref, onClose)

  useLayoutEffect(() => {
    const el = ref.current
    const trigger = el?.parentElement?.querySelector('[data-trigger]')?.getBoundingClientRect()
    if (!el || !trigger) return
    const { width, height } = el.getBoundingClientRect()
    const below = trigger.bottom + 4
    setPos({
      left: Math.max(MARGIN, Math.min(trigger.right - width, window.innerWidth - width - MARGIN)),
      top: below + height <= window.innerHeight - MARGIN ? below : Math.max(MARGIN, trigger.top - 4 - height)
    })
  }, [])

  const item = (label: string, action: () => unknown, danger = false) => (
    <button
      className={`menu-item${danger ? ' danger' : ''}`}
      onClick={() => {
        onClose()
        void action()
      }}
    >
      {label}
    </button>
  )

  const loggedIn = profile.account?.loggedIn === true
  const tokenAccount = profile.auth === 'token'
  return (
    <div className="menu profile-menu" ref={ref} style={pos ?? { left: 0, top: 0, visibility: 'hidden' }}>
      <div className="menu-head">
        {profile.account?.email && <div className="menu-email">{profile.account.email}</div>}
        {tokenAccount && <div className="menu-email">OAuth token</div>}
        <div>{updatedAt ? `Updated ${ago(now - updatedAt)}` : 'Not updated yet'}</div>
      </div>
      {!profile.isDefault && item(`Sync from ${mainProfileName()}`, () => syncProfile(profile))}
      {!profile.isDefault && item(`Copy MCP servers from ${mainProfileName()}…`, () => setModal({ type: 'mcp', profileId: profile.id }))}
      {tokenAccount && item('Replace token…', () => setModal({ type: 'replaceToken', profileId: profile.id }))}
      {!tokenAccount && item(loggedIn ? 'Log in again' : 'Log in', () => loginProfile(profile))}
      {!tokenAccount && loggedIn && item('Log out', () => logout(profile))}
      {item('Open profile folder', () => attempt(mc.invoke('profiles:openConfigDir', profile.id)))}
      {item('Name and color…', () => setModal({ type: 'editProfile', profileId: profile.id }))}
      {!profile.isDefault && item('Delete account…', () => remove(profile), true)}
    </div>
  )
}
