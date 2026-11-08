import { describe, expect, it } from 'vitest'
import {
  addIndexLine,
  indexLine,
  inspectMemory,
  memoryMeta,
  memoryNameProblem,
  memorySnippet,
  newMemoryText,
  parseIndex,
  planAdopt,
  removeIndexLine
} from '../src/shared/memory'

const lines = (...l: string[]): string => l.join('\n') + '\n'

// as Claude Code writes an index
const INDEX = lines(
  '# Memory Index',
  '',
  '- [No explanatory comments](no-explanatory-comments.md) — Bilal wants zero Claude-added code comments',
  '- [claude-config repo](claude-config-repo.md) — ~/.claude is a private git repo (bilal-arvis/claude-config)',
  '- [Offline compile check](./offline-compile-check.md)',
  '',
  'See also [the docs](https://example.com/page.md) and [a note elsewhere](../other/note.md).'
)

describe('the index', () => {
  it('reads the lines that point to a memory', () => {
    expect(parseIndex(INDEX)).toEqual([
      { line: 2, title: 'No explanatory comments', file: 'no-explanatory-comments.md', hook: 'Bilal wants zero Claude-added code comments' },
      { line: 3, title: 'claude-config repo', file: 'claude-config-repo.md', hook: '~/.claude is a private git repo (bilal-arvis/claude-config)' },
      { line: 4, title: 'Offline compile check', file: 'offline-compile-check.md', hook: '' }
    ])
    expect(parseIndex('')).toEqual([])
    // other ways of writing the same line
    expect(parseIndex('* [A](a.md): when it rains\n+ [B](b.md) - hook\n')).toMatchObject([
      { file: 'a.md', hook: 'when it rains' },
      { file: 'b.md', hook: 'hook' }
    ])
  })

  it('adds a line after the last one, and starts a list where none was', () => {
    const added = addIndexLine(INDEX, { title: 'Unity [CLI] tips', file: 'unity-cli-tips.md', hook: 'menu builders,\nscreenshots' })
    expect(added.split('\n').slice(4, 7)).toEqual(['- [Offline compile check](./offline-compile-check.md)', '- [Unity CLI tips](unity-cli-tips.md) — menu builders, screenshots', ''])
    expect(parseIndex(added)).toHaveLength(4)

    expect(addIndexLine('', { title: 'First', file: 'first.md', hook: '' })).toBe('- [First](first.md)\n')
    expect(addIndexLine('# Memory Index\n', { title: 'First', file: 'first.md', hook: 'why' })).toBe('# Memory Index\n\n- [First](first.md) — why\n')
    // a file without a last line break gets one
    expect(addIndexLine('- [A](a.md)', { title: '', file: 'b.md', hook: '' })).toBe('- [A](a.md)\n- [b](b.md)\n')
  })

  it('takes out the lines of one file and nothing else', () => {
    const without = removeIndexLine(INDEX, 'Claude-Config-Repo.md')
    expect(parseIndex(without).map((entry) => entry.file)).toEqual(['no-explanatory-comments.md', 'offline-compile-check.md'])
    expect(without).toBe(INDEX.replace('- [claude-config repo](claude-config-repo.md) — ~/.claude is a private git repo (bilal-arvis/claude-config)\n', ''))
    expect(removeIndexLine(INDEX, 'nothing.md')).toBe(INDEX)
  })

  it('writes a line as it reads it', () => {
    const entry = { title: 'A title', file: 'a-title.md', hook: 'when to read it' }
    expect(parseIndex(indexLine(entry))[0]).toMatchObject(entry)
  })
})

describe('inspectMemory', () => {
  const files = ['no-explanatory-comments.md', 'claude-config-repo.md', 'offline-compile-check.md']

  it('has nothing to say when the index and the folder agree', () => {
    expect(inspectMemory({ text: INDEX, bytes: INDEX.length }, files)).toEqual([])
    expect(inspectMemory(null, [])).toEqual([])
  })

  it('names files the index does not list, and lines whose file is gone', () => {
    const issues = inspectMemory({ text: INDEX, bytes: INDEX.length }, ['no-explanatory-comments.md', 'Claude-Config-Repo.md', 'stray.md'])
    expect(issues).toEqual([
      { level: 'warning', message: 'MEMORY.md points to a file that is not there: offline-compile-check.md.' },
      { level: 'info', message: 'Not in MEMORY.md, so a session does not know of it: stray.md.' }
    ])
    expect(inspectMemory(null, ['a.md'])[0].message).toContain('There is no MEMORY.md.')
    // a long list is cut short
    const many = Array.from({ length: 8 }, (_, i) => `- [M${i}](m${i}.md)`).join('\n')
    expect(inspectMemory({ text: many, bytes: many.length }, [])[0].message).toBe('MEMORY.md points to files that are not there: m0.md, m1.md, m2.md, m3.md, m4.md and 3 more.')
  })

  it('warns when a session would not read the index to its end', () => {
    const long = Array.from({ length: 201 }, (_, i) => `- [M${i}](m${i}.md)`).join('\n') + '\n'
    const all = Array.from({ length: 201 }, (_, i) => `m${i}.md`)
    expect(inspectMemory({ text: long, bytes: long.length }, all)).toEqual([{ level: 'warning', message: 'MEMORY.md has 201 lines. A session reads the first 200; what comes after is not seen.' }])
    const heavy = `- [A](a.md) — ${'x'.repeat(26_000)}\n`
    expect(inspectMemory({ text: heavy, bytes: heavy.length }, ['a.md'])[0].message).toBe('MEMORY.md has 26 KB. A session reads the first 25 KB; what comes after is not seen.')
  })
})

describe('one memory', () => {
  // as Claude Code writes a memory file
  const REAL = lines(
    '---',
    'name: claude-config-repo',
    'description: ~/.claude is a private git repo syncing user-level skills/agents/settings',
    'metadata: ',
    '  node_type: memory',
    '  type: reference',
    '  originSessionId: 63c5497c-b5b9-4965-81e7-397beb58ae62',
    '  modified: 2026-08-31T21:51:12.751Z',
    '---',
    '',
    'The folder is a git repo.'
  )

  it('reads what the frontmatter says', () => {
    expect(memoryMeta(REAL)).toEqual({ name: 'claude-config-repo', description: '~/.claude is a private git repo syncing user-level skills/agents/settings', type: 'reference' })
    expect(memoryMeta('Just text.\n')).toEqual({ name: '', description: '', type: '' })
    expect(memoryMeta('---\nname: [broken\n---\n')).toEqual({ name: '', description: '', type: '' })
    // an older file with the type at the top
    expect(memoryMeta('---\nname: a\ntype: feedback\n---\n').type).toBe('feedback')
  })

  it('writes a new memory the same way', () => {
    const text = newMemoryText({ name: 'release-steps', description: 'How a release\nis cut: tag, then build', type: 'project', modified: '2026-10-08T10:00:00.000Z' })
    expect(memoryMeta(text)).toEqual({ name: 'release-steps', description: 'How a release is cut: tag, then build', type: 'project' })
    expect(text).toContain('  node_type: memory\n  type: project\n  modified: 2026-10-08T10:00:00.000Z\n---\n\n')
  })

  it('asks for a name that can be a file', () => {
    expect(memoryNameProblem('release-steps')).toBeNull()
    expect(memoryNameProblem('')).toBe('Give the memory a name.')
    expect(memoryNameProblem('memory')).toBe('MEMORY.md is the index itself.')
    expect(memoryNameProblem('Release Steps')).toBe('Use lower-case letters, digits and hyphens only.')
  })

  it('shows the text around what a search found', () => {
    const text = 'The editor must never be started without asking. Bilal may be in the middle of something else on the machine.'
    expect(memorySnippet(text, ['bilal'], 20)).toBe('…ted without asking. Bilal may be in the middl…')
    expect(memorySnippet(text, ['editor', 'machine'], 10)).toBe('The editor must neve…')
    expect(memorySnippet(text, ['editor', 'missing'])).toBeNull()
    expect(memorySnippet('Işık açık kalsın', ['isik'])).toBe('Işık açık kalsın')
    expect(memorySnippet(text, [])).toBeNull()
  })
})

describe('planAdopt', () => {
  const file = (name: string, hash: string): { name: string; hash: string } => ({ name, hash })

  it('copies what the target lacks and leaves what it has', () => {
    const plan = planAdopt([file('MEMORY.md', 'i1'), file('a.md', 'a'), file('b.md', 'b')], [file('MEMORY.md', 'i2'), file('B.md', 'b')], 'elif')
    expect(plan).toEqual({ copy: [{ from: 'a.md', to: 'a.md' }], same: ['b.md'] })
  })

  it('brings a file of the same name with other content under a name that says where it came from', () => {
    const plan = planAdopt([file('a.md', 'theirs')], [file('a.md', 'mine')], 'elif')
    expect(plan).toEqual({ copy: [{ from: 'a.md', to: 'a.from-elif.md' }], same: [] })
    // brought in before: nothing to do
    expect(planAdopt([file('a.md', 'theirs')], [file('a.md', 'mine'), file('a.from-elif.md', 'theirs')], 'elif')).toEqual({ copy: [], same: ['a.md'] })
    // brought in before, and changed since on one side
    expect(planAdopt([file('a.md', 'newer')], [file('a.md', 'mine'), file('a.from-elif.md', 'theirs')], 'elif').copy).toEqual([{ from: 'a.md', to: 'a.from-elif-2.md' }])
  })

  it('never touches the index, and never gives two files one name', () => {
    const plan = planAdopt([file('MEMORY.md', 'x'), file('a.md', '1'), file('A.md', '2')], [], 'solo')
    expect(plan.copy).toEqual([
      { from: 'a.md', to: 'a.md' },
      { from: 'A.md', to: 'A.from-solo.md' }
    ])
  })
})
