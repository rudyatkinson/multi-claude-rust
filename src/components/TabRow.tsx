import { useEffect, useRef, type CSSProperties, type PointerEvent, type ReactNode } from 'react'
import { panelKindOf, type Group, type PanelKind } from '@shared/layout'
import { tabMark } from '@shared/turn'
import type { ProfileView, SessionInfo } from '@shared/types'
import { isActiveRun } from '@shared/workflows'
import { tabLabel, usageView } from '../format'
import { useUnsaved } from '../library'
import { closeTab, finishRename, openTabMenu, refocus, setActive, startRename, useStore, type TabState } from '../store'
import { LibraryIcon, WorkflowIcon } from './icons'

/** What the tab of a panel says about it. */
const PANEL_TABS: Record<PanelKind, { title: string; detail: string; tooltip: string; icon: () => ReactNode }> = {
  workflows: { title: 'Workflows', detail: 'designer', tooltip: 'Workflows: chain sessions across accounts', icon: WorkflowIcon },
  library: { title: 'Library', detail: 'skills · agents · memory · instructions', tooltip: 'Library: the skills and agents Claude Code works with', icon: LibraryIcon }
}

/** What a tab shows: in its strip, in the sidebar's list, and in the list of all the tabs of its area. */
export interface TabView {
  id: string
  /** In front of the title: the state of a session, the icon of a panel. */
  mark: ReactNode
  title: string
  detail: string
  tooltip: string
  /** The account's colour; a panel has none. */
  accent: string
  /** The account's name; a panel has none. */
  account?: string
  /** `exited`, `asleep`, `attention`, `panel`. */
  classes: string[]
  session?: SessionInfo
  /** For the session's own name being typed in place of the title. */
  topic?: string | null
}

/** What the views of tabs are made of. */
export interface TabViewData {
  sessions: SessionInfo[]
  profiles: ProfileView[]
  tabStatus: Record<string, TabState>
  attention: Record<string, true>
  /** A workflow run is under way. */
  running: boolean
  /** Something in the Library has unsaved changes. */
  unsaved: boolean
}

export function useTabViewData(): TabViewData {
  const sessions = useStore((s) => s.sessions)
  const profiles = useStore((s) => s.profiles)
  const tabStatus = useStore((s) => s.tabStatus)
  const attention = useStore((s) => s.attention)
  const running = useStore((s) => s.runs.some(isActiveRun))
  const unsaved = useUnsaved((s) => Object.keys(s.items).length > 0)
  return { sessions, profiles, tabStatus, attention, running, unsaved }
}

/** The views of an area's tabs, in tab order; tabs whose session or panel is not known are left out. */
export function viewsOf(group: Group, { sessions, profiles, tabStatus, attention, running, unsaved }: TabViewData): TabView[] {
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
    const asleep = session.status === 'sleeping'
    const sign = tabMark({
      exited: session.status === 'exited',
      asleep,
      sleptAt: session.sleptAt,
      turn: session.turn,
      busy: status?.busy === true,
      busySince: status?.busySince
    })
    const topic = status?.topic ?? null
    const title = tabLabel(session, topic)
    return [
      {
        id,
        mark: <span className={`tab-state ${sign.state}`} title={sign.label} />,
        title,
        // the folder moves to the second line once the tab is called something else
        detail: [title === session.title ? null : session.title, profile?.name, session.model].filter(Boolean).join(' · '),
        tooltip: [asleep ? sign.label : null, session.name, topic, session.cwd, profile?.name].filter(Boolean).join('\n'),
        accent: profile?.color ?? '#888',
        account: profile?.name,
        classes: [sign.state === 'ended' && 'exited', asleep && 'asleep', attention[session.id] && 'attention'].filter((c): c is string => !!c),
        session,
        topic
      }
    ]
  })
}

/** The views of an area's tabs (see `viewsOf`). */
export function useTabViews(group: Group): TabView[] {
  return viewsOf(group, useTabViewData())
}

/** A tab was chosen: it comes to the front, also when it was there already and only the keyboard had left it. */
export function chooseTab(id: string): void {
  setActive(id)
  refocus()
}

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

interface RowProps {
  view: TabView
  /** The tab its area shows. */
  active: boolean
  /** A dragged tab would land in front of it or behind it. */
  drop: 'before' | 'after' | null
  /** A button went down on the tab: it may become a drag (see `tabDrag.ts`). */
  onDragStart(e: PointerEvent<HTMLElement>): void
}

/**
 * One tab: its mark, title and detail, the context window's fill and a close button. A click
 * brings it to the front, a double-click renames a session, the middle button closes it and the
 * right one opens its menu. The same in an area's strip and in the sidebar's list.
 */
export function TabRow({ view, active, drop, onDragStart }: RowProps) {
  const renamingId = useStore((s) => s.renamingId)
  const { id, session } = view
  const renaming = session !== undefined && renamingId === id
  const context = session?.contextPct
  const classes = ['tab', active && 'active', ...view.classes, drop && `drop-${drop}`].filter(Boolean).join(' ')
  return (
    <div
      data-id={id}
      className={classes}
      style={{ '--accent': view.accent } as CSSProperties}
      title={renaming ? undefined : view.tooltip}
      onPointerDown={(e) => {
        // the name being typed takes the pointer for selecting text
        if (!renaming) onDragStart(e)
      }}
      onClick={() => chooseTab(id)}
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
        {renaming && session ? <RenameInput session={session} placeholder={view.topic ?? session.title} /> : <span className="tab-title">{view.title}</span>}
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
}
