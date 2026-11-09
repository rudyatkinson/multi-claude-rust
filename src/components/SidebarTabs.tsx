import { useEffect, useRef, type CSSProperties } from 'react'
import { groupsOf, type Group } from '@shared/layout'
import { dropTarget } from '@shared/tabOrder'
import { closeNewSession, closeTab, focusArea, moveTabTo, openNewSession, openTabMenu, useStore } from '../store'
import { scrollAtEdge, startTabPointerDrag, type DropMark, type ListTarget, type TabDrop } from '../tabDrag'
import { chooseTab, TabRow, useTabViewData, viewsOf, type TabView } from './TabRow'

/**
 * Where a tab dragged over the list at height `y` (client coordinates) would land: in front of
 * or behind the row under the pointer, in that row's area; an area's header takes it to the end
 * of that area. `list` holds one `[data-area]` section per area, in reading order.
 */
function dropInList(list: HTMLElement, y: number): TabDrop | null {
  const sections = [...list.querySelectorAll<HTMLElement>('[data-area]')]
  if (sections.length === 0) return null
  // the last section that starts above the pointer; above them all, the first
  const section = sections.reduce((found, s) => (y >= s.getBoundingClientRect().top ? s : found), sections[0])
  const groupId = section.dataset.area!
  const head = section.querySelector<HTMLElement>('.area-head')
  const toEnd = (mark: DropMark | null): TabDrop => ({ preview: null, mark, commit: (id) => moveTabTo(id, groupId, null) })
  if (head && y < head.getBoundingClientRect().bottom) return toEnd({ kind: 'area', groupId })
  const rows = [...section.querySelectorAll<HTMLElement>('.tab')].map((tab) => {
    const { top, bottom } = tab.getBoundingClientRect()
    return { id: tab.dataset.id!, left: top, right: bottom }
  })
  // top to bottom along the column
  const target = dropTarget(rows, y)
  if (!target) return toEnd(head ? { kind: 'area', groupId } : null)
  return { preview: null, mark: { kind: 'tab', tabId: target.id, side: target.side }, commit: (id) => moveTabTo(id, groupId, target.beforeId) }
}

/** An area's tabs as small marks in the collapsed sidebar: a click brings one to the front. */
function MiniTabs({ views, group }: { views: readonly TabView[]; group: Group }) {
  return views.map((view) => (
    <button
      key={view.id}
      data-id={view.id}
      className={['mini-tab', view.id === group.active && 'active', ...view.classes].filter(Boolean).join(' ')}
      style={{ '--accent': view.accent } as CSSProperties}
      title={view.account ? `${view.title} · ${view.account}` : view.title}
      onClick={() => chooseTab(view.id)}
      onAuxClick={(e) => {
        if (e.button === 1) closeTab(view.id)
      }}
      onContextMenu={(e) => {
        e.preventDefault()
        openTabMenu(view.id, e.clientX, e.clientY)
      }}
    >
      {view.mark}
    </button>
  ))
}

/**
 * Every tab in the sidebar, under the accounts (Settings → Window → Tab strip: Sidebar): the
 * areas in reading order, each under an "Area N" header when there are several, and a button for
 * a new session in the area that has the keyboard. The rows work as in a strip: drag one within
 * the list, onto another area's header, or out onto the areas to split them. Collapsed, the
 * sidebar shows a column of marks instead.
 */
export function SidebarTabs() {
  const layout = useStore((s) => s.layout)
  const collapsed = useStore((s) => s.sidebarCollapsed)
  const activeId = useStore((s) => s.activeId)
  const mark = useStore((s) => s.dropMark)
  const pickerOpen = useStore((s) => s.newSession !== null)
  const claudeFound = useStore((s) => s.claude.path !== null)
  const data = useTabViewData()
  const zone = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const groups = groupsOf(layout.root)
  const several = groups.length > 1

  // the tab with the keyboard stays in view, also when it was chosen with the keyboard
  useEffect(() => {
    if (activeId) list.current?.querySelector(`[data-id="${activeId}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [activeId, collapsed])

  // a tab dragged from this list: over it, it moves within it; elsewhere the areas decide
  const dropHere: ListTarget = (x, y, scroll) => {
    const box = zone.current?.getBoundingClientRect()
    const rows = list.current
    if (!box || !rows || x < box.left || x >= box.right || y < box.top || y >= box.bottom) return undefined
    if (scroll) scrollAtEdge(rows, x, y, true)
    return dropInList(rows, y)
  }

  const newSession = (
    <button
      className={collapsed ? 'tab-new mini-tab new' : 'tab-new row'}
      disabled={!claudeFound}
      title="New session in the area that has the keyboard (Ctrl+Shift+T)"
      onClick={() => (pickerOpen ? closeNewSession() : openNewSession())}
    >
      {collapsed ? '+' : '+ New session'}
    </button>
  )

  return (
    <div className={`sidebar-tabs${mark?.kind === 'list' ? ' files' : ''}`} ref={zone} onContextMenu={(e) => e.preventDefault()}>
      <div className="sidebar-tab-list" ref={list}>
        {groups.map((group, index) => {
          const views = viewsOf(group, data)
          const focused = group.id === layout.focused
          // the colour of the account whose tab the area shows; a panel has none, and its header then stays in the text colour
          const shown = views.find((v) => v.id === group.active)
          const accent = shown?.session ? shown.accent : undefined
          return (
            <div key={group.id} className={`area${focused ? ' focused' : ''}`} data-area={group.id}>
              {several &&
                (collapsed ? (
                  // a thin line between two areas
                  index > 0 && <div className="mini-area-sep" />
                ) : (
                  <button
                    className={`area-head${focused ? ' focused' : ''}${mark?.kind === 'area' && mark.groupId === group.id ? ' drop-into' : ''}`}
                    style={accent ? ({ '--accent': accent } as CSSProperties) : undefined}
                    title={focused ? `Area ${index + 1} has the keyboard` : `Give Area ${index + 1} the keyboard`}
                    onClick={() => focusArea(group.id)}
                  >
                    <span className="area-name">Area {index + 1}</span>
                    <span className="area-count">{views.length}</span>
                  </button>
                ))}
              {collapsed ? (
                <MiniTabs views={views} group={group} />
              ) : (
                views.map((view) => (
                  <TabRow
                    key={view.id}
                    view={view}
                    active={view.id === group.active}
                    drop={mark?.kind === 'tab' && mark.tabId === view.id ? mark.side : null}
                    onDragStart={(e) => startTabPointerDrag(e, view.id, dropHere)}
                  />
                ))
              )}
            </div>
          )
        })}
      </div>
      {newSession}
    </div>
  )
}
