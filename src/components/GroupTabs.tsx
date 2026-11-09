import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react'
import { createPortal } from 'react-dom'
import type { Group, Rect } from '@shared/layout'
import { useDismiss } from '../hooks'
import { closeNewSession, openNewSession, refocus, useStore } from '../store'
import { startTabPointerDrag } from '../tabDrag'
import { chooseTab, TabRow, useTabViews, type TabView } from './TabRow'

const MARGIN = 6

interface ListProps {
  groupId: string
  views: readonly TabView[]
  activeId: string | null
  onChoose(id: string): void
  onClose(): void
}

/** Every tab of an area, for a strip too short to show them all. It opens at the area's ▾ and stays inside the window. */
function TabList({ groupId, views, activeId, onChoose, onClose }: ListProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  // the ▾ toggles the list itself
  useDismiss(ref, onClose, `[data-group="${groupId}"] .tab-more`)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const anchor = document.querySelector(`[data-group="${groupId}"] .tab-more`)?.getBoundingClientRect()
    const { width, height } = el.getBoundingClientRect()
    const a = anchor ?? { right: MARGIN, bottom: MARGIN }
    // below the button and flush with its right edge
    setPos({
      left: Math.max(MARGIN, Math.min(a.right - width, window.innerWidth - width - MARGIN)),
      top: Math.max(MARGIN, Math.min(a.bottom + 4, window.innerHeight - height - MARGIN))
    })
  }, [groupId])

  // the keyboard comes to the list once it is placed: nothing can be focused while it is hidden
  const placed = pos !== null
  useEffect(() => {
    if (!placed) return
    const list = ref.current
    ;(list?.querySelector<HTMLElement>('.tab-list-item.active') ?? list?.querySelector<HTMLElement>('.tab-list-item'))?.focus()
  }, [placed])

  const onKeyDown = (e: KeyboardEvent): void => {
    const items = [...(ref.current?.querySelectorAll<HTMLElement>('.tab-list-item') ?? [])]
    const at = items.indexOf(document.activeElement as HTMLElement)
    const next = e.key === 'ArrowDown' ? at + 1 : e.key === 'ArrowUp' ? at - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : null
    if (next === null || items.length === 0) return
    e.preventDefault()
    items[(next + items.length) % items.length].focus()
  }

  return createPortal(
    <div
      className="menu tab-list"
      ref={ref}
      role="menu"
      style={pos ?? { left: 0, top: 0, visibility: 'hidden' }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {views.map((view) => (
        <button
          key={view.id}
          role="menuitem"
          className={['menu-item', 'tab-list-item', view.id === activeId && 'active', ...view.classes].filter(Boolean).join(' ')}
          style={{ '--accent': view.accent } as CSSProperties}
          title={view.tooltip}
          onClick={() => onChoose(view.id)}
        >
          {view.mark}
          <span className="tab-text">
            <span className="tab-title">{view.title}</span>
            {view.detail && <span className="tab-detail">{view.detail}</span>}
          </span>
        </button>
      ))}
    </div>,
    document.body
  )
}

interface Props {
  group: Group
  rect: Rect
  /** This area has the keyboard. */
  focused: boolean
}

/**
 * The tab strip of one area, a row on top of it: its sessions and panels, a "+" that opens a new
 * session in it, and a ▾ with the list of all its tabs while they do not fit. With the tabs in
 * the sidebar (Settings → Window → Tab strip) the areas have no strip; see `SidebarTabs`.
 */
export function GroupTabs({ group, rect, focused }: Props) {
  const pickerHere = useStore((s) => s.newSession?.groupId === group.id)
  const claudeFound = useStore((s) => s.claude.path !== null)
  // where a dragged tab would land, when that is in this strip
  const mark = useStore((s) => {
    const m = s.dropMark
    return m?.kind === 'tab' && group.tabs.includes(m.tabId) ? m : null
  })
  const views = useTabViews(group)
  const strip = useRef<HTMLDivElement>(null)
  const [overflow, setOverflow] = useState(false)
  const [listOpen, setListOpen] = useState(false)

  const scrollTo = useCallback((id: string) => {
    strip.current?.querySelector(`[data-id="${id}"]`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [])

  // a tab chosen with the keyboard may be scrolled out of the strip
  useEffect(() => {
    if (group.active) scrollTo(group.active)
  }, [group.active, scrollTo])

  // whether the tabs fit: measured after every render (tabs come and go, titles change) and whenever the strip changes size
  const measure = useCallback(() => {
    const el = strip.current
    if (el) setOverflow(el.scrollWidth > el.clientWidth)
  }, [])
  useLayoutEffect(measure)
  useEffect(() => {
    const el = strip.current
    if (!el) return
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [measure])

  // nothing left to list
  useEffect(() => {
    if (!overflow) setListOpen(false)
  }, [overflow])

  return (
    <div className={`tabbar${focused ? ' focused' : ''}`} data-group={group.id} style={{ left: rect.x, top: rect.y, width: rect.w }}>
      <div className="tabs" ref={strip}>
        {views.map((view) => (
          <TabRow
            key={view.id}
            view={view}
            active={view.id === group.active}
            drop={mark?.tabId === view.id ? mark.side : null}
            onDragStart={(e) => startTabPointerDrag(e, view.id)}
          />
        ))}
      </div>
      <div className="tab-actions">
        <button
          className="tab-new"
          disabled={!claudeFound}
          title="New session here (Ctrl+Shift+T)"
          onClick={() => (pickerHere ? closeNewSession() : openNewSession(group.id))}
        >
          +
        </button>
        {overflow && (
          <button
            className={`tab-more${listOpen ? ' on' : ''}`}
            title={`All ${views.length} tabs of this area`}
            aria-haspopup="menu"
            aria-expanded={listOpen}
            onClick={() => setListOpen(!listOpen)}
          >
            <span className="tab-count">{views.length}</span>▾
          </button>
        )}
      </div>
      {listOpen && (
        <TabList
          groupId={group.id}
          views={views}
          activeId={group.active}
          onChoose={(id) => {
            setListOpen(false)
            chooseTab(id)
            scrollTo(id)
          }}
          onClose={() => {
            setListOpen(false)
            refocus()
          }}
        />
      )}
    </div>
  )
}
