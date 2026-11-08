import { useEffect, useRef, type CSSProperties, type PointerEvent } from 'react'
import type { Group, Rect } from '@shared/layout'
import { tabMark } from '@shared/turn'
import type { SessionInfo } from '@shared/types'
import { tabLabel, usageView } from '../format'
import { closeNewSession, closeSession, finishRename, openNewSession, openTabMenu, setActive, startRename, useStore } from '../store'
import { focusTerminal } from '../terminal/terminalRegistry'

/** Where a dragged tab would land in a strip: in front of or behind one of its tabs. */
export interface StripMark {
  tabId: string
  side: 'before' | 'after'
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

interface Props {
  group: Group
  rect: Rect
  /** This area has the keyboard. */
  focused: boolean
  mark: StripMark | null
  /** A button went down on a tab: it may become a drag (see `Workspace`). */
  onTabPointerDown(e: PointerEvent<HTMLElement>, id: string): void
}

/** The tab strip of one area: its sessions, and a "+" that opens a new one in it. */
export function GroupTabs({ group, rect, focused, mark, onTabPointerDown }: Props) {
  const sessions = useStore((s) => s.sessions)
  const profiles = useStore((s) => s.profiles)
  const tabStatus = useStore((s) => s.tabStatus)
  const attention = useStore((s) => s.attention)
  const renamingId = useStore((s) => s.renamingId)
  const pickerHere = useStore((s) => s.newSession?.groupId === group.id)
  const claudeFound = useStore((s) => s.claude.path !== null)
  const strip = useRef<HTMLDivElement>(null)

  // a tab chosen with the keyboard may be scrolled out of the strip
  useEffect(() => {
    if (group.active) strip.current?.querySelector(`[data-id="${group.active}"]`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [group.active])

  return (
    <div className={`tabbar${focused ? ' focused' : ''}`} data-group={group.id} style={{ left: rect.x, top: rect.y, width: rect.w }}>
      <div className="tabs" ref={strip}>
        {group.tabs.map((id) => {
          const session = sessions.find((s) => s.id === id)
          if (!session) return null
          const profile = profiles.find((p) => p.id === session.profileId)
          const status = tabStatus[session.id]
          const sign = tabMark({ exited: session.status === 'exited', turn: session.turn, busy: status?.busy === true, busySince: status?.busySince })
          const topic = status?.topic ?? null
          const label = tabLabel(session, topic)
          // the folder moves to the second line once the tab is called something else
          const detail = [label === session.title ? null : session.title, profile?.name, session.model].filter(Boolean).join(' · ')
          const context = session.contextPct
          const renaming = renamingId === session.id
          const classes = [
            'tab',
            session.id === group.active && 'active',
            sign.state === 'ended' && 'exited',
            attention[session.id] && 'attention',
            mark?.tabId === session.id && `drop-${mark.side}`
          ]
            .filter(Boolean)
            .join(' ')
          return (
            <div
              key={session.id}
              data-id={session.id}
              className={classes}
              style={{ '--accent': profile?.color ?? '#888' } as CSSProperties}
              title={renaming ? undefined : [session.name, topic, session.cwd, profile?.name].filter(Boolean).join('\n')}
              onPointerDown={(e) => {
                // the name being typed takes the pointer for selecting text
                if (!renaming) onTabPointerDown(e, session.id)
              }}
              onClick={() => {
                setActive(session.id)
                // also when the tab was in front already and only the keyboard had left the terminal
                focusTerminal(session.id)
              }}
              onDoubleClick={() => {
                if (session.kind === 'claude') startRename(session.id)
              }}
              onAuxClick={(e) => {
                if (e.button === 1) void closeSession(session.id)
              }}
              onContextMenu={(e) => {
                e.preventDefault()
                openTabMenu(session.id, e.clientX, e.clientY)
              }}
            >
              <span className={`tab-state ${sign.state}`} title={sign.label} />
              <span className="tab-text">
                {renaming ? <RenameInput session={session} placeholder={topic ?? session.title} /> : <span className="tab-title">{label}</span>}
                <span className="tab-detail">{detail}</span>
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
                  void closeSession(session.id)
                }}
              >
                ×
              </button>
            </div>
          )
        })}
      </div>
      <button
        className="tab-new"
        disabled={!claudeFound}
        title="New session here (Ctrl+Shift+T)"
        onClick={() => (pickerHere ? closeNewSession() : openNewSession(group.id))}
      >
        +
      </button>
    </div>
  )
}
