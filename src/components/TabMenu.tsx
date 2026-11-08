import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import type { SessionInfo } from '@shared/types'
import { pinnedAccount } from '@shared/folders'
import { groupOf, groupsOf } from '@shared/layout'
import {
  closeSession,
  closeSessions,
  closeTabMenu,
  duplicateSession,
  mergeAreas,
  revealFolder,
  setFolderAccount,
  setModal,
  splitTab,
  startRename,
  useStore
} from '../store'

const MARGIN = 6

/** The menu of a right-clicked tab. It opens at the pointer and stays inside the window. */
export function TabMenu() {
  const menu = useStore((s) => s.tabMenu)
  const session = useStore((s) => s.sessions.find((x) => x.id === s.tabMenu?.id))
  if (!menu || !session) return null
  // a new position is a new menu: it measures itself again
  return <Menu key={`${menu.id}:${menu.x}:${menu.y}`} session={session} x={menu.x} y={menu.y} />
}

function Menu({ session, x, y }: { session: SessionInfo; x: number; y: number }) {
  const layout = useStore((s) => s.layout)
  const platform = useStore((s) => s.platform)
  const account = useStore((s) => s.profiles.find((p) => p.id === session.profileId))
  // the folder always opens with this tab's account
  const pinned = useStore((s) => pinnedAccount(s.folderAccounts, session.cwd, s.platform, s.profiles) === session.profileId)
  const otherAccounts = useStore((s) => s.profiles.some((p) => p.id !== session.profileId && p.account?.loggedIn))
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { width, height } = el.getBoundingClientRect()
    setPos({
      left: Math.max(MARGIN, Math.min(x, window.innerWidth - width - MARGIN)),
      top: Math.max(MARGIN, Math.min(y, window.innerHeight - height - MARGIN))
    })
    // the keyboard must leave the terminal, or Escape would go to Claude Code
    el.focus()
  }, [x, y])

  // "others" and "to the right" are about the tabs of this tab's own area
  const group = groupOf(layout, session.id)
  const tabs = group?.tabs ?? []
  const others = tabs.filter((id) => id !== session.id)
  const toTheRight = tabs.slice(tabs.indexOf(session.id) + 1)
  const areas = groupsOf(layout.root).length
  const split = (edge: 'right' | 'bottom'): void => {
    if (group) splitTab(session.id, group.id, edge)
  }
  const isClaude = session.kind === 'claude'

  const item = (label: string, action: () => unknown, opts: { danger?: boolean; disabled?: boolean } = {}): ReactNode => (
    <button
      className={`menu-item${opts.danger ? ' danger' : ''}`}
      disabled={opts.disabled}
      onClick={() => {
        closeTabMenu()
        void action()
      }}
    >
      {label}
    </button>
  )

  const onKeyDown = (e: KeyboardEvent): void => {
    if (e.key === 'Escape') {
      e.preventDefault()
      closeTabMenu()
      return
    }
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>('.menu-item:not(:disabled)') ?? [])]
    const at = items.indexOf(document.activeElement as HTMLButtonElement)
    const step = e.key === 'ArrowDown' ? 1 : -1
    items[(at + step + items.length) % items.length]?.focus()
  }

  return (
    <>
      {/* swallows the click that closes the menu; on the terminal a right-click would paste */}
      <div
        className="menu-backdrop"
        onMouseDown={(e) => {
          e.preventDefault()
          closeTabMenu()
        }}
        onContextMenu={(e) => e.preventDefault()}
      />
      <div className="menu tab-menu" ref={ref} style={pos} tabIndex={-1} onKeyDown={onKeyDown} onContextMenu={(e) => e.preventDefault()}>
        {isClaude && item('Rename…', () => startRename(session.id))}
        {isClaude && item('Duplicate', () => duplicateSession(session.id), { disabled: session.status !== 'running' })}
        {item('Split right', () => split('right'), { disabled: tabs.length < 2 })}
        {item('Split down', () => split('bottom'), { disabled: tabs.length < 2 })}
        {areas > 1 && item('Merge all areas', mergeAreas)}
        {isClaude && item('Continue on another account…', () => setModal({ type: 'transfer', sessionId: session.id }), { disabled: !otherAccounts })}
        {isClaude && item(platform === 'win32' ? 'Open folder in Explorer' : 'Open folder', () => revealFolder(session.id))}
        {isClaude &&
          account &&
          item(`${pinned ? '✓ ' : ''}Always open this folder with ${account.name}`, () => setFolderAccount(session.cwd, pinned ? null : account.id))}
        <div className="menu-sep" />
        {item('Close', () => closeSession(session.id))}
        {item('Close others', () => closeSessions(others), { disabled: others.length === 0 })}
        {item('Close tabs to the right', () => closeSessions(toTheRight), { disabled: toTheRight.length === 0 })}
      </div>
    </>
  )
}
