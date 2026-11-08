import { describe, expect, it } from 'vitest'
import { CHANGELOG, NEWEST_VERSION, parseChangelog, parseInline, releaseNotes } from '../src/changelog'

describe('parseChangelog', () => {
  const md = [
    '# Changelog',
    '',
    'All notable changes. The format follows [Keep a Changelog](https://keepachangelog.com/).',
    '',
    '## [Unreleased]',
    '',
    '- Something on the way',
    '',
    '## [1.2.0] - 2026-11-08',
    '',
    '### Added',
    '- **Bold start.** And `code`.',
    '- An item that goes on',
    '  on the next line.',
    '',
    '### Fixed',
    '- A fix',
    '',
    '## 1.1.0 - 2026-10-01',
    '### Changed',
    '- Older'
  ].join('\r\n')

  it('reads releases, their parts and items, newest first, and skips the introduction', () => {
    const releases = parseChangelog(md)
    expect(releases.map((r) => [r.version, r.date])).toEqual([
      ['Unreleased', null],
      ['1.2.0', '2026-11-08'],
      ['1.1.0', '2026-10-01']
    ])
    // items before any part heading get a part without a title
    expect(releases[0].parts).toEqual([{ title: '', items: ['Something on the way'] }])
    expect(releases[1].parts).toEqual([
      { title: 'Added', items: ['**Bold start.** And `code`.', 'An item that goes on on the next line.'] },
      { title: 'Fixed', items: ['A fix'] }
    ])
    expect(releases[2].parts[0]).toEqual({ title: 'Changed', items: ['Older'] })
  })

  it('has nothing for a file without releases', () => {
    expect(parseChangelog('# Changelog\n\nNothing yet.')).toEqual([])
  })
})

describe('parseInline', () => {
  it('splits bold and code from the text around them', () => {
    expect(parseInline('**Tab overflow menu.** A `▾` button next to `+`.')).toEqual([
      { kind: 'bold', text: 'Tab overflow menu.' },
      { kind: 'text', text: ' A ' },
      { kind: 'code', text: '▾' },
      { kind: 'text', text: ' button next to ' },
      { kind: 'code', text: '+' },
      { kind: 'text', text: '.' }
    ])
  })

  it('takes nothing inside code for markup and leaves anything else as it is written', () => {
    expect(parseInline('`a **b**` and *c*')).toEqual([
      { kind: 'code', text: 'a **b**' },
      { kind: 'text', text: ' and *c*' }
    ])
    expect(parseInline('a ** b and `unclosed')).toEqual([{ kind: 'text', text: 'a ** b and `unclosed' }])
    expect(parseInline('')).toEqual([])
  })
})

describe('the changelog of this build', () => {
  it('is read, newest release first, and knows its newest version', () => {
    expect(CHANGELOG.length).toBeGreaterThan(0)
    expect(NEWEST_VERSION).toBe(CHANGELOG.find((r) => r.date !== null)?.version)
    expect(CHANGELOG.every((r) => r.parts.every((p) => p.items.length > 0))).toBe(true)
  })
})

describe('releaseNotes', () => {
  const known = parseChangelog('## [0.6.0] - 2026-11-08\n### Added\n- Updates')

  it('takes the entry of this build when it has the version', () => {
    expect(releaseNotes('0.6.0', 'something else', known)).toBe(known[0])
  })

  it('reads the notes of a newer version, with or without their heading', () => {
    const part = { title: 'Fixed', items: ['A **fix**', 'Another one on two lines'] }
    expect(releaseNotes('0.6.1', '### Fixed\n- A **fix**\n- Another one\n  on two lines\n', known)).toEqual({ version: '0.6.1', date: null, parts: [part] })
    expect(releaseNotes('0.6.1', '## [0.6.1] - 2026-11-20\r\n\r\n### Fixed\r\n- A **fix**\r\n- Another one on two lines', known)).toEqual({
      version: '0.6.1',
      date: '2026-11-20',
      parts: [part]
    })
    // a list without a part heading
    expect(releaseNotes('0.6.1', '- One thing', known)?.parts).toEqual([{ title: '', items: ['One thing'] }])
  })

  it('has no list for notes that are none, or for no notes', () => {
    expect(releaseNotes('0.6.1', 'See the release page.', known)).toBeNull()
    expect(releaseNotes('0.6.1', '  ', known)).toBeNull()
    expect(releaseNotes('0.6.1', null, known)).toBeNull()
  })
})
