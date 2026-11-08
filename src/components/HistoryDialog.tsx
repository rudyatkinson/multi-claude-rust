import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { searchConversations, type ConversationEntry } from '@shared/conversations'
import { errorMessage, mc } from '../api'
import { ago, splitPath } from '../format'
import { openConversation, setModal, useStore } from '../store'
import { Dialog } from './Dialogs'

const SHOWN = 100

/** Finds a past conversation of any account and opens it in a tab. */
export function HistoryDialog() {
  const profiles = useStore((s) => s.profiles)
  const [entries, setEntries] = useState<ConversationEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  /** Null shows every account. */
  const [account, setAccount] = useState<string | null>(null)
  const [selected, setSelected] = useState(0)
  const list = useRef<HTMLDivElement>(null)

  useEffect(() => {
    mc.invoke('conversations:list').then(setEntries, (err) => setError(errorMessage(err)))
  }, [])

  const names = useMemo(() => Object.fromEntries(profiles.map((p) => [p.id, p.name])), [profiles])
  const withHistory = useMemo(() => profiles.filter((p) => entries?.some((e) => e.profileId === p.id)), [profiles, entries])
  const results = useMemo(() => {
    const pool = (entries ?? []).filter((e) => account === null || e.profileId === account)
    return searchConversations(pool, query, names)
  }, [entries, account, query, names])
  const shown = results.slice(0, SHOWN)
  const now = Date.now()

  // the list changes under the selection as one types
  useEffect(() => setSelected(0), [query, account])
  useEffect(() => {
    list.current?.querySelector('.history-row.selected')?.scrollIntoView({ block: 'nearest' })
  }, [selected])

  const close = (): void => setModal(null)
  const open = (entry: ConversationEntry | undefined): void => {
    if (!entry) return
    close()
    void openConversation(entry)
  }

  return (
    <Dialog title="Conversations" onClose={close}>
      <input
        className="history-search"
        autoFocus
        value={query}
        placeholder="Search titles, messages, folders and accounts"
        spellCheck={false}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault()
            setSelected((at) => Math.min(Math.max(at + (e.key === 'ArrowDown' ? 1 : -1), 0), Math.max(shown.length - 1, 0)))
          } else if (e.key === 'Enter') {
            e.preventDefault()
            open(shown[selected])
          }
        }}
      />
      {withHistory.length > 1 && (
        <div className="chips">
          <button className={`chip${account === null ? ' selected' : ''}`} onClick={() => setAccount(null)}>
            All accounts
          </button>
          {withHistory.map((p) => (
            <button key={p.id} className={`chip${account === p.id ? ' selected' : ''}`} style={{ '--accent': p.color } as CSSProperties} onClick={() => setAccount(p.id)}>
              <span className="dot" />
              {p.name}
            </button>
          ))}
        </div>
      )}

      {error && <p className="note warn">{error}</p>}
      {!entries && !error && <p className="note">Reading the conversations…</p>}
      {entries && shown.length === 0 && <p className="note">{entries.length === 0 ? 'No conversations yet.' : 'Nothing matches.'}</p>}
      <div className="history-list" ref={list}>
        {shown.map((entry, index) => {
          const profile = profiles.find((p) => p.id === entry.profileId)
          const { name, parent } = splitPath(entry.cwd)
          // the first message says more than a title that only repeats it
          const detail = entry.firstPrompt && !entry.firstPrompt.startsWith(entry.title) ? entry.firstPrompt : entry.lastPrompt
          return (
            <button
              key={`${entry.profileId}:${entry.id}`}
              className={`history-row${index === selected ? ' selected' : ''}`}
              style={{ '--accent': profile?.color ?? '#888' } as CSSProperties}
              title={entry.cwd}
              onMouseEnter={() => setSelected(index)}
              onClick={() => open(entry)}
            >
              <span className="history-title">{entry.title}</span>
              {detail && detail !== entry.title && <span className="history-detail">{detail}</span>}
              <span className="history-meta">
                <span className="dot" />
                {profile?.name ?? 'Deleted account'} · {name}
                <span className="history-parent">{parent}</span>
                <span className="spacer" />
                {ago(now - entry.updatedAt)}
              </span>
            </button>
          )
        })}
      </div>
      {results.length > SHOWN && <p className="note">Showing the {SHOWN} most recent of {results.length}. Type to narrow it down.</p>}
      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          Close
        </button>
      </div>
    </Dialog>
  )
}
