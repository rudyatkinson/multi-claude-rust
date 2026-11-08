/** A version that can be installed, as `updater:check` and `updater:available` report it. */
export interface UpdateInfo {
  version: string
  /** The release notes of `latest.json`: the release's part of CHANGELOG.md, in Markdown. */
  notes: string | null
  /** When it was published, RFC 3339. */
  pubDate: string | null
  /** The version that runs now. */
  currentVersion: string
}

/** How far the download of an update is, in bytes; `total` is null when the server did not say. */
export interface UpdateProgress {
  downloaded: number
  total: number | null
}

/** The day of an RFC 3339 publication time (`2026-11-20T10:00:00Z` is `2026-11-20`), as the changelog writes dates. */
export function releaseDate(pubDate: string | null): string | null {
  return /^\d{4}-\d{2}-\d{2}/.exec(pubDate ?? '')?.[0] ?? null
}

/** The part of the download that is done, 0 to 1; null when the size is not known. */
export function progressFraction(progress: UpdateProgress): number | null {
  if (progress.total === null || progress.total <= 0) return null
  return Math.min(1, Math.max(0, progress.downloaded / progress.total))
}

const megabytes = (bytes: number): string => `${(bytes / 1_000_000).toFixed(1)} MB`

/** What the update bar says while it downloads. */
export function progressText(progress: UpdateProgress): string {
  const fraction = progressFraction(progress)
  if (fraction === null) return `Downloading… ${megabytes(progress.downloaded)}`
  if (fraction >= 1) return 'Installing…'
  return `Downloading… ${Math.floor(fraction * 100)}% of ${megabytes(progress.total ?? 0)}`
}

/** What an install would end: running sessions, running workflows, unsaved Library drafts. */
export interface InstallLosses {
  runningSessions: number
  activeRuns: number
  unsavedDrafts: number
  /** Settings → Sessions → reopen tabs on startup. */
  restoreTabs: boolean
}

/** The question to ask before installing, or null when the restart ends nothing that is going on. */
export function installQuestion(losses: InstallLosses): { title: string; message: string; confirmLabel: string } | null {
  const lines: string[] = []
  if (losses.runningSessions > 0) {
    lines.push(
      losses.restoreTabs
        ? 'Running sessions end; their tabs come back after the restart.'
        : 'Running sessions end, and their tabs do not come back: reopening tabs on startup is off in Settings.'
    )
  }
  if (losses.activeRuns > 0) lines.push(losses.activeRuns === 1 ? 'The workflow that is running stops.' : 'The workflows that are running stop.')
  if (losses.unsavedDrafts > 0) lines.push('Unsaved changes in the Library are lost.')
  if (lines.length === 0) return null
  return { title: 'Install the update', message: lines.join(' '), confirmLabel: 'Install and restart' }
}
