import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { Unicode11Addon } from '@xterm/addon-unicode11'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { Terminal } from '@xterm/xterm'
import type { IpcEvents } from '@shared/ipc'
import { wakesOnKey } from '@shared/sleep'
import { isTyping } from '@shared/terminalInput'
import { mc } from '../api'

export interface TerminalOptions {
  fontSize: number
  platform: string
  osBuild?: number
  /** The program in the terminal changed the terminal title. */
  onTitle(title: string): void
}

/** Position of the highlighted match among all matches; `index` is -1 when there are too many to count. */
export interface SearchResult {
  index: number
  count: number
}

interface Entry {
  term: Terminal
  fit: FitAddon
  host: HTMLElement
  observer: ResizeObserver
  /** Output up to this offset is already on screen. */
  lastSeq: number
  attached: boolean
  queue: IpcEvents['pty:data'][]
  /** The GPU renderer, held only while the terminal is on screen. */
  webgl?: WebglAddon
  fitTimer?: ReturnType<typeof setTimeout>
  /** Loaded with the first search in this terminal. */
  search?: SearchAddon
  onSearchResult?: (result: SearchResult) => void
  /** Set while the session sleeps: what is typed is not sent, the first key wakes it instead. */
  onWake?: () => void
}

// xterm instances live outside React so that re-renders never recreate a terminal.
const entries = new Map<string, Entry>()

const THEME = {
  background: '#141413',
  foreground: '#e8e6dc',
  cursor: '#d97757',
  cursorAccent: '#141413',
  selectionBackground: '#4a4843'
}

const FIT_DELAY_MS = 40

// the result count is only reported for searches that draw their matches
const SEARCH_DECORATIONS = {
  matchBackground: '#5c4a1f',
  matchOverviewRuler: '#e0b050',
  activeMatchBackground: '#a8552f',
  activeMatchColorOverviewRuler: '#d97757'
}

function openUrl(uri: string): void {
  void mc.invoke('shell:openExternal', uri).catch(() => {})
}

async function paste(id: string, term: Terminal, platform: string): Promise<void> {
  const { text, hasImage } = await mc.invoke('clipboard:read')
  if (text) term.paste(text)
  // No text: let Claude Code read the image itself through its own paste key.
  else if (hasImage) mc.send('pty:write', { id, data: platform === 'win32' ? '\x1bv' : '\x16' })
}

function copySelection(term: Terminal): void {
  void mc.invoke('clipboard:writeText', term.getSelection())
  term.clearSelection()
}

/**
 * Keys the terminal treats differently from xterm's default. The app's own shortcuts never get
 * here: the window takes them first (see `matchShortcut`).
 */
function keyHandler(id: string, term: Terminal, opts: TerminalOptions): (e: KeyboardEvent) => boolean {
  return (e) => {
    const key = e.key.toLowerCase()
    const wake = entries.get(id)?.onWake
    // asleep: the first key wakes the session and goes nowhere; copying what it showed, a modifier
    // on its own (on the way to Ctrl+Tab) and the system's Cmd keys leave it asleep
    const copying = opts.platform !== 'darwin' && e.ctrlKey && !e.altKey && key === 'c' && (e.shiftKey || term.hasSelection())
    if (wake && !copying && !e.metaKey) {
      if (e.type === 'keydown' && wakesOnKey(e.key)) wake()
      e.preventDefault()
      return false
    }
    if (e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey && e.key === 'Enter') {
      // newline without submitting, the sequence Claude Code's terminal setup binds to Shift+Enter
      if (e.type === 'keydown') mc.send('pty:write', { id, data: '\x1b\r' })
      e.preventDefault()
      return false
    }
    if (e.type !== 'keydown') return true
    // macOS copies and pastes with Cmd through the Edit menu
    if (opts.platform === 'darwin' || !e.ctrlKey || e.altKey) return true
    if (key === 'c' && (e.shiftKey || term.hasSelection())) {
      e.preventDefault()
      if (term.hasSelection()) copySelection(term)
      return false
    }
    if (key === 'v') {
      e.preventDefault()
      void paste(id, term, opts.platform)
      return false
    }
    return true
  }
}

export function mountTerminal(id: string, host: HTMLElement, opts: TerminalOptions): void {
  if (entries.has(id)) return
  const term = new Terminal({
    allowProposedApi: true,
    cursorBlink: true,
    fontFamily: "'Cascadia Mono', 'Cascadia Code', Consolas, Menlo, 'DejaVu Sans Mono', monospace",
    fontSize: opts.fontSize,
    scrollback: 10000,
    theme: THEME,
    linkHandler: { activate: (_event, uri) => openUrl(uri) },
    windowsPty: opts.platform === 'win32' ? { backend: 'conpty', buildNumber: opts.osBuild } : undefined
  })
  const fit = new FitAddon()
  term.loadAddon(fit)
  term.loadAddon(new WebLinksAddon((_event, uri) => openUrl(uri)))
  const unicode = new Unicode11Addon()
  term.loadAddon(unicode)
  term.unicode.activeVersion = '11'
  term.open(host)

  // a window being resized reports many sizes; the program in the terminal hears of the last one
  const observer = new ResizeObserver(() => {
    const current = entries.get(id)
    if (!current) return
    clearTimeout(current.fitTimer)
    current.fitTimer = setTimeout(() => fitTerminal(id), FIT_DELAY_MS)
  })
  const entry: Entry = { term, fit, host, observer, lastSeq: 0, attached: false, queue: [] }
  entries.set(id, entry)

  term.attachCustomKeyEventHandler(keyHandler(id, term, opts))
  term.onData((data) => {
    // asleep: nothing runs to take it; what a person typed (an input method, say) wakes it, the
    // terminal's own reports (its focus coming back, for one) do not
    if (entry.onWake) {
      if (isTyping(data)) entry.onWake()
      return
    }
    mc.send('pty:write', { id, data })
  })
  term.onTitleChange((title) => opts.onTitle(title))
  term.onResize(({ cols, rows }) => mc.send('pty:resize', { id, cols, rows }))
  host.addEventListener('contextmenu', (e) => {
    e.preventDefault()
    if (term.hasSelection()) copySelection(term)
    else if (entry.onWake) entry.onWake()
    else void paste(id, term, opts.platform)
  })
  observer.observe(host)
  fitTerminal(id)

  void mc.invoke('sessions:attach', id).then((res) => {
    if (!entries.has(id)) return
    if (res) {
      term.write(res.replay)
      entry.lastSeq = res.seq
    }
    entry.attached = true
    for (const event of entry.queue) writeData(entry, event)
    entry.queue = []
    mc.send('pty:resize', { id, cols: term.cols, rows: term.rows })
  })
}

function writeData(entry: Entry, event: IpcEvents['pty:data']): void {
  if (event.seq <= entry.lastSeq) return
  entry.lastSeq = event.seq
  entry.term.write(event.data)
}

export function handlePtyData(event: IpcEvents['pty:data']): void {
  const entry = entries.get(event.id)
  if (!entry) return
  if (entry.attached) writeData(entry, event)
  else entry.queue.push(event)
}

export function fitTerminal(id: string): void {
  const entry = entries.get(id)
  // a hidden tab has no size; it is fitted when it becomes visible
  if (!entry || entry.host.offsetWidth === 0 || entry.host.offsetHeight === 0) return
  try {
    entry.fit.fit()
  } catch {
    // renderer not ready yet
  }
}

export function focusTerminal(id: string): void {
  entries.get(id)?.term.focus()
}

/**
 * Tells a terminal that its session sleeps (`onWake` is what a key or a click in it calls) or is
 * awake again (null). While it sleeps it keeps what it shows and sends nothing.
 */
export function setTerminalAsleep(id: string, onWake: (() => void) | null): void {
  const entry = entries.get(id)
  if (entry) entry.onWake = onWake ?? undefined
}

/**
 * Tells a terminal whether it is on screen. Browsers allow only a handful of WebGL contexts, so
 * a terminal draws with the GPU while it is shown and gives its context back when it is not.
 * After a lost context it stays on the DOM renderer until it is shown the next time.
 */
export function setTerminalVisible(id: string, visible: boolean): void {
  const entry = entries.get(id)
  if (!entry) return
  if (!visible) {
    entry.webgl?.dispose()
    entry.webgl = undefined
    return
  }
  if (entry.webgl) return
  try {
    const webgl = new WebglAddon()
    webgl.onContextLoss(() => {
      webgl.dispose()
      if (entry.webgl === webgl) entry.webgl = undefined
    })
    entry.term.loadAddon(webgl)
    entry.webgl = webgl
  } catch {
    // DOM renderer
  }
}

/** Types text into a terminal as a paste, so the program in it sees it as one piece. */
export function pasteText(id: string, text: string): void {
  entries.get(id)?.term.paste(text)
}

export function disposeTerminal(id: string): void {
  const entry = entries.get(id)
  if (!entry) return
  clearTimeout(entry.fitTimer)
  entry.observer.disconnect()
  entry.term.dispose()
  entries.delete(id)
}

export function disposeMissing(liveIds: Set<string>): void {
  for (const id of [...entries.keys()]) {
    if (!liveIds.has(id)) disposeTerminal(id)
  }
}

export function setFontSize(size: number): void {
  for (const [id, entry] of entries) {
    entry.term.options.fontSize = size
    fitTerminal(id)
  }
}

function searchAddon(entry: Entry): SearchAddon {
  if (!entry.search) {
    entry.search = new SearchAddon()
    entry.term.loadAddon(entry.search)
    entry.search.onDidChangeResults(({ resultIndex, resultCount }) => entry.onSearchResult?.({ index: resultIndex, count: resultCount }))
  }
  return entry.search
}

/**
 * Highlights the matches of `query` in a terminal's buffer and moves to the next or the previous
 * one. `incremental` keeps the current match while it still fits, for searching as one types.
 */
export function searchTerminal(id: string, query: string, direction: 'next' | 'prev', incremental = false): void {
  const entry = entries.get(id)
  if (!entry) return
  const search = searchAddon(entry)
  if (direction === 'next') search.findNext(query, { incremental, decorations: SEARCH_DECORATIONS })
  else search.findPrevious(query, { decorations: SEARCH_DECORATIONS })
}

/**
 * Removes the highlights. The current match is a selection as far as the terminal is concerned,
 * and with a selection Ctrl+C copies instead of interrupting, so this must run when a search ends.
 */
export function clearSearch(id: string): void {
  const entry = entries.get(id)
  if (!entry?.search) return
  entry.search.clearDecorations()
  entry.term.clearSelection()
}

/** Reports how many matches the running search has. Returns a function that stops the reports. */
export function onSearchResults(id: string, listener: (result: SearchResult) => void): () => void {
  const entry = entries.get(id)
  if (!entry) return () => {}
  entry.onSearchResult = listener
  return () => {
    if (entry.onSearchResult === listener) entry.onSearchResult = undefined
  }
}
