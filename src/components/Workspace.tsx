import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview'
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent, type RefObject } from 'react'
import {
  bodyRect,
  canDrop,
  flatten,
  groupsOf,
  hitTest,
  layoutRects,
  panelKindOf,
  panelTabId,
  previewRect,
  ratioAt,
  stripRect,
  type DividerRect,
  type Geometry,
  type Hit,
  type HitOptions,
  type Metrics,
  type PanelKind,
  type Rect,
  type StripSide
} from '@shared/layout'
import { pathsAsInput } from '@shared/paths'
import { sleepLabel } from '@shared/sleep'
import { dropTarget, type DropTarget } from '@shared/tabOrder'
import { tabStripOf, type SessionInfo } from '@shared/types'
import { errorMessage } from '../api'
import { followDrag } from '../pointerDrag'
import {
  closeSession,
  endTabDrag,
  moveTabTo,
  openDropped,
  openNewSession,
  refocus,
  restartSession,
  setActive,
  setSplitRatio,
  setTabTitle,
  splitTab,
  splitTabRoot,
  startTabDrag,
  useStore,
  wakeSession
} from '../store'
import { fitTerminal, focusTerminal, mountTerminal, pasteText, setTerminalAsleep, setTerminalVisible } from '../terminal/terminalRegistry'
import { GroupTabs, type StripMark } from './GroupTabs'
import { LibraryPanel } from './LibraryPanel'
import { ArrivedBar, LimitBar } from './LimitBar'
import { SearchBar } from './SearchBar'
import { WorkflowsPanel } from './WorkflowsPanel'

/**
 * The size of an area's tab strip: the height of the row on top, the width of the column on the
 * left. `--strip` in the stylesheet follows it (set on the workspace).
 */
const STRIP = { top: 44, side: 200 } as const
const METRICS: Metrics = { gap: 1, minW: 260, minH: 170 }
const ROOT_BAND = 28
const EDGE_SHARE = 0.25
// how close to an end of a tab strip a dragged tab starts scrolling it, by how much, and how often
const SCROLL_EDGE = 48
const SCROLL_STEP = 14
const SCROLL_TICK_MS = 50

interface SlotProps {
  session: SessionInfo
  /** Where the terminal goes; null while its tab is not the one its area shows. */
  rect: Rect | null
  /** It has the keyboard. */
  focused: boolean
  searching: boolean
  color: string
}

function TerminalSlot({ session, rect, focused, searching, color }: SlotProps) {
  const host = useRef<HTMLDivElement>(null)
  // on its way to another account: it looks ended for a moment, but restarting it now would break the move
  const moving = session.moving === true
  const shown = rect !== null

  useEffect(() => {
    const { settings, platform, osBuild } = useStore.getState()
    // disposal follows the session list (see store), not this component's lifetime
    mountTerminal(session.id, host.current!, {
      fontSize: settings.fontSize,
      platform,
      osBuild,
      onTitle: (title) => setTabTitle(session.id, title)
    })
  }, [session.id])

  useEffect(() => {
    setTerminalVisible(session.id, shown)
    if (shown) fitTerminal(session.id)
  }, [shown, session.id])

  useEffect(() => {
    if (focused) focusTerminal(session.id)
  }, [focused, session.id])

  // asleep: the terminal keeps what it showed; a key in it or a click on it wakes the session
  const asleep = session.status === 'sleeping'
  useEffect(() => {
    setTerminalAsleep(session.id, asleep ? () => void wakeSession(session.id) : null)
  }, [asleep, session.id])

  const exited = session.status === 'exited'
  return (
    <div
      className={`slot${shown ? ' shown' : ''}${focused ? ' focused' : ''}${asleep ? ' asleep' : ''}`}
      style={{ '--accent': color, ...(rect && { left: rect.x, top: rect.y, width: rect.w, height: rect.h }) } as CSSProperties}
      // a click anywhere in another area moves the keyboard there
      onMouseDownCapture={() => {
        if (shown && !focused) setActive(session.id)
      }}
    >
      <div className="term-host" ref={host} />
      {asleep && shown && (
        <div
          className="sleep-overlay"
          title={sleepLabel(session.sleptAt)}
          // keeps the keyboard where it is until the session is awake; then it goes to the terminal
          onMouseDown={(e) => {
            e.preventDefault()
            void wakeSession(session.id)
          }}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button type="button" className="btn sleep-wake" onClick={() => void wakeSession(session.id)}>
            <span className="tab-state asleep" />
            Sleeping · Wake
          </button>
        </div>
      )}
      {searching && <SearchBar id={session.id} />}
      {moving && <div className="exit-bar">Moving the conversation to the other account…</div>}
      {!moving && shown && <ArrivedBar session={session} />}
      {!moving && shown && <LimitBar session={session} />}
      {exited && !moving && (
        <div className="exit-bar">
          <span>
            {session.kind === 'login' ? 'Login finished' : 'Session ended'}
            {session.exitCode ? ` (code ${session.exitCode})` : ''}
          </span>
          <span className="spacer" />
          {session.kind === 'claude' && (
            <button className="btn" onClick={() => void restartSession(session.id, true)}>
              Continue conversation
            </button>
          )}
          <button className="btn" onClick={() => void restartSession(session.id, false)}>
            {session.kind === 'login' ? 'Try again' : 'Restart'}
          </button>
          <button className="btn" onClick={() => void closeSession(session.id)}>
            Close
          </button>
        </div>
      )}
    </div>
  )
}

/**
 * The line between two areas. Dragging it moves a marker only; the areas take their new size
 * when it is released, since every step would resize the terminals and their programs.
 */
function Divider({ divider, area }: { divider: DividerRect; area: RefObject<HTMLDivElement | null> }) {
  const [ghost, setGhost] = useState<number | null>(null)
  const row = divider.dir === 'row'

  const ratioOf = (clientX: number, clientY: number): number => {
    const box = area.current!.getBoundingClientRect()
    return ratioAt(divider, clientX - box.left, clientY - box.top)
  }
  // where the marker sits for a ratio, along the split's own box
  const offset = (ratio: number): number => (row ? divider.box.x + ratio * (divider.box.w - divider.w) : divider.box.y + ratio * (divider.box.h - divider.h))

  return (
    <>
      <div
        className={`divider ${row ? 'row' : 'column'}`}
        style={{ left: divider.x, top: divider.y, width: divider.w, height: divider.h }}
        title="Drag to resize; double-click for equal halves"
        // keeps the keyboard in the terminal
        onMouseDown={(e) => e.preventDefault()}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId)
          setGhost(ratioOf(e.clientX, e.clientY))
        }}
        onPointerMove={(e) => {
          if (ghost !== null) setGhost(ratioOf(e.clientX, e.clientY))
        }}
        onPointerUp={() => {
          if (ghost !== null) setSplitRatio(divider.splitId, ghost)
          setGhost(null)
        }}
        onPointerCancel={() => setGhost(null)}
        onDoubleClick={() => setSplitRatio(divider.splitId, 0.5)}
      />
      {ghost !== null && (
        <div
          className="divider-ghost"
          style={row ? { left: offset(ghost), top: divider.y, width: 2, height: divider.h } : { left: divider.x, top: offset(ghost), width: divider.w, height: 2 }}
        />
      )}
    </>
  )
}

/**
 * A panel in the body of the area whose tab in front it is. The terminals of that area stay
 * mounted underneath, hidden, as they are while another of its tabs is in front.
 */
function PanelHost({ kind, rect, focused }: { kind: PanelKind; rect: Rect; focused: boolean }) {
  const host = useRef<HTMLDivElement>(null)
  const id = panelTabId(kind)

  // with the keyboard in its area the panel takes it, unless one of its own fields has it already
  useEffect(() => {
    const el = host.current
    if (focused && el && !el.contains(document.activeElement)) el.querySelector<HTMLElement>('.wf-panel')?.focus()
  }, [focused])

  return (
    <div
      className="panel-host"
      ref={host}
      data-tab={id}
      style={{ left: rect.x, top: rect.y, width: rect.w, height: rect.h }}
      // a click anywhere in it moves the keyboard to its area, as a click in a terminal does
      onMouseDownCapture={() => {
        if (!focused) setActive(id)
      }}
    >
      {kind === 'workflows' ? <WorkflowsPanel /> : <LibraryPanel />}
    </div>
  )
}

/** What a dragged tab would do where the pointer is. */
interface DropState {
  hit: Hit
  /** The part of the window that lights up; a drop on a tab strip marks a tab instead. */
  preview: Rect | null
  target: DropTarget | null
}

const sameDrop = (a: DropState | null, b: DropState | null): boolean =>
  a === b || (!!a && !!b && JSON.stringify(a.hit) === JSON.stringify(b.hit) && a.target?.id === b.target?.id && a.target?.side === b.target?.side)

/** Where files dragged in from outside would go: into a terminal as paths, or onto a tab strip as folders to open. */
interface FileDrop {
  groupId: string
  /** The session whose terminal takes the paths; missing for a drop on the tab strip. */
  sessionId?: string
  preview: Rect
}

/** The areas of the window: each with its tab strip and the terminal or the panel of the tab it shows. */
export function Workspace() {
  const sessions = useStore((s) => s.sessions)
  const profiles = useStore((s) => s.profiles)
  const layout = useStore((s) => s.layout)
  const searchId = useStore((s) => s.searchId)
  const hasAccount = useStore((s) => s.profiles.some((p) => p.account?.loggedIn))
  const side = useStore((s) => tabStripOf(s.settings) === 'side')
  const area = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [drop, setDrop] = useState<DropState | null>(null)
  const [fileDrop, setFileDrop] = useState<FileDrop | null>(null)

  const strip = side ? STRIP.side : STRIP.top
  const stripSide: StripSide = side ? 'left' : 'top'
  const metrics: Metrics = useMemo(() => ({ ...METRICS, stripSide, strip }), [stripSide, strip])
  const hitOptions: HitOptions = { strip, stripSide, rootBand: ROOT_BAND, edgeShare: EDGE_SHARE }

  useLayoutEffect(() => {
    const el = area.current!
    const measure = (): void => setSize((prev) => (prev.w === el.clientWidth && prev.h === el.clientHeight ? prev : { w: el.clientWidth, h: el.clientHeight }))
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    measure()
    return () => observer.disconnect()
  }, [])

  const geometry: Geometry = useMemo(() => layoutRects(layout.root, size.w, size.h, metrics), [layout.root, size.w, size.h, metrics])
  const groups = groupsOf(layout.root)
  const rectOf = new Map(geometry.groups.map((g) => [g.id, g]))
  // the area's rectangle without its tab strip
  const bodyIn = (rect: Rect): Rect => {
    const body = bodyRect(rect, strip, stripSide)
    return { ...body, w: Math.max(0, body.w), h: Math.max(0, body.h) }
  }
  // the body of the area each shown tab is in
  const bodyOf = new Map<string, Rect>()
  for (const group of groups) {
    const rect = rectOf.get(group.id)
    if (group.active && rect) bodyOf.set(group.active, bodyIn(rect))
  }
  const focusedId = groups.find((g) => g.id === layout.focused)?.active ?? null

  /**
   * What the tab being dragged would do with the pointer at a point (client coordinates); null
   * where it cannot be dropped. With `scroll`, a tab strip under the pointer near one of its ends
   * scrolls a step first.
   */
  const dropAt = (clientX: number, clientY: number, scroll: boolean): DropState | null => {
    const id = useStore.getState().tabDrag
    const el = area.current
    if (!id || !el) return null
    const box = el.getBoundingClientRect()
    const hit = hitTest(geometry, size.w, size.h, clientX - box.left, clientY - box.top, hitOptions)
    if (!hit || !canDrop(layout, id, hit, geometry, size.w, size.h, metrics)) return null
    if (hit.kind !== 'strip') return { hit, preview: previewRect(geometry, size.w, size.h, hit, hitOptions), target: null }
    const tabStrip = el.querySelector<HTMLElement>(`[data-group="${hit.groupId}"] .tabs`)
    const tabs = [...(tabStrip?.querySelectorAll<HTMLElement>('.tab') ?? [])]
    if (tabStrip && scroll) {
      const { left, right, top, bottom } = tabStrip.getBoundingClientRect()
      if (side) {
        if (clientY < top + SCROLL_EDGE) tabStrip.scrollTop -= SCROLL_STEP
        else if (clientY > bottom - SCROLL_EDGE) tabStrip.scrollTop += SCROLL_STEP
      } else if (clientX < left + SCROLL_EDGE) tabStrip.scrollLeft -= SCROLL_STEP
      else if (clientX > right - SCROLL_EDGE) tabStrip.scrollLeft += SCROLL_STEP
    }
    // along the strip: left to right in a row, top to bottom in a column
    const target = dropTarget(
      tabs.map((tab) => {
        const { left, right, top, bottom } = tab.getBoundingClientRect()
        return side ? { id: tab.dataset.id!, left: top, right: bottom } : { id: tab.dataset.id!, left, right }
      }),
      side ? clientY : clientX
    )
    return { hit, preview: null, target }
  }

  /** What files dragged over a point (client coordinates) would do; null where they cannot go. */
  const filesAt = (clientX: number, clientY: number): FileDrop | null => {
    const el = area.current
    if (!el) return null
    // a panel, a dialog or a menu that lies over the areas takes no files
    const top = document.elementFromPoint(clientX, clientY)
    if (!top || !el.contains(top)) return null
    const box = el.getBoundingClientRect()
    const x = clientX - box.left
    const y = clientY - box.top
    const rect = geometry.groups.find((g) => x >= g.x && x < g.x + g.w && y >= g.y && y < g.y + g.h)
    const group = rect && groups.find((g) => g.id === rect.id)
    if (!rect || !group) return null
    if (side ? x < rect.x + strip : y < rect.y + strip) return { groupId: group.id, preview: stripRect(rect, strip, stripSide) }
    // typing paths only makes sense into a session that is running; a panel takes none
    const session = sessions.find((s) => s.id === group.active)
    if (!session || session.kind !== 'claude' || session.status !== 'running') return null
    return { groupId: group.id, sessionId: session.id, preview: bodyIn(rect) }
  }

  /** Files dragged in from outside, as the webview reports them: with positions in physical pixels. */
  const onFileDrag = (event: DragDropEvent): void => {
    if (event.type === 'leave') {
      setFileDrop(null)
      return
    }
    const scale = window.devicePixelRatio || 1
    const target = filesAt(event.position.x / scale, event.position.y / scale)
    if (event.type !== 'drop') {
      setFileDrop((prev) => (prev?.groupId === target?.groupId && prev?.sessionId === target?.sessionId ? prev : target))
      return
    }
    setFileDrop(null)
    if (!target || event.paths.length === 0) return
    if (!target.sessionId) {
      void openDropped(event.paths, target.groupId)
      return
    }
    const text = pathsAsInput(event.paths, useStore.getState().platform)
    if (!text) return
    setActive(target.sessionId)
    pasteText(target.sessionId, text)
    focusTerminal(target.sessionId)
  }

  // the handlers below outlive the render that set them up; they reach the current areas through this
  const current = useRef({ dropAt, onFileDrag })
  current.current = { dropAt, onFileDrag }

  useEffect(() => {
    let unlisten: (() => void) | null = null
    let gone = false
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => current.current.onFileDrag(event.payload))
        .then(
          (fn) => {
            if (gone) fn()
            else unlisten = fn
          },
          (err: unknown) => console.warn('Files dropped on the window are not taken:', errorMessage(err))
        )
    } catch (err) {
      // outside the app's webview, as in a plain browser
      console.warn('Files dropped on the window are not taken:', errorMessage(err))
    }
    return () => {
      gone = true
      unlisten?.()
    }
  }, [])

  /** A button went down on a tab: a few pixels on, it drags the tab, to another place in a strip or to an area of its own. */
  const onTabPointerDown = (e: PointerEvent<HTMLElement>, id: string): void => {
    let at: { x: number; y: number } | null = null
    let ticker: ReturnType<typeof setInterval> | undefined
    const show = (scroll: boolean): DropState | null => {
      const next = at ? current.current.dropAt(at.x, at.y, scroll) : null
      setDrop((prev) => (sameDrop(prev, next) ? prev : next))
      return next
    }
    const finish = (): void => {
      clearInterval(ticker)
      setDrop(null)
      endTabDrag()
    }
    followDrag(e, {
      start() {
        startTabDrag(id)
        // near an end of a strip it keeps scrolling while the pointer rests there
        ticker = setInterval(() => show(true), SCROLL_TICK_MS)
      },
      move(x, y) {
        at = { x, y }
        return show(false) !== null
      },
      drop(x, y) {
        // null when the tab closed during the drag
        const dragged = useStore.getState().tabDrag
        const next = current.current.dropAt(x, y, false)
        finish()
        if (!dragged || !next) {
          refocus()
          return
        }
        const { hit } = next
        if (hit.kind === 'strip') moveTabTo(dragged, hit.groupId, next.target?.beforeId ?? null)
        else if (hit.kind === 'move') moveTabTo(dragged, hit.groupId, null)
        else if (hit.kind === 'split') splitTab(dragged, hit.groupId, hit.edge)
        else splitTabRoot(dragged, hit.edge)
      },
      cancel() {
        finish()
        refocus()
      }
    })
  }

  const markFor = (groupId: string): StripMark | null =>
    drop?.hit.kind === 'strip' && drop.hit.groupId === groupId && drop.target ? { tabId: drop.target.id, side: drop.target.side } : null

  // no tab at all: the one area there is tells how to start
  const emptyRect = flatten(layout).length === 0 ? rectOf.get(groups[0].id) : undefined
  const emptyBody = emptyRect && bodyIn(emptyRect)

  return (
    <div className={`workspace${side ? ' side' : ''}`} ref={area} style={{ '--strip': `${strip}px` } as CSSProperties}>
      {groups.map((group) => {
        const rect = rectOf.get(group.id)
        return rect ? (
          <GroupTabs key={group.id} group={group} rect={rect} focused={group.id === layout.focused} mark={markFor(group.id)} onTabPointerDown={onTabPointerDown} />
        ) : null
      })}
      {/* keyed by the panel, so that one dragged to another area keeps what it shows */}
      {groups.map((group) => {
        const kind = group.active ? panelKindOf(group.active) : null
        const body = group.active ? bodyOf.get(group.active) : undefined
        return kind && body ? <PanelHost key={group.active} kind={kind} rect={body} focused={group.id === layout.focused} /> : null
      })}
      {emptyBody && (
        <div className="empty" style={{ left: emptyBody.x, top: emptyBody.y, width: emptyBody.w, height: emptyBody.h }}>
          <div className="empty-title">No open sessions</div>
          <div className="muted">
            {hasAccount
              ? 'Use the + in the tab bar (Ctrl+Shift+T) to pick an account and a folder and start Claude Code.'
              : 'Log in to the account on the left to get started.'}
          </div>
          {hasAccount && (
            <button className="btn primary" onClick={() => openNewSession()}>
              New session
            </button>
          )}
        </div>
      )}
      {/* one flat list in a steady order: a terminal stays in the element it was opened in, and where it shows is a matter of CSS */}
      {sessions.map((session) => (
        <TerminalSlot
          key={session.id}
          session={session}
          rect={bodyOf.get(session.id) ?? null}
          focused={session.id === focusedId}
          searching={session.id === searchId}
          color={profiles.find((p) => p.id === session.profileId)?.color ?? '#888'}
        />
      ))}
      {geometry.dividers.map((divider) => (
        <Divider key={divider.splitId} divider={divider} area={area} />
      ))}
      {drop?.preview && <div className="drop-preview" style={{ left: drop.preview.x, top: drop.preview.y, width: drop.preview.w, height: drop.preview.h }} />}
      {fileDrop && (
        <div className="drop-preview files" style={{ left: fileDrop.preview.x, top: fileDrop.preview.y, width: fileDrop.preview.w, height: fileDrop.preview.h }}>
          {fileDrop.sessionId ? 'Drop to insert the path' : 'Drop to open the folder here'}
        </div>
      )}
    </div>
  )
}
