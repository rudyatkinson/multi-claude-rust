import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import type { SessionInfo } from '@shared/types'
import { pinnedAccount } from '@shared/folders'
import { groupOf, groupsOf, panelKindOf, panelTabId, type PanelKind } from '@shared/layout'
import {
  closePanel,
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

/** The menu of a right-clicked tab, a session's or a panel's. It opens at the pointer and stays inside the window. */
export function TabMenu() {
  const menu = useStore((s) => s.tabMenu)
  const session = useStore((s) => s.sessions.find((x) => x.id === s.tabMenu?.id))
  const panel = useStore((s) => {
    const kind = s.tabMenu ? panelKindOf(s.tabMenu.id) : null
    return kind && groupOf(s.layout, panelTabId(kind)) ? kind : null
  })
  if (!menu || (!session && !panel)) return null
  // a new position is a new menu: it measures itself again
  return (
    <Menu key={`${menu.id}:${menu.x}:${menu.y}`} x={menu.x} y={menu.y}>
      {session ? <SessionItems session={session} /> : <PanelItems kind={panel!} />}
    </Menu>
  )
}

function Item({ label, action, danger, disabled }: { label: string; action: () => unknown; danger?: boolean; disabled?: boolean }) {
  return (
    <button
      className={`menu-item${danger ? ' danger' : ''}`}
      disabled={disabled}
      onClick={() => {
        closeTabMenu()
        void action()
      }}
    >
      {label}
    </button>
  )
}

/** Where the menu goes, the backdrop that closes it, and its arrow keys. */
function Menu({ x, y, children }: { x: number; y: number; children: ReactNode }) {
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
        {children}
      </div>
    </>
  )
}

/** Split off and merge back: the same for every kind of tab. */
function AreaItems({ id }: { id: string }) {
  const layout = useStore((s) => s.layout)
  const group = groupOf(layout, id)
  const tabs = group?.tabs.length ?? 0
  const areas = groupsOf(layout.root).length
  const split = (edge: 'right' | 'bottom'): void => {
    if (group) splitTab(id, group.id, edge)
  }
  return (
    <>
      <Item label="Split right" action={() => split('right')} disabled={tabs < 2} />
      <Item label="Split down" action={() => split('bottom')} disabled={tabs < 2} />
      {areas > 1 && <Item label="Merge all areas" action={mergeAreas} />}
    </>
  )
}

function PanelItems({ kind }: { kind: PanelKind }) {
  return (
    <>
      <AreaItems id={panelTabId(kind)} />
      <div className="menu-sep" />
      <Item label="Close" action={() => closePanel(kind)} />
    </>
  )
}

function SessionItems({ session }: { session: SessionInfo }) {
  const layout = useStore((s) => s.layout)
  const platform = useStore((s) => s.platform)
  const account = useStore((s) => s.profiles.find((p) => p.id === session.profileId))
  // the folder always opens with this tab's account
  const pinned = useStore((s) => pinnedAccount(s.folderAccounts, session.cwd, s.platform, s.profiles) === session.profileId)
  const otherAccounts = useStore((s) => s.profiles.some((p) => p.id !== session.profileId && p.account?.loggedIn))

  // "others" and "to the right" are about the tabs of this tab's own area, panels among them included
  const tabs = groupOf(layout, session.id)?.tabs ?? []
  const others = tabs.filter((id) => id !== session.id)
  const toTheRight = tabs.slice(tabs.indexOf(session.id) + 1)
  const isClaude = session.kind === 'claude'

  return (
    <>
      {isClaude && <Item label="Rename…" action={() => startRename(session.id)} />}
      {isClaude && <Item label="Duplicate" action={() => duplicateSession(session.id)} disabled={session.status !== 'running'} />}
      <AreaItems id={session.id} />
      {isClaude && <Item label="Continue on another account…" action={() => setModal({ type: 'transfer', sessionId: session.id })} disabled={!otherAccounts} />}
      {isClaude && <Item label={platform === 'win32' ? 'Open folder in Explorer' : 'Open folder'} action={() => revealFolder(session.id)} />}
      {isClaude && account && (
        <Item
          label={`${pinned ? '✓ ' : ''}Always open this folder with ${account.name}`}
          action={() => setFolderAccount(session.cwd, pinned ? null : account.id)}
        />
      )}
      <div className="menu-sep" />
      <Item label="Close" action={() => closeSession(session.id)} />
      <Item label="Close others" action={() => closeTabsAmong(others)} disabled={others.length === 0} />
      <Item label="Close tabs to the right" action={() => closeTabsAmong(toTheRight)} disabled={toTheRight.length === 0} />
    </>
  )
}

/** Closes the sessions among these tabs (asking once), then, unless that was cancelled, the panels among them. */
async function closeTabsAmong(ids: readonly string[]): Promise<void> {
  const panels = ids.map(panelKindOf).filter((kind): kind is PanelKind => kind !== null)
  const closed = await closeSessions(ids.filter((id) => panelKindOf(id) === null))
  if (closed) for (const kind of panels) closePanel(kind)
}
