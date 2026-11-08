import { progressFraction, progressText, releaseDate } from '@shared/updates'
import { releaseNotes } from '../changelog'
import { setModal } from '../store'
import { installUpdate, later, useUpdate } from '../updates'
import { ReleaseView } from './ChangelogDialog'
import { Dialog } from './Dialogs'

const close = (): void => setModal(null)

/** A new version, above the run bars: what's new, install it, or not now. While it downloads, how far it is. */
export function UpdateBar() {
  const info = useUpdate((s) => s.info)
  const hidden = useUpdate((s) => s.hidden)
  const installing = useUpdate((s) => s.installing)
  const progress = useUpdate((s) => s.progress)
  if (!info || (hidden && !installing)) return null
  const fraction = progress ? progressFraction(progress) : null

  return (
    <div className="update-bar" role="status">
      <span className="update-text">
        Multi Claude {info.version} is available
        <span className="muted"> · this is {info.currentVersion}</span>
      </span>
      <span className="spacer" />
      {installing ? (
        <>
          <span className="update-status">{progress ? progressText(progress) : 'Preparing the download…'}</span>
          <span className={`update-meter${fraction === null ? ' unknown' : ''}`}>
            <span style={fraction === null ? undefined : { width: `${fraction * 100}%` }} />
          </span>
        </>
      ) : (
        <>
          <button className="btn" onClick={() => setModal({ type: 'updateNotes' })}>
            What's new
          </button>
          <button className="btn primary" onClick={() => void installUpdate()}>
            Install and restart
          </button>
          <button className="btn" title="Hide this until the app starts again" onClick={later}>
            Later
          </button>
        </>
      )}
    </div>
  )
}

/** The changes of the version the bar offers: from this build's changelog when it lists them, else the release notes. */
export function UpdateNotesDialog() {
  const info = useUpdate((s) => s.info)
  const installing = useUpdate((s) => s.installing)
  const release = info && releaseNotes(info.version, info.notes)

  return (
    <Dialog title={info ? `What's new in ${info.version}` : "What's new"} onClose={close}>
      <div className="changelog">
        {release ? (
          <ReleaseView release={{ ...release, date: release.date ?? releaseDate(info?.pubDate ?? null) }} />
        ) : info?.notes ? (
          <p className="update-notes">{info.notes}</p>
        ) : (
          <p className="muted">This release lists no changes.</p>
        )}
      </div>
      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          Close
        </button>
        {info && (
          <button
            className="btn primary"
            autoFocus
            disabled={installing}
            onClick={() => {
              close()
              void installUpdate()
            }}
          >
            Install and restart
          </button>
        )}
      </div>
    </Dialog>
  )
}
