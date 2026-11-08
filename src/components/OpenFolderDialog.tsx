import { useState } from 'react'
import type { Place } from '@shared/layout'
import type { ResumeMode } from '@shared/types'
import { splitPath } from '../format'
import { setFolderAccount, setModal, startSession, useStore } from '../store'
import { AccountChoices } from './AccountChoices'
import { Dialog } from './Dialogs'
import { ResumeChoice } from './ResumeChoice'

/** Asks which account should open a folder that came from the Explorer context menu. */
export function OpenFolderDialog({ cwd, place }: { cwd: string; place?: Place }) {
  const profiles = useStore((s) => s.profiles)
  const [resume, setResume] = useState<ResumeMode | undefined>(undefined)
  const [always, setAlways] = useState(false)

  const close = (): void => setModal(null)
  const choose = (profileId: string): void => {
    close()
    if (always) void setFolderAccount(cwd, profileId)
    void startSession(profileId, cwd, resume, place)
  }

  const accounts = profiles.filter((p) => p.account?.loggedIn)
  const checking = profiles.some((p) => !p.account)
  const { name, parent } = splitPath(cwd)

  return (
    <Dialog title="Open with which account?" onClose={close}>
      <div className="open-folder-path" title={cwd}>
        <span className="open-folder-name">{name}</span>
        <span className="muted small">{parent}</span>
      </div>

      <AccountChoices accounts={accounts} onChoose={choose} />
      {accounts.length === 0 && <p className="note">{checking ? 'Checking accounts…' : 'No account is logged in.'}</p>}

      <div className="field">
        <span>Conversation</span>
        <ResumeChoice value={resume} onChange={setResume} />
      </div>
      <label className="check" title="Undo it from the tab's right-click menu">
        <input type="checkbox" checked={always} onChange={(e) => setAlways(e.target.checked)} />
        Always use this account for this folder
      </label>
      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          Cancel
        </button>
      </div>
    </Dialog>
  )
}
