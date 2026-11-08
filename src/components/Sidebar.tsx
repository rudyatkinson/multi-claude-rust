import { isActiveRun } from '@shared/workflows'
import { closePanel, openPanel, setModal, toggleSidebar, useStore } from '../store'
import { AccountCard } from './AccountCard'
import { AccountMini } from './AccountMini'
import { useUnsaved } from '../library'
import { HistoryIcon, LibraryIcon, SidebarIcon, WorkflowIcon } from './icons'

const NO_CLAUDE = 'claude was not found. Set its path in Settings.'

/** The account list. Collapsed, it narrows to a strip of initials with small usage bars. */
export function Sidebar() {
  const profiles = useStore((s) => s.profiles)
  const claude = useStore((s) => s.claude)
  const appVersion = useStore((s) => s.appVersion)
  const ready = useStore((s) => s.ready)
  const collapsed = useStore((s) => s.sidebarCollapsed)
  const panel = useStore((s) => s.panel)
  const running = useStore((s) => s.runs.some(isActiveRun))
  const unsaved = useUnsaved((s) => Object.keys(s.items).length > 0)
  const versions = [claude.version && `Claude Code ${claude.version.replace(/\s*\(.*\)$/, '')}`, appVersion && `Multi Claude ${appVersion}`].filter(Boolean).join(' | ')

  return (
    <aside className={`sidebar${collapsed ? ' mini' : ''}`}>
      {ready && !claude.path && (
        <button className="alert" title={collapsed ? NO_CLAUDE : undefined} onClick={() => setModal({ type: 'settings' })}>
          {collapsed ? '!' : NO_CLAUDE}
        </button>
      )}

      <div className="cards">
        {profiles.map((profile) =>
          collapsed ? <AccountMini key={profile.id} profile={profile} /> : <AccountCard key={profile.id} profile={profile} />
        )}
      </div>

      <div className="sidebar-foot">
        <div className="sidebar-actions">
          <button
            className={collapsed ? 'icon-btn' : 'btn'}
            title={collapsed ? 'Add account' : undefined}
            disabled={!claude.path}
            onClick={() => setModal({ type: 'addAccount' })}
          >
            {collapsed ? '+' : '+ Add account'}
          </button>
          <button className="icon-btn" title={`${collapsed ? 'Expand' : 'Collapse'} sidebar (Ctrl+Shift+B)`} onClick={toggleSidebar}>
            <SidebarIcon />
          </button>
          <button
            className={`icon-btn${panel === 'workflows' ? ' on' : ''}${running ? ' live' : ''}`}
            title={running ? 'Workflows (one is running)' : 'Workflows: chain sessions across accounts'}
            onClick={() => (panel === 'workflows' ? closePanel() : openPanel('workflows'))}
          >
            <WorkflowIcon />
          </button>
          <button
            className={`icon-btn${panel === 'library' ? ' on' : ''}${unsaved ? ' unsaved' : ''}`}
            title={unsaved ? 'Library (something has unsaved changes)' : 'Library: the skills and agents Claude Code works with'}
            onClick={() => (panel === 'library' ? closePanel() : openPanel('library'))}
          >
            <LibraryIcon />
          </button>
          <button className="icon-btn" title="Find a past conversation (Ctrl+Shift+H)" onClick={() => setModal({ type: 'history' })}>
            <HistoryIcon />
          </button>
          <button className="icon-btn" title="Settings" onClick={() => setModal({ type: 'settings' })}>
            ⚙
          </button>
        </div>
        {!collapsed && versions && <div className="version">{versions}</div>}
      </div>
    </aside>
  )
}
