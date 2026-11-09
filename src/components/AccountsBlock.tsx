import { useCallback, useEffect, useRef, useState, type PointerEvent } from 'react'
import { clampAccountsHeight, DEFAULT_SPLIT, isOneCard, ONE_CARD_PX, snapSplit } from '@shared/sidebarSplit'
import type { ProfileView } from '@shared/types'
import { DRAG_THRESHOLD } from '../pointerDrag'
import { setAccountsSplit, useStore } from '../store'
import { AccountCard } from './AccountCard'
import { AccountMini } from './AccountMini'
import { PeopleIcon } from './icons'
import { OtherAccountsPopover } from './OtherAccountsPopover'

// the account the single card showed last: it stays while a panel's tab, or no tab, is in front
let lastShown: string | null = null

/**
 * The account the single card shows: one looked at from the list of the others, else the account
 * of the tab that has the keyboard, else the one shown last, else the main account.
 */
function useShownAccount(profiles: readonly ProfileView[]): ProfileView | undefined {
  const peek = useStore((s) => s.peekAccount)
  const active = useStore((s) => (s.activeId ? (s.sessions.find((x) => x.id === s.activeId)?.profileId ?? null) : null))
  const known = (id: string | null): ProfileView | undefined => (id === null ? undefined : profiles.find((p) => p.id === id))
  const shown = known(peek) ?? known(active) ?? known(lastShown) ?? profiles.find((p) => p.isDefault) ?? profiles[0]
  useEffect(() => {
    if (shown) lastShown = shown.id
  })
  return shown
}

/** The single card, with the button that lists the other accounts at the end of its head. */
function OneCard({ profiles }: { profiles: readonly ProfileView[] }) {
  const shown = useShownAccount(profiles)
  const [open, setOpen] = useState(false)
  const close = useCallback(() => setOpen(false), [])
  if (!shown) return null
  const others = profiles.length - 1
  const button =
    others > 0 ? (
      <button
        className={`icon-btn others${open ? ' on' : ''}`}
        title={`Other accounts (${others})`}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <PeopleIcon />
        <span className="others-count">{others}</span>
      </button>
    ) : null
  return (
    <>
      <AccountCard key={shown.id} profile={shown} extra={button} />
      {open && <OtherAccountsPopover shownId={shown.id} onClose={close} />}
    </>
  )
}

/**
 * The account cards at the top of the sidebar. With the tabs in the sidebar they are as tall as
 * the line below them says, or fold to the single card of the active tab's account; otherwise
 * they take all the room. Collapsed, the sidebar shows initials with small usage bars instead.
 */
export function AccountsBlock({ withTabs }: { withTabs: boolean }) {
  const profiles = useStore((s) => s.profiles)
  const collapsed = useStore((s) => s.sidebarCollapsed)
  const split = useStore((s) => s.accountsSplit)

  if (collapsed) {
    return (
      <div className="cards accounts">
        {profiles.map((profile) => (
          <AccountMini key={profile.id} profile={profile} />
        ))}
      </div>
    )
  }
  if (withTabs && isOneCard(split)) {
    return (
      <div className="cards accounts one">
        <OneCard profiles={profiles} />
      </div>
    )
  }
  return (
    <div className="cards accounts" style={withTabs && !isOneCard(split) ? { maxHeight: split.height } : undefined}>
      {profiles.map((profile) => (
        <AccountCard key={profile.id} profile={profile} />
      ))}
    </div>
  )
}

/** A drag of the line, from the moment the button went down on it. */
interface SplitDrag {
  pointerId: number
  startY: number
  /** How tall the accounts were. */
  startH: number
  /** How tall their cards would be with room enough. */
  contentH: number
  /** The height the accounts and the tab list share. */
  space: number
  /** Where the accounts start, in the sidebar. */
  top: number
  moved: boolean
}

/**
 * The line between the accounts and the tabs in the sidebar. Dragging it moves a marker; the
 * accounts take their new height when it is let go. Up past about one card they fold to the
 * single card of the active tab's account; down from there they open again. A double-click
 * goes back to the height a new window starts with.
 */
export function SplitLine() {
  const one = useStore((s) => isOneCard(s.accountsSplit))
  const drag = useRef<SplitDrag | null>(null)
  const [ghost, setGhost] = useState<{ top: number; one: boolean } | null>(null)

  // the height the accounts get when the pointer is at `clientY`, and whether that folds them to one card
  const heightAt = (d: SplitDrag, clientY: number): { h: number; one: boolean } => {
    const h = d.startH + clientY - d.startY
    return h < ONE_CARD_PX ? { h, one: true } : { h: Math.min(clampAccountsHeight(h, d.space), Math.max(ONE_CARD_PX, d.contentH)), one: false }
  }

  const onPointerDown = (e: PointerEvent<HTMLDivElement>): void => {
    if (e.button !== 0) return
    const line = e.currentTarget
    const accounts = line.parentElement?.querySelector<HTMLElement>(':scope > .accounts')
    const tabs = line.parentElement?.querySelector<HTMLElement>(':scope > .sidebar-tabs')
    if (!accounts || !tabs) return
    line.setPointerCapture(e.pointerId)
    const startH = accounts.getBoundingClientRect().height
    // the cards' full height, as the block would be without its limit; folded, the other cards are not there to measure
    const contentH = one ? Infinity : Math.max(startH, accounts.scrollHeight)
    drag.current = {
      pointerId: e.pointerId,
      startY: e.clientY,
      startH,
      contentH,
      space: startH + tabs.getBoundingClientRect().height,
      top: accounts.offsetTop,
      moved: false
    }
  }

  const onPointerMove = (e: PointerEvent<HTMLDivElement>): void => {
    const d = drag.current
    if (!d || e.pointerId !== d.pointerId) return
    if (!d.moved && Math.abs(e.clientY - d.startY) < DRAG_THRESHOLD) return
    d.moved = true
    const at = heightAt(d, e.clientY)
    // folded, the marker stays where one card would end
    setGhost({ top: d.top + Math.max(0, at.one ? Math.min(at.h, ONE_CARD_PX) : at.h), one: at.one })
  }

  const end = (e: PointerEvent<HTMLDivElement>, commit: boolean): void => {
    const d = drag.current
    if (!d || e.pointerId !== d.pointerId) return
    drag.current = null
    setGhost(null)
    if (!commit || !d.moved) return
    const at = heightAt(d, e.clientY)
    setAccountsSplit(at.one ? { mode: 'one' } : snapSplit(at.h, d.space))
  }

  return (
    <>
      <div
        className={`sidebar-split${ghost ? ' dragging' : ''}`}
        role="separator"
        aria-orientation="horizontal"
        title={
          one
            ? 'Drag down to show all accounts; double-click for the default height'
            : 'Drag to share the room between accounts and tabs (up to the top for a single card); double-click for the default height'
        }
        // keeps the keyboard where it is and stops a text selection
        onMouseDown={(e) => e.preventDefault()}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={(e) => end(e, true)}
        onPointerCancel={(e) => end(e, false)}
        onLostPointerCapture={(e) => end(e, false)}
        onDoubleClick={() => setAccountsSplit(DEFAULT_SPLIT)}
      />
      {ghost && <div className={`sidebar-split-ghost${ghost.one ? ' one' : ''}`} style={{ top: ghost.top }} />}
    </>
  )
}
