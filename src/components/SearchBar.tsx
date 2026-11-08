import { useEffect, useRef, useState } from 'react'
import { closeSearch, useStore } from '../store'
import { clearSearch, onSearchResults, searchTerminal, type SearchResult } from '../terminal/terminalRegistry'

/** Finds text in what a terminal has printed so far. It sits over the terminal's top right corner. */
export function SearchBar({ id }: { id: string }) {
  const requests = useStore((s) => s.searchRequests)
  const [query, setQuery] = useState('')
  const [result, setResult] = useState<SearchResult | null>(null)
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => onSearchResults(id, setResult), [id])

  // also when the shortcut is pressed again while the bar is open
  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [requests])

  const find = (text: string, direction: 'next' | 'prev', incremental = false): void => {
    if (text) {
      searchTerminal(id, text, direction, incremental)
    } else {
      clearSearch(id)
      setResult(null)
    }
  }

  // the highlights are capped; past the cap there is a count but no position
  const count = !query ? '' : !result || result.count === 0 ? 'No results' : result.index < 0 ? `${result.count}` : `${result.index + 1}/${result.count}`

  return (
    <div className="search-bar">
      <input
        ref={input}
        value={query}
        placeholder="Find"
        spellCheck={false}
        onChange={(e) => {
          setQuery(e.target.value)
          find(e.target.value, 'next', true)
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            find(query, e.shiftKey ? 'prev' : 'next')
          } else if (e.key === 'Escape') {
            e.preventDefault()
            closeSearch()
          }
        }}
      />
      <span className="search-count">{count}</span>
      <button title="Previous (Shift+Enter)" disabled={!query} onClick={() => find(query, 'prev')}>
        ↑
      </button>
      <button title="Next (Enter)" disabled={!query} onClick={() => find(query, 'next')}>
        ↓
      </button>
      <button title="Close (Esc)" onClick={() => closeSearch()}>
        ×
      </button>
    </div>
  )
}
