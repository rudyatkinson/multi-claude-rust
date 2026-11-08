import { describe, expect, it } from 'vitest'
import { fieldItems, fieldText, frontmatterBlock, parseDoc, setField, splitItems, type FieldInput } from '../src/shared/frontmatter'

const lines = (...l: string[]): string => l.join('\n') + '\n'

// as a subagent is written: a number, a list in two spellings, a field the form does not know
const AGENT = lines(
  '---',
  'name: code-reviewer',
  'description: Reviews a diff for bugs.',
  'tools: Read, Grep, Glob',
  'maxTurns: 20',
  '# what it brings along',
  'skills:',
  '  - arch-review',
  '  - unity-cli',
  'mcpServers:',
  '  - github',
  '---',
  '',
  'You review code.'
)

const set = (text: string, key: string, value: FieldInput): string => {
  const result = setField(text, key, value)
  if ('failed' in result) throw new Error(result.failed)
  return result.text
}

describe('numbers', () => {
  it('writes a number without quotes and reads it back', () => {
    const next = set(AGENT, 'maxTurns', 35)
    expect(next).toContain('\nmaxTurns: 35\n')
    expect(fieldText(parseDoc(next), 'maxTurns')).toBe('35')
    // nothing else moved
    expect(next.replace('maxTurns: 35', 'maxTurns: 20')).toBe(AGENT)
  })

  it('adds a number to a file that had none', () => {
    const next = set(lines('---', 'name: a', '---', '', 'Text.'), 'maxTurns', 5)
    expect(next).toBe(lines('---', 'name: a', 'maxTurns: 5', '---', '', 'Text.'))
  })

  it('keeps a number typed as text a text', () => {
    // the form's number field hands over a number; a text of digits is quoted, as before
    expect(set(AGENT, 'maxTurns', '35')).toContain('maxTurns: "35"')
  })

  it('refuses what is not a number', () => {
    expect(setField(AGENT, 'maxTurns', Number.NaN)).toEqual({ failed: '"maxTurns" needs a number.' })
    expect(setField(AGENT, 'maxTurns', Number.POSITIVE_INFINITY)).toHaveProperty('failed')
  })
})

describe('lists', () => {
  it('writes several names as a YAML list and changes only those lines', () => {
    const next = set(AGENT, 'skills', ['arch-review', 'implement-feature', 'unity-cli'])
    expect(next).toContain(lines('skills:', '  - arch-review', '  - implement-feature', '  - unity-cli', 'mcpServers:'))
    expect(fieldItems(parseDoc(next), 'skills')).toEqual(['arch-review', 'implement-feature', 'unity-cli'])
    // the comment above the field and the field after it stay
    expect(next).toContain('# what it brings along\nskills:')
    expect(next).toContain(lines('mcpServers:', '  - github', '---'))
  })

  it('quotes an entry YAML would read as something else', () => {
    const next = set(AGENT, 'skills', ['true', '12', 'a: b'])
    expect(fieldItems(parseDoc(next), 'skills')).toEqual(['true', '12', 'a: b'])
  })

  it('takes the field out for an empty list', () => {
    const next = set(AGENT, 'skills', [])
    expect(parseDoc(next).keys).toEqual(['name', 'description', 'tools', 'maxTurns', 'mcpServers'])
    expect(set(next, 'skills', [])).toBe(next)
  })

  it('adds a list to a file that had none', () => {
    const next = set(lines('---', 'name: a', '---'), 'skills', ['one'])
    expect(next).toBe(lines('---', 'name: a', 'skills:', '  - one', '---'))
  })

  it('reads the entries of a list however it is written', () => {
    const parsed = parseDoc(AGENT)
    expect(fieldItems(parsed, 'skills')).toEqual(['arch-review', 'unity-cli'])
    expect(fieldItems(parsed, 'tools')).toEqual(['Read', 'Grep', 'Glob'])
    expect(fieldItems(parseDoc(lines('---', 'tools: [Read, Bash]', '---')), 'tools')).toEqual(['Read', 'Bash'])
    expect(fieldItems(parsed, 'nothing')).toEqual([])
    expect(splitItems(' a, ,b ,')).toEqual(['a', 'b'])
  })
})

describe('frontmatterBlock', () => {
  it('writes the fields of a new file so that they read back', () => {
    const text = `${frontmatterBlock({ name: 'x', description: 'Says: use it. #1', maxTurns: 3 })}\nBody.\n`
    const parsed = parseDoc(text)
    expect(parsed.error).toBeNull()
    expect(fieldText(parsed, 'description')).toBe('Says: use it. #1')
    expect(fieldText(parsed, 'maxTurns')).toBe('3')
    expect(parsed.body).toBe('Body.\n')
  })
})
