import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { pinnedAccount } from '@shared/folders'
import { defaultProfileId } from '@shared/openTarget'
import type { ProfileView, ResumeMode } from '@shared/types'
import { mc } from '../api'
import { splitPath, usageView } from '../format'
import { useDismiss } from '../hooks'
import { attempt, closeNewSession, startSession, useStore } from '../store'
import { ResumeChoice } from './ResumeChoice'

const MARGIN = 6

/**
 * Picks the account and the folder of a new session. Opens below the "+" of the area the session
 * is for, and stays inside the window however narrow that area is.
 */
export function NewSessionPopover({ groupId }: { groupId: string }) {
  const profiles = useStore((s) => s.profiles)
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const recentFolders = useStore((s) => s.recentFolders)
  const folderAccounts = useStore((s) => s.folderAccounts)
  const platform = useStore((s) => s.platform)
  const homeDir = useStore((s) => s.homeDir)
  const accounts = profiles.filter((p) => p.account?.loggedIn)
  const [initial] = useState(() => defaultProfileId(profiles))
  // the account chosen here, if one was: it then also wins over the account a folder is pinned to
  const [picked, setPicked] = useState<string | null>(null)
  const [resume, setResume] = useState<ResumeMode | undefined>(undefined)
  const [external, setExternal] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)

  useLayoutEffect(() => {
    const el = ref.current
    const button = document.querySelector(`[data-group="${groupId}"] .tab-new`)
    if (!el) return
    const anchor = button?.getBoundingClientRect() ?? { left: MARGIN, right: MARGIN, bottom: MARGIN }
    // the "+" of a column strip is at its foot: the picker opens beside it, upwards
    const side = button?.closest('.tabbar.side') != null
    const { width, height } = el.getBoundingClientRect()
    setPos({
      left: Math.max(MARGIN, Math.min(side ? anchor.right + 4 : anchor.left, window.innerWidth - width - MARGIN)),
      top: Math.max(MARGIN, Math.min(side ? anchor.bottom - height : anchor.bottom + 4, window.innerHeight - height - MARGIN))
    })
  }, [groupId])

  // the "+" buttons toggle the picker themselves
  useDismiss(ref, closeNewSession, '.tab-new')
  // take the keyboard from the terminal so that Tab, Enter and Escape work here; only once the
  // popover is placed, because nothing can be focused while it is still hidden
  const placed = pos !== null
  useEffect(() => {
    if (placed) ref.current?.querySelector<HTMLElement>('.folder, .btn')?.focus()
  }, [placed])

  // the chosen account may log out while the popover is open
  const selected = accounts.find((p) => p.id === (picked ?? initial)) ?? accounts[0]

  const pinOf = (cwd: string): ProfileView | undefined => {
    if (picked) return undefined
    const id = pinnedAccount(folderAccounts, cwd, platform, accounts)
    return accounts.find((p) => p.id === id)
  }

  // a launcher script for an external terminal would have to contain the token
  const canOpenExternally = selected?.auth !== 'token'

  const open = (cwd: string): void => {
    const account = pinOf(cwd) ?? selected
    if (!account) return
    closeNewSession()
    if (external && account.auth !== 'token') void attempt(mc.invoke('sessions:openExternal', { profileId: account.id, cwd, resume }))
    else void startSession(account.id, cwd, resume, { groupId })
  }

  const browse = async (): Promise<void> => {
    const cwd = await attempt(mc.invoke('dialog:pickFolder', recentFolders[0] ?? homeDir))
    if (cwd) open(cwd)
  }

  return (
    <div className="popover new-session" ref={ref} style={pos ?? { visibility: 'hidden' }}>
      <div className="popover-title">Account</div>
      {accounts.length === 0 && <div className="muted small">Log in to an account on the left first.</div>}
      <div className="chips">
        {accounts.map((account) => {
          // the 5-hour limit is what decides whether the account can take another session now;
          // a plan without one (Ultra) shows its weekly limit instead
          const state = usage[account.id]
          const session = state?.fiveHour
          const view = session ? usageView(session, now) : state?.sevenDay ? usageView(state.sevenDay, now) : null
          const limitName = session ? '5-hour' : 'weekly'
          return (
            <button
              key={account.id}
              className={`chip${account.id === selected?.id ? ' selected' : ''}`}
              style={{ '--accent': account.color } as CSSProperties}
              title={[account.account?.email, view && `${limitName} limit ${view.pct}% used`].filter(Boolean).join('\n')}
              onClick={() => setPicked(account.id)}
            >
              <span className="dot" />
              {account.name}
              {view && <span className={`chip-usage ${view.tone}`}>{view.pct}%</span>}
            </button>
          )
        })}
      </div>

      <div className="popover-title">Conversation</div>
      <ResumeChoice value={resume} onChange={setResume} />

      <div className="popover-title">Folder</div>
      <div className="folder-list">
        {recentFolders.slice(0, 8).map((folder) => {
          const { name, parent } = splitPath(folder)
          const pin = pinOf(folder)
          return (
            <button
              key={folder}
              className="folder"
              title={pin ? `${folder}\nAlways opens with ${pin.name}; choose an account above to use another` : folder}
              disabled={!selected}
              onClick={() => open(folder)}
            >
              <span className="folder-name">
                {name}
                {pin && (
                  <span className="folder-account" style={{ '--accent': pin.color } as CSSProperties}>
                    <span className="dot" />
                    {pin.name}
                  </span>
                )}
              </span>
              <span className="folder-parent">{parent}</span>
            </button>
          )
        })}
        {recentFolders.length === 0 && <div className="muted small">No folders used yet.</div>}
      </div>
      <button className="btn full" disabled={!selected} onClick={() => void browse()}>
        Browse…
      </button>
      <label className="check" title={canOpenExternally ? undefined : 'Not available for token accounts'}>
        <input
          type="checkbox"
          checked={external && canOpenExternally}
          disabled={!canOpenExternally}
          onChange={(e) => setExternal(e.target.checked)}
        />
        Open in an external terminal
      </label>
    </div>
  )
}
