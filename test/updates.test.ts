import { describe, expect, it } from 'vitest'
import { installQuestion, progressFraction, progressText, releaseDate } from '../src/shared/updates'

describe('the download of an update', () => {
  it('says how far it is when the size is known', () => {
    expect(progressFraction({ downloaded: 2_500_000, total: 10_000_000 })).toBe(0.25)
    expect(progressText({ downloaded: 2_500_000, total: 10_000_000 })).toBe('Downloading… 25% of 10.0 MB')
    expect(progressText({ downloaded: 9_999_999, total: 10_000_000 })).toBe('Downloading… 99% of 10.0 MB')
    expect(progressText({ downloaded: 10_000_000, total: 10_000_000 })).toBe('Installing…')
    // more than announced is still done, not more than done
    expect(progressFraction({ downloaded: 12, total: 10 })).toBe(1)
  })

  it('counts the bytes when the size is not known', () => {
    expect(progressFraction({ downloaded: 1_234_567, total: null })).toBeNull()
    expect(progressFraction({ downloaded: 5, total: 0 })).toBeNull()
    expect(progressText({ downloaded: 1_234_567, total: null })).toBe('Downloading… 1.2 MB')
  })
})

describe('installQuestion', () => {
  const nothing = { runningSessions: 0, activeRuns: 0, unsavedDrafts: 0, restoreTabs: true }

  it('asks nothing when the restart ends nothing', () => {
    expect(installQuestion(nothing)).toBeNull()
  })

  it('says that running sessions end and whether their tabs come back', () => {
    expect(installQuestion({ ...nothing, runningSessions: 2 })).toEqual({
      title: 'Install the update',
      message: 'Running sessions end; their tabs come back after the restart.',
      confirmLabel: 'Install and restart'
    })
    expect(installQuestion({ ...nothing, runningSessions: 1, restoreTabs: false })?.message).toBe(
      'Running sessions end, and their tabs do not come back: reopening tabs on startup is off in Settings.'
    )
  })

  it('names workflows that stop and drafts that are lost', () => {
    expect(installQuestion({ ...nothing, runningSessions: 1, activeRuns: 1, unsavedDrafts: 2 })?.message).toBe(
      'Running sessions end; their tabs come back after the restart. The workflow that is running stops. Unsaved changes in the Library are lost.'
    )
    expect(installQuestion({ ...nothing, activeRuns: 2 })?.message).toBe('The workflows that are running stop.')
  })
})

describe('releaseDate', () => {
  it('takes the day of a publication time', () => {
    expect(releaseDate('2026-11-20T10:00:00Z')).toBe('2026-11-20')
    expect(releaseDate('2026-11-20T10:00:00.123+03:00')).toBe('2026-11-20')
    expect(releaseDate('yesterday')).toBeNull()
    expect(releaseDate(null)).toBeNull()
  })
})
