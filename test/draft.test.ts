import { describe, expect, it } from 'vitest'
import { NO_DRAFT, draftActions, isDirty, type DocOnDisk, type DraftState, type Question } from '../src/draft'
import { cleanUnsaved, unsavedNote } from '../src/shared/unsaved'

interface Ref {
  file: string
}
interface Detail extends Ref, DocOnDisk {}

/** An editor over a disk that lives in memory; the hash of a text is the text itself. */
function editor(files: Record<string, string>, answers: boolean[] = []) {
  let state: DraftState<Detail> = { ...NO_DRAFT }
  const asked: Question[] = []
  const failed: string[] = []
  const warned: string[] = []
  const saved: string[] = []
  const read = (file: string): Detail => {
    if (!(file in files)) throw new Error('gone')
    return { file, text: files[file], hash: `#${files[file]}` }
  }
  const actions = draftActions<Ref, Detail>({
    get: () => state,
    set: (patch) => {
      state = { ...state, ...patch }
    },
    key: (ref) => ref.file,
    label: (detail) => `"${detail.file}"`,
    read: async (ref) => read(ref.file),
    write: async (ref, text, baseHash) => {
      if (baseHash !== null && baseHash !== read(ref.file).hash) return { conflict: read(ref.file) }
      files[ref.file] = text
      return { saved: read(ref.file) }
    },
    problem: ({ text }) => (text.includes('BROKEN') ? 'It is broken.' : null),
    saved: (detail) => void saved.push(detail.file),
    ask: async (question) => {
      asked.push(question)
      return answers.shift() ?? false
    },
    fail: (err) => void failed.push((err as Error).message),
    warn: (message) => void warned.push(message)
  })
  return { actions, files, asked, failed, warned, saved, state: () => state }
}

describe('a draft of the Library', () => {
  it('opens a file, takes what is typed and saves it', async () => {
    const e = editor({ 'a.md': 'one\n' })
    expect(await e.actions.open({ file: 'a.md' })).toBe(true)
    expect(isDirty(e.state())).toBe(false)
    e.actions.changeText('two\n')
    expect(isDirty(e.state())).toBe(true)
    await e.actions.save()
    expect(e.files['a.md']).toBe('two\n')
    expect(isDirty(e.state())).toBe(false)
    expect(e.saved).toEqual(['a.md'])
    // nothing to save: nothing is written
    await e.actions.save()
    expect(e.saved).toEqual(['a.md'])
  })

  it('asks before unsaved text is dropped for another file', async () => {
    const e = editor({ 'a.md': 'a\n', 'b.md': 'b\n' }, [false, true])
    await e.actions.open({ file: 'a.md' })
    e.actions.changeText('mine\n')
    expect(await e.actions.open({ file: 'b.md' })).toBe(false)
    expect(e.state().draft?.detail.file).toBe('a.md')
    expect(e.asked[0]).toMatchObject({ title: 'Unsaved changes', message: '"a.md" has changes that are not saved. Opening something else discards them.' })
    expect(await e.actions.open({ file: 'b.md' })).toBe(true)
    expect(e.state().draft).toMatchObject({ text: 'b\n', detail: { file: 'b.md' } })
    // the file that is open already is not read again, and nothing is asked
    e.actions.changeText('typed\n')
    expect(await e.actions.open({ file: 'b.md' })).toBe(true)
    expect(e.state().draft?.text).toBe('typed\n')
    expect(e.asked).toHaveLength(2)
  })

  it('shows a change on disk at once when nothing is unsaved, and waits when something is', async () => {
    const e = editor({ 'a.md': 'one\n' })
    await e.actions.open({ file: 'a.md' })
    e.files['a.md'] = 'theirs\n'
    await e.actions.checkDisk()
    expect(e.state()).toMatchObject({ draft: { text: 'theirs\n' }, onDisk: null })

    e.actions.changeText('mine\n')
    e.files['a.md'] = 'theirs again\n'
    await e.actions.checkDisk()
    expect(e.state().draft?.text).toBe('mine\n')
    expect(e.state().onDisk?.text).toBe('theirs again\n')
    e.actions.loadDisk()
    expect(e.state()).toMatchObject({ draft: { text: 'theirs again\n' }, onDisk: null })
  })

  it('does not save over a file that changed, unless told to', async () => {
    const e = editor({ 'a.md': 'one\n' })
    await e.actions.open({ file: 'a.md' })
    e.actions.changeText('mine\n')
    e.files['a.md'] = 'theirs\n'
    await e.actions.save()
    expect(e.files['a.md']).toBe('theirs\n')
    expect(e.state()).toMatchObject({ saving: false, onDisk: { text: 'theirs\n' }, draft: { text: 'mine\n' } })
    await e.actions.save(true)
    expect(e.files['a.md']).toBe('mine\n')
    expect(e.state().onDisk).toBeNull()
  })

  it('asks before it saves a text with a problem', async () => {
    const e = editor({ 'a.md': 'one\n' }, [false, true])
    await e.actions.open({ file: 'a.md' })
    e.actions.changeText('BROKEN\n')
    await e.actions.save()
    expect(e.files['a.md']).toBe('one\n')
    expect(e.asked[0].message).toBe('It is broken.\n\nSave it like this anyway?')
    await e.actions.save()
    expect(e.files['a.md']).toBe('BROKEN\n')
  })

  it('reports a failed call and keeps the draft', async () => {
    const e = editor({ 'a.md': 'one\n' })
    expect(await e.actions.open({ file: 'nope.md' })).toBe(false)
    await e.actions.open({ file: 'a.md' })
    e.actions.changeText('mine\n')
    delete e.files['a.md']
    await e.actions.save()
    expect(e.failed).toEqual(['gone', 'gone'])
    expect(e.state()).toMatchObject({ saving: false, draft: { text: 'mine\n' } })
    // looking at the disk says nothing about a file that is gone; the list does
    await e.actions.checkDisk()
    expect(e.failed).toHaveLength(2)
  })

  it('reverts to what is on disk after a yes', async () => {
    const e = editor({ 'a.md': 'one\n' }, [false, true])
    await e.actions.open({ file: 'a.md' })
    e.actions.changeText('mine\n')
    await e.actions.revert()
    expect(e.state().draft?.text).toBe('mine\n')
    await e.actions.revert()
    expect(e.state().draft?.text).toBe('one\n')
  })

  it('changes fields and the body through the text, and warns once about a value it cannot write', async () => {
    const e = editor({ 'a.md': '---\nname: a\n---\n\nBody.\n', 'b.md': '---\nname: [broken\n---\n' })
    await e.actions.open({ file: 'a.md' })
    e.actions.changeField('maxTurns', 5)
    e.actions.changeField('skills', ['one', 'two'])
    e.actions.changeBody('New body.\n')
    expect(e.state().draft?.text).toBe('---\nname: a\nmaxTurns: 5\nskills:\n  - one\n  - two\n---\n\nNew body.\n')
    e.actions.changeField('skills', [])
    expect(e.state().draft?.text).toBe('---\nname: a\nmaxTurns: 5\n---\n\nNew body.\n')

    e.actions.close()
    await e.actions.open({ file: 'b.md' })
    e.actions.changeField('name', 'x')
    e.actions.changeField('name', 'xy')
    expect(e.warned).toEqual(['The frontmatter has a problem. Fix it in the Source tab first.'])
    expect(e.state().draft?.text).toBe('---\nname: [broken\n---\n')
  })
})

describe('the quit question', () => {
  it('names one draft, or lists several', () => {
    expect(unsavedNote([])).toBe('')
    expect(unsavedNote(['The agent "a"'])).toBe('The agent "a" has changes that are not saved. They are lost when the app quits.')
    expect(unsavedNote(['The skill "s"', 'The agent "a"'])).toBe('These have changes that are not saved, which are lost when the app quits:\n  • The skill "s"\n  • The agent "a"')
  })

  it('takes only a few short texts from the window', () => {
    expect(cleanUnsaved(null)).toEqual([])
    expect(cleanUnsaved('The agent')).toEqual([])
    expect(cleanUnsaved(['ok', 5, '', '  ', 'two\nlines'])).toEqual(['ok', 'two lines'])
    expect(cleanUnsaved(Array.from({ length: 20 }, (_, i) => `item ${i}`))).toHaveLength(8)
    expect(cleanUnsaved(['x'.repeat(500)])[0]).toHaveLength(120)
  })
})
