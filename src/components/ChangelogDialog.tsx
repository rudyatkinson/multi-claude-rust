import { useEffect } from 'react'
import { CHANGELOG, markChangelogSeen, parseInline } from '../changelog'
import { setModal } from '../store'
import { Dialog } from './Dialogs'

const close = (): void => setModal(null)

function Inline({ text }: { text: string }) {
  return (
    <>
      {parseInline(text).map((piece, i) =>
        piece.kind === 'bold' ? <strong key={i}>{piece.text}</strong> : piece.kind === 'code' ? <code key={i}>{piece.text}</code> : piece.text
      )}
    </>
  )
}

/** What's new: the changelog that came with this build, newest release first. */
export function ChangelogDialog() {
  // opening it once is reading it: the dot on the sidebar button goes
  useEffect(() => markChangelogSeen(), [])

  return (
    <Dialog title="What's new" onClose={close}>
      <div className="changelog">
        {CHANGELOG.length === 0 && <p className="muted">No changes are listed.</p>}
        {CHANGELOG.map((release) => (
          <section key={release.version} className="changelog-release">
            <div className="changelog-head">
              <span className="changelog-version">{release.version}</span>
              {release.date && <span className="muted">{release.date}</span>}
            </div>
            {release.parts.map((part, i) => (
              <div key={`${part.title}-${i}`} className="changelog-part">
                {part.title && <div className="changelog-part-title">{part.title}</div>}
                <ul>
                  {part.items.map((item, j) => (
                    <li key={j}>
                      <Inline text={item} />
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </section>
        ))}
      </div>
      <div className="dialog-actions">
        <button className="btn primary" autoFocus onClick={close}>
          Close
        </button>
      </div>
    </Dialog>
  )
}
