import { isActiveRun } from '@shared/workflows'
import { NEWEST_VERSION, useChangelogSeen } from '../changelog'
import { isPanelShown, setModal, toggleSidebar, togglePanel, useStore } from '../store'
import { AccountCard } from './AccountCard'
import { AccountMini } from './AccountMini'
import { useUnsaved } from '../library'
import { showUpdateBar, useUpdate } from '../updates'
import { HistoryIcon, LibraryIcon, SettingsIcon, SidebarIcon, SparkleIcon, WorkflowIcon } from './icons'

const NO_CLAUDE = 'claude was not found. Set its path in Settings.'

/** The account list. Collapsed, it narrows to a strip of initials with small usage bars. */
export function Sidebar() {
  const profiles = useStore((s) => s.profiles)
  const claude = useStore((s) => s.claude)
  const appVersion = useStore((s) => s.appVersion)
  const ready = useStore((s) => s.ready)
  const collapsed = useStore((s) => s.sidebarCollapsed)
  // on while the panel's tab is the one its area shows; pressing it again then closes the tab
  const workflowsOn = useStore((s) => isPanelShown(s.layout, 'workflows'))
  const libraryOn = useStore((s) => isPanelShown(s.layout, 'library'))
  const running = useStore((s) => s.runs.some(isActiveRun))
  const unsaved = useUnsaved((s) => Object.keys(s.items).length > 0)
  const news = useChangelogSeen((s) => NEWEST_VERSION !== null && s.seen !== NEWEST_VERSION)
  const update = useUpdate((s) => s.info)
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
        {!collapsed && (
          <button className="btn full" disabled={!claude.path} onClick={() => setModal({ type: 'addAccount' })}>
            + Add account
          </button>
        )}
        <div className="sidebar-actions">
          {collapsed && (
            <button className="icon-btn foot-btn" title="Add account" disabled={!claude.path} onClick={() => setModal({ type: 'addAccount' })}>
              +
            </button>
          )}
          <button className="icon-btn foot-btn" title={`${collapsed ? 'Expand' : 'Collapse'} sidebar (Ctrl+Shift+B)`} onClick={toggleSidebar}>
            <SidebarIcon />
          </button>
          <button
            className={`icon-btn foot-btn${workflowsOn ? ' on' : ''}${running ? ' live' : ''}`}
            title={running ? 'Workflows (one is running)' : 'Workflows: chain sessions across accounts'}
            onClick={() => togglePanel('workflows')}
          >
            <WorkflowIcon />
          </button>
          <button
            className={`icon-btn foot-btn${libraryOn ? ' on' : ''}${unsaved ? ' unsaved' : ''}`}
            title={unsaved ? 'Library (something has unsaved changes)' : 'Library: the skills and agents Claude Code works with'}
            onClick={() => togglePanel('library')}
          >
            <LibraryIcon />
          </button>
          <button className="icon-btn foot-btn" title="Find a past conversation (Ctrl+Shift+H)" onClick={() => setModal({ type: 'history' })}>
            <HistoryIcon />
          </button>
          {/* the dot: the newest version's changes have not been opened yet */}
          <button className={`icon-btn foot-btn${news ? ' news' : ''}`} title="What's new" onClick={() => setModal({ type: 'changelog' })}>
            <SparkleIcon />
          </button>
          <button className="icon-btn foot-btn" title="Settings" onClick={() => setModal({ type: 'settings' })}>
            <SettingsIcon />
          </button>
        </div>
        {!collapsed && (versions || update) && (
          <div className="version">
            {versions}
            {/* a newer version is known: shows its bar again after "Later" */}
            {update && (
              <button className="version-update" title={`Multi Claude ${update.version} is available`} onClick={showUpdateBar}>
                ↑ {update.version}
              </button>
            )}
          </div>
        )}
      </div>
    </aside>
  )
}
