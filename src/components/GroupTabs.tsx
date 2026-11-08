import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { panelKindOf, type Group, type PanelKind, type Rect } from '@shared/layout'
import { tabMark } from '@shared/turn'
import { tabStripOf, type SessionInfo } from '@shared/types'
import { isActiveRun } from '@shared/workflows'
import { tabLabel, usageView } from '../format'
import { useDismiss } from '../hooks'
import { useUnsaved } from '../library'
import { closeNewSession, closeTab, finishRename, openNewSession, openTabMenu, refocus, setActive, startRename, useStore } from '../store'
import { LibraryIcon, WorkflowIcon } from './icons'

/** Where a dragged tab would land in a strip: in front of or behind one of its tabs. */
export interface StripMark {
  tabId: string
  side: 'before' | 'after'
}

/** What the tab of a panel says about it. */
const PANEL_TABS: Record<PanelKind, { title: string; detail: string; tooltip: string; icon: () => ReactNode }> = {
  workflows: { title: 'Workflows', detail: 'designer', tooltip: 'Workflows: chain sessions across accounts', icon: WorkflowIcon },
  library: { title: 'Library', detail: 'skills · agents · memory · instructions', tooltip: 'Library: the skills and agents Claude Code works with', icon: LibraryIcon }
}

/** What a tab shows, in its strip and in the list of all the tabs of its area. */
interface TabView {
  id: string
  /** In front of the title: the state of a session, the icon of a panel. */
  mark: ReactNode
  title: string
  detail: string
  tooltip: string
  /** The account's colour; a panel has none. */
  accent: string
  /** `exited`, `attention`, `panel`. */
  classes: string[]
  session?: SessionInfo
  /** For the session's own name being typed in place of the title. */
  topic?: string | null
}

const MARGIN = 6

/** The tab's name being typed. Enter or leaving the field saves it, Escape keeps the old one. */
function RenameInput({ session, placeholder }: { session: SessionInfo; placeholder: string }) {
  const input = useRef<HTMLInputElement>(null)
  const cancelled = useRef(false)

  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [])

  return (
    <input
      ref={input}
      className="tab-rename"
      defaultValue={session.name ?? ''}
      placeholder={placeholder}
      maxLength={80}
      spellCheck={false}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onBlur={(e) => finishRename(session.id, cancelled.current ? null : e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Escape') cancelled.current = true
        if (e.key === 'Enter' || e.key === 'Escape') e.currentTarget.blur()
      }}
    />
  )
}

/** The views of an area's tabs, in tab order; tabs whose session or panel is not known are left out. */
function useTabViews(group: Group): TabView[] {
  const sessions = useStore((s) => s.sessions)
  const profiles = useStore((s) => s.profiles)
  const tabStatus = useStore((s) => s.tabStatus)
  const attention = useStore((s) => s.attention)
  const running = useStore((s) => s.runs.some(isActiveRun))
  const unsaved = useUnsaved((s) => Object.keys(s.items).length > 0)

  return group.tabs.flatMap((id): TabView[] => {
    const kind = panelKindOf(id)
    if (kind) {
      const panel = PANEL_TABS[kind]
      // what the sidebar buttons show: a run under way, something unsaved
      const live = kind === 'workflows' && running
      const dirty = kind === 'library' && unsaved
      const Icon = panel.icon
      return [
        {
          id,
          mark: (
            <span className={`tab-icon${live ? ' live' : ''}${dirty ? ' unsaved' : ''}`}>
              <Icon />
            </span>
          ),
          title: panel.title,
          detail: panel.detail,
          tooltip: live ? 'Workflows (one is running)' : dirty ? 'Library (something has unsaved changes)' : panel.tooltip,
          accent: 'var(--muted)',
          classes: ['panel']
        }
      ]
    }
    const session = sessions.find((s) => s.id === id)
    if (!session) return []
    const profile = profiles.find((p) => p.id === session.profileId)
    const status = tabStatus[session.id]
    const sign = tabMark({ exited: session.status === 'exited', turn: session.turn, busy: status?.busy === true, busySince: status?.busySince })
    const topic = status?.topic ?? null
    const title = tabLabel(session, topic)
    return [
      {
        id,
        mark: <span className={`tab-state ${sign.state}`} title={sign.label} />,
        title,
        // the folder moves to the second line once the tab is called something else
        detail: [title === session.title ? null : session.title, profile?.name, session.model].filter(Boolean).join(' · '),
        tooltip: [session.name, topic, session.cwd, profile?.name].filter(Boolean).join('\n'),
        accent: profile?.color ?? '#888',
        classes: [sign.state === 'ended' && 'exited', attention[session.id] && 'attention'].filter((c): c is string => !!c),
        session,
        topic
      }
    ]
  })
}

interface ListProps {
  groupId: string
  views: readonly TabView[]
  activeId: string | null
  /** The strip is a column on the left: the list opens beside its button rather than below it. */
  side: boolean
  onChoose(id: string): void
  onClose(): void
}

/** Every tab of an area, for a strip too short to show them all. It opens at the area's ▾ and stays inside the window. */
function TabList({ groupId, views, activeId, side, onChoose, onClose }: ListProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  // the ▾ toggles the list itself
  useDismiss(ref, onClose, `[data-group="${groupId}"] .tab-more`)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const anchor = document.querySelector(`[data-group="${groupId}"] .tab-more`)?.getBoundingClientRect()
    const { width, height } = el.getBoundingClientRect()
    const a = anchor ?? { left: MARGIN, right: MARGIN, top: MARGIN, bottom: MARGIN }
    // below the button and flush with its right edge; beside a column strip, flush with its bottom
    const left = side ? a.right + 4 : a.right - width
    const top = side ? a.bottom - height : a.bottom + 4
    setPos({
      left: Math.max(MARGIN, Math.min(left, window.innerWidth - width - MARGIN)),
      top: Math.max(MARGIN, Math.min(top, window.innerHeight - height - MARGIN))
    })
  }, [groupId, side])

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
  mark: StripMark | null
  /** A button went down on a tab: it may become a drag (see `Workspace`). */
  onTabPointerDown(e: PointerEvent<HTMLElement>, id: string): void
}

/**
 * The tab strip of one area: its sessions and panels, a "+" that opens a new session in it, and
 * a ▾ with the list of all its tabs while they do not fit. A row on top of the area, or a column
 * on its left (Settings → Window → Tab strip).
 */
export function GroupTabs({ group, rect, focused, mark, onTabPointerDown }: Props) {
  const renamingId = useStore((s) => s.renamingId)
  const pickerHere = useStore((s) => s.newSession?.groupId === group.id)
  const claudeFound = useStore((s) => s.claude.path !== null)
  const side = useStore((s) => tabStripOf(s.settings) === 'side')
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
    if (el) setOverflow(side ? el.scrollHeight > el.clientHeight : el.scrollWidth > el.clientWidth)
  }, [side])
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

  const choose = (id: string): void => {
    setActive(id)
    // also when the tab was in front already and only the keyboard had left it
    refocus()
  }

  const style: CSSProperties = side ? { left: rect.x, top: rect.y, height: rect.h } : { left: rect.x, top: rect.y, width: rect.w }

  return (
    <div className={`tabbar${side ? ' side' : ''}${focused ? ' focused' : ''}`} data-group={group.id} style={style}>
      <div className="tabs" ref={strip}>
        {views.map((view) => {
          const { id, session } = view
          const renaming = session !== undefined && renamingId === id
          const context = session?.contextPct
          const classes = ['tab', id === group.active && 'active', ...view.classes, mark?.tabId === id && `drop-${mark.side}`].filter(Boolean).join(' ')
          return (
            <div
              key={id}
              data-id={id}
              className={classes}
              style={{ '--accent': view.accent } as CSSProperties}
              title={renaming ? undefined : view.tooltip}
              onPointerDown={(e) => {
                // the name being typed takes the pointer for selecting text
                if (!renaming) onTabPointerDown(e, id)
              }}
              onClick={() => choose(id)}
              onDoubleClick={() => {
                if (session?.kind === 'claude') startRename(id)
              }}
              onAuxClick={(e) => {
                if (e.button === 1) closeTab(id)
              }}
              onContextMenu={(e) => {
                e.preventDefault()
                openTabMenu(id, e.clientX, e.clientY)
              }}
            >
              {view.mark}
              <span className="tab-text">
                {renaming && session ? (
                  <RenameInput session={session} placeholder={view.topic ?? session.title} />
                ) : (
                  <span className="tab-title">{view.title}</span>
                )}
                <span className="tab-detail">{view.detail}</span>
              </span>
              {context !== undefined && (
                <span className={`tab-context ${usageView({ usedPct: context }, 0).tone}`} title={`Context window ${context}% used`}>
                  {context}%
                </span>
              )}
              <button
                className="tab-close"
                title="Close (Ctrl+Shift+W)"
                onClick={(e) => {
                  e.stopPropagation()
                  closeTab(id)
                }}
              >
                ×
              </button>
            </div>
          )
        })}
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
          side={side}
          onChoose={(id) => {
            setListOpen(false)
            choose(id)
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
