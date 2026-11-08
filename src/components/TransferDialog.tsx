import { useEffect } from 'react'
import { tabLabel } from '../format'
import { setModal, transferSession, useStore } from '../store'
import { AccountChoices } from './AccountChoices'
import { Dialog } from './Dialogs'

/** Picks the account a tab's conversation continues on. */
export function TransferDialog({ sessionId }: { sessionId: string }) {
  const session = useStore((s) => s.sessions.find((x) => x.id === sessionId))
  const profiles = useStore((s) => s.profiles)
  const status = useStore((s) => s.tabStatus[sessionId])

  const close = (): void => setModal(null)

  // the tab was closed while the dialog was open
  useEffect(() => {
    if (!session) setModal(null)
  }, [session])
  if (!session) return null

  const source = profiles.find((p) => p.id === session.profileId)
  const others = profiles.filter((p) => p.id !== session.profileId && p.account?.loggedIn)
  const choose = (profileId: string): void => {
    close()
    void transferSession(sessionId, profileId)
  }

  return (
    <Dialog title="Continue on another account" onClose={close}>
      <div className="open-folder-path" title={session.cwd}>
        <span className="open-folder-name">{tabLabel(session, status?.topic)}</span>
        <span className="muted small">
          {source?.name} · {session.cwd}
        </span>
      </div>

      <AccountChoices accounts={others} onChoose={choose} />
      {others.length === 0 && <p className="note">No other account is logged in.</p>}

      <p className="note">
        The tab switches to the account you pick and the conversation goes on there; {source?.name ?? 'the current account'} keeps its copy in
        its history. The first message on the new account sends the whole conversation again
        {session.contextPct !== undefined ? ` (its context window is ${session.contextPct}% full)` : ''}.
      </p>
      {status?.busy && <p className="note warn">Claude is still working in this tab. Moving now cuts its current answer short.</p>}
      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          Cancel
        </button>
      </div>
    </Dialog>
  )
}
