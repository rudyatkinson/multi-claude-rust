import { useEffect, useRef, useState } from 'react'
import { LIBRARY_SECTIONS, sectionHooks, useUnsaved } from '../library'
import { closePanel, setLibrarySection, useStore } from '../store'
import { AgentsSection } from './AgentsSection'
import { RefreshIcon } from './icons'
import { InstructionsSection } from './InstructionsSection'
import { MemorySection } from './MemorySection'
import { SkillsSection } from './SkillsSection'

/**
 * The Library: what Claude Code reads from disk, a section for each kind. It lies over the
 * terminals, which keep running. A section keeps what is typed in it when another one shows or
 * the panel closes; only quitting asks about it.
 */
export function LibraryPanel() {
  const section = useStore((s) => s.librarySection)
  const unsaved = useUnsaved((s) => s.items)
  const [reading, setReading] = useState(false)
  const root = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const { modal, confirm, librarySection } = useStore.getState()
      // a dialog on top takes the keys for itself
      if (modal || confirm) return
      if (e.key === 'Escape') closePanel()
      else if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 's') {
        e.preventDefault()
        sectionHooks(librarySection)?.save()
      }
    }
    // coming back to the window: a Claude session may have worked on the files meanwhile
    const onFocus = (): void => void sectionHooks(useStore.getState().librarySection)?.refresh()
    document.addEventListener('keydown', onKey)
    window.addEventListener('focus', onFocus)
    return () => {
      document.removeEventListener('keydown', onKey)
      window.removeEventListener('focus', onFocus)
    }
  }, [])

  // the terminal underneath must not keep the keyboard
  useEffect(() => {
    if (!root.current?.contains(document.activeElement)) root.current?.focus()
  }, [section])

  const refresh = (): void => {
    setReading(true)
    void sectionHooks(section)
      ?.refresh()
      .finally(() => setReading(false))
  }

  return (
    <div className="wf-panel" ref={root} tabIndex={-1}>
      <header className="wf-head">
        <span className="wf-title">Library</span>
        <div className="segmented lib-sections">
          {LIBRARY_SECTIONS.map((s) => (
            <button key={s.id} type="button" className={s.id === section ? 'selected' : ''} title={s.blurb} onClick={() => setLibrarySection(s.id)}>
              {s.label}
              {unsaved[s.id] && <span className="sk-dot" title="Unsaved changes" />}
            </button>
          ))}
        </div>
        <span className="muted">{LIBRARY_SECTIONS.find((s) => s.id === section)?.blurb}</span>
        <span className="spacer" />
        <button className={`icon-btn${reading ? ' spinning' : ''}`} title="Read the folders again" onClick={refresh}>
          <RefreshIcon />
        </button>
        <button className="icon-btn" title="Close (Esc)" onClick={closePanel}>
          ×
        </button>
      </header>
      {section === 'skills' && <SkillsSection />}
      {section === 'agents' && <AgentsSection />}
      {section === 'memory' && <MemorySection />}
      {section === 'instructions' && <InstructionsSection />}
    </div>
  )
}
