import { describe, expect, it } from 'vitest'
import { asBool, freeSkillName, isReservedSkillDir, nameClashes, newNameProblem, slugifySkillName, type SkillRoot } from '../src/shared/skills'
import {
  fieldBool,
  fieldText,
  firstBodyLine,
  inspectSkill,
  invocationOf,
  nameOf,
  newSkillText,
  parseSkill,
  renameInText,
  setBody,
  setField,
  splitSkill
} from '../src/shared/skillText'

const lines = (...l: string[]): string => l.join('\n') + '\n'

// as the user's own skills are written: a hint in brackets without quotes, an empty tools line
const REAL = lines(
  '---',
  'name: arch-review',
  'argument-hint: [path | PR#]',
  'allowed-tools:',
  'description: Reviews the branch diff (or a given path/PR) against the architecture contract — layers, DI, naming.',
  '---',
  '',
  '# Arch review',
  '',
  'Do the review.'
)

const RICH = lines(
  '---',
  '# who wrote this',
  'name: unity-cli  # the command',
  'allowed-tools:',
  '- Read',
  '- Bash(git *, gh pr *)',
  '',
  '# what it is for',
  'description: >-',
  '  Use when driving Unity',
  '  from the terminal.',
  'hooks:',
  '  Stop:',
  '    - hooks:',
  '        - type: command',
  '          command: echo done',
  'user-invocable: no',
  '---',
  'Body right under the fence.'
)

const set = (text: string, key: string, value: string | boolean | null): string => {
  const result = setField(text, key, value)
  if ('failed' in result) throw new Error(result.failed)
  return result.text
}

describe('splitSkill', () => {
  it('finds the frontmatter between the fences', () => {
    const parts = splitSkill(REAL)
    expect(parts.frontmatter).toBe(lines('name: arch-review', 'argument-hint: [path | PR#]', 'allowed-tools:', 'description: Reviews the branch diff (or a given path/PR) against the architecture contract — layers, DI, naming.'))
    expect(REAL.slice(parts.bodyStart)).toBe('\n# Arch review\n\nDo the review.\n')
    expect(parts.unclosed).toBe(false)
  })

  it('reads an empty frontmatter and fences with spaces after them', () => {
    expect(splitSkill('---\n---\nbody')).toMatchObject({ frontmatter: '', bodyStart: 8 })
    expect(splitSkill('--- \nname: x\n---\t\nbody').frontmatter).toBe('name: x\n')
    expect(splitSkill('---\nname: x\n---').frontmatter).toBe('name: x\n')
  })

  it('takes a file that does not open with a fence as text only', () => {
    expect(splitSkill('# Title\n---\nname: x\n---\n')).toMatchObject({ frontmatter: null, bodyStart: 0, unclosed: false })
    expect(splitSkill('\n---\nname: x\n---\n').frontmatter).toBeNull()
    expect(splitSkill('').frontmatter).toBeNull()
  })

  it('notices a frontmatter that is never closed', () => {
    expect(splitSkill('---\nname: x\n\n# Title\n')).toMatchObject({ frontmatter: null, unclosed: true })
    expect(splitSkill('---').unclosed).toBe(true)
    // a fence that is indented belongs to a value
    expect(splitSkill('---\ndescription: |\n  ---\n  more\n').unclosed).toBe(true)
  })
})

describe('parseSkill', () => {
  it('reads the fields as the form shows them', () => {
    const parsed = parseSkill(REAL)
    expect(parsed.error).toBeNull()
    expect(parsed.keys).toEqual(['name', 'argument-hint', 'allowed-tools', 'description'])
    expect(nameOf(parsed)).toBe('arch-review')
    // brackets without quotes are a list to YAML; the form shows what the author wrote
    expect(fieldText(parsed, 'argument-hint')).toBe('[path | PR#]')
    expect(fieldText(parsed, 'allowed-tools', true)).toBe('')
    expect(fieldText(parsed, 'description')).toContain('architecture contract — layers')
    expect(fieldText(parsed, 'model')).toBe('')
    expect(parsed.body).toBe('# Arch review\n\nDo the review.\n')
    expect(parsed.lines).toBe(10)
  })

  it('reads lists, folded text, comments and switches', () => {
    const parsed = parseSkill(RICH)
    expect(parsed.error).toBeNull()
    expect(parsed.keys).toEqual(['name', 'allowed-tools', 'description', 'hooks', 'user-invocable'])
    expect(nameOf(parsed)).toBe('unity-cli')
    expect(fieldText(parsed, 'allowed-tools', true)).toBe('Read, Bash(git *, gh pr *)')
    expect(fieldText(parsed, 'description')).toBe('Use when driving Unity from the terminal.')
    expect(fieldBool(parsed, 'user-invocable', true)).toBe(false)
    expect(invocationOf(parsed)).toEqual({ modelInvocable: true, userInvocable: false })
    expect(parsed.body).toBe('Body right under the fence.\n')
  })

  it('says why the fields cannot be read, with the line in the file', () => {
    const colon = parseSkill('---\nname: x\ndescription: Use when: it rains\n---\n')
    expect(colon.error?.line).toBe(3)
    expect(colon.error?.message).toBe('A value contains a colon followed by a space, and has to stand in quotes for that.')
    expect(parseSkill('---\nname: a\nname: b\n---\n').error).toEqual({ message: 'A field is written twice.', line: 3 })
    expect(parseSkill('---\nname: x\ndescription: "open\n---\n').error?.message).not.toMatch(/at line/)
    expect(parseSkill('---\n- a\n- b\n---\n').error?.message).toContain('not a list of')
    expect(parseSkill('---\n{name: x}\n---\n').error?.message).toContain('braces')
    expect(parseSkill('---\nname: x\n').error?.message).toContain('never closed')
  })

  it('reads a file without frontmatter as text', () => {
    const parsed = parseSkill('# Just text\n\nmore\n')
    expect(parsed).toMatchObject({ error: null, keys: [], body: '# Just text\n\nmore\n', lines: 3 })
    expect(nameOf(parsed)).toBeNull()
    expect(firstBodyLine(parsed.body)).toBe('Just text')
    expect(parseSkill('')).toMatchObject({ keys: [], body: '', lines: 0 })
  })

  it('reads yes and no, on and off, 1 and 0 as switches', () => {
    expect(['true', 'Yes', 'ON', '1'].map(asBool)).toEqual([true, true, true, true])
    expect(['false', 'no', 'Off', '0'].map(asBool)).toEqual([false, false, false, false])
    expect(asBool('maybe')).toBeNull()
    const parsed = parseSkill('---\ndisable-model-invocation: yes\nuser-invocable: maybe\n---\n')
    expect(invocationOf(parsed)).toEqual({ modelInvocable: false, userInvocable: true })
  })
})

describe('setField', () => {
  it('changes the lines of one field and nothing else', () => {
    const next = set(REAL, 'description', 'Short now.')
    expect(next).toBe(REAL.replace(/^description: .*$/m, 'description: Short now.'))
    // the bracket hint and the empty tools line stand as they were
    expect(next).toContain('argument-hint: [path | PR#]\nallowed-tools:\n')
  })

  it('keeps comments, order and fields the form does not know', () => {
    const next = set(RICH, 'description', 'One line.')
    expect(next).toBe(RICH.replace('description: >-\n  Use when driving Unity\n  from the terminal.\n', 'description: One line.\n'))
    expect(next).toContain('\n\n# what it is for\ndescription: One line.\nhooks:\n  Stop:')
    const tools = set(RICH, 'allowed-tools', 'Read, Grep')
    expect(tools).toContain('name: unity-cli  # the command\nallowed-tools: Read, Grep\n\n# what it is for\n')
    expect(tools).toContain('          command: echo done\nuser-invocable: no\n---\nBody right under the fence.\n')
  })

  it('writes a value as typed when YAML reads it the same way, else in quotes', () => {
    expect(set(REAL, 'argument-hint', '[file | diff]')).toContain('\nargument-hint: [file | diff]\n')
    expect(set(REAL, 'description', 'Use when: it rains')).toContain('\ndescription: "Use when: it rains"\n')
    expect(set(REAL, 'description', 'ends with a hash # not a comment')).toContain('description: "ends with a hash # not a comment"')
    expect(set(REAL, 'description', 'true')).toContain('\ndescription: "true"\n')
    expect(set(REAL, 'model', '123')).toContain('\nmodel: "123"\n')
    expect(set(REAL, 'description', "it's fine")).toContain("\ndescription: it's fine\n")
    expect(set(REAL, 'description', 'Türkçe açıklama — çalışır')).toContain('\ndescription: Türkçe açıklama — çalışır\n')
  })

  it('reads back exactly what was written', () => {
    for (const value of ['plain', '[a | b]', 'a: b', '- not a list', '"quoted"', "'single'", 'x # y', '{braces}', '@at', '*star', '!bang', '% percent', 'back\\slash', '50%', 'a, b, Bash(git *, gh pr *)']) {
      expect(fieldText(parseSkill(set(REAL, 'description', value)), 'description')).toBe(value)
    }
  })

  it('writes several lines as a block and reads them back', () => {
    const next = set(REAL, 'description', 'First line.\nSecond line.')
    expect(next).toContain('\ndescription: |-\n  First line.\n  Second line.\n---\n')
    expect(fieldText(parseSkill(next), 'description')).toBe('First line.\nSecond line.')
  })

  it('adds a field at the end and takes one out', () => {
    const added = set(REAL, 'model', 'opus')
    expect(added).toContain('naming.\nmodel: opus\n---\n')
    expect(parseSkill(added).keys).toEqual(['name', 'argument-hint', 'allowed-tools', 'description', 'model'])
    expect(set(added, 'model', '')).toBe(REAL)
    expect(set(added, 'model', null)).toBe(REAL)
    // taking out what is not there changes nothing
    expect(set(REAL, 'effort', null)).toBe(REAL)
    expect(set(RICH, 'allowed-tools', null)).toBe(RICH.replace('allowed-tools:\n- Read\n- Bash(git *, gh pr *)\n', ''))
  })

  it('sets and clears the switches', () => {
    const off = set(REAL, 'disable-model-invocation', true)
    expect(off).toContain('\ndisable-model-invocation: true\n---\n')
    expect(invocationOf(parseSkill(off)).modelInvocable).toBe(false)
    expect(set(off, 'disable-model-invocation', null)).toBe(REAL)
    expect(set(RICH, 'user-invocable', true)).toContain('\nuser-invocable: true\n---\n')
  })

  it('gives a file without frontmatter one', () => {
    expect(set('# Title\n\ntext\n', 'description', 'Does things.')).toBe('---\ndescription: Does things.\n---\n\n# Title\n\ntext\n')
    expect(set('', 'name', 'x')).toBe('---\nname: x\n---\n')
    expect(set('---\n---\nbody\n', 'name', 'x')).toBe('---\nname: x\n---\nbody\n')
    expect(set('# Title\n', 'description', null)).toBe('# Title\n')
  })

  it('refuses instead of guessing when the frontmatter is broken', () => {
    const broken = '---\nname: x\ndescription: Use when: it rains\n---\n'
    expect(setField(broken, 'model', 'opus')).toEqual({ failed: 'The frontmatter has a problem. Fix it in the Source tab first.' })
    expect(setField('---\nname: x\n', 'model', 'opus')).toHaveProperty('failed')
    expect(setField(REAL, 'not a key', 'x')).toHaveProperty('failed')
  })

  it('changes only the first of a field that is written with quotes around its key', () => {
    const quoted = '---\n"name": x\n\'description\': old\n---\n'
    expect(set(quoted, 'description', 'new')).toBe('---\n"name": x\ndescription: new\n---\n')
  })
})

describe('setBody', () => {
  it('replaces the text under the frontmatter and keeps the empty line after the fence', () => {
    expect(setBody(REAL, 'New body.\n')).toBe(REAL.replace('# Arch review\n\nDo the review.\n', 'New body.\n'))
    expect(setBody(RICH, 'Other.\n')).toBe(RICH.replace('Body right under the fence.\n', 'Other.\n'))
  })

  it('is the same text when the body is the same', () => {
    for (const text of [REAL, RICH, '# no frontmatter\n', '---\nname: x\n---', '---\nname: x\n---\n', '---\nname: x\n---\n\n']) {
      expect(setBody(text, parseSkill(text).body)).toBe(text)
    }
  })

  it('starts a body under a frontmatter that had none', () => {
    expect(setBody('---\nname: x\n---\n', 'Hello\n')).toBe('---\nname: x\n---\n\nHello\n')
    expect(setBody('---\nname: x\n---', 'Hello\n')).toBe('---\nname: x\n---\n\nHello\n')
    expect(setBody('plain\n', 'other\n')).toBe('other\n')
  })
})

describe('new skills and renames', () => {
  it('writes a first text that passes every check but the missing work', () => {
    const text = newSkillText('review-pr', 'Reviews a PR: use when asked.')
    expect(text).toBe('---\nname: review-pr\ndescription: "Reviews a PR: use when asked."\n---\n\n# Review Pr\n\nWrite here what Claude should do when this skill is used.\n')
    expect(inspectSkill(parseSkill(text), 'review-pr')).toEqual([])
    expect(fieldText(parseSkill(newSkillText('x', '')), 'description')).toContain('What this skill does')
  })

  it('lets the name in the file follow the folder when it was the folder name', () => {
    expect(renameInText(REAL, 'arch-review', 'arch-check')).toBe(REAL.replace('name: arch-review', 'name: arch-check'))
    // a name of its own stays, unless a copy must not answer to the name of its original
    expect(renameInText(REAL, 'some-folder', 'other')).toBe(REAL)
    expect(renameInText(REAL, 'some-folder', 'other', true)).toContain('name: other\n')
    // no name in the file: the folder is the name
    expect(renameInText('# text\n', 'a', 'b', true)).toBe('# text\n')
  })
})

describe('inspectSkill', () => {
  const messages = (text: string, dir: string, scope: 'claude' | 'upload'): string[] =>
    inspectSkill(parseSkill(text), dir)
      .filter((i) => i.scope === scope)
      .map((i) => `${i.level}: ${i.message}`)

  it('has nothing to say to Claude Code about the real file, and names what claude.ai would refuse', () => {
    expect(messages(REAL, 'arch-review', 'claude')).toEqual([])
    expect(messages(REAL, 'arch-review', 'upload')).toEqual(['warning: claude.ai does not accept this field: argument-hint. Claude Code uses them.'])
  })

  it('reports a frontmatter that cannot be read as an error, and nothing else', () => {
    const issues = inspectSkill(parseSkill('---\nname: x\ndescription: Use when: it rains\n---\n'), 'x')
    expect(issues).toHaveLength(1)
    expect(issues[0]).toMatchObject({ level: 'error', scope: 'claude' })
    expect(issues[0].message).toContain('(line 3)')
    expect(issues[0].message).toContain('without any of its fields')
  })

  it('warns about a missing or overlong description and a very long file', () => {
    expect(messages('---\nname: x\n---\n# Title\n', 'x', 'claude')[0]).toContain('No description')
    const long = set(REAL, 'description', 'x'.repeat(1600))
    expect(messages(long, 'arch-review', 'claude')[0]).toContain('1600 characters')
    expect(messages(long, 'arch-review', 'upload').join(' ')).toContain('at most 1024')
    expect(messages(setBody(REAL, 'line\n'.repeat(600)), 'arch-review', 'claude')[0]).toContain('info: SKILL.md has 607 lines')
  })

  it('checks the name only for the upload', () => {
    expect(messages(REAL, 'other-folder', 'claude')).toEqual([])
    expect(messages(REAL, 'other-folder', 'upload')[0]).toContain('differs from the folder')
    expect(messages(set(REAL, 'name', 'Arch Review'), 'arch-review', 'upload')[0]).toContain('does not pass on claude.ai')
    expect(messages('# text\n', 'x', 'upload')[0]).toContain('No name in the file')
    expect(messages(set(REAL, 'description', 'a <b> c'), 'arch-review', 'upload').join(' ')).toContain('< or >')
  })
})

describe('names', () => {
  it('accepts kebab-case and says what is wrong with anything else', () => {
    expect(newNameProblem('review-pr-2')).toBeNull()
    expect(newNameProblem('')).toContain('Give the skill a name')
    expect(newNameProblem('Review')).toContain('lower-case')
    expect(newNameProblem('a b')).toContain('lower-case')
    expect(newNameProblem('a/b')).toContain('lower-case')
    expect(newNameProblem('..')).toContain('lower-case')
    expect(newNameProblem('-a')).toContain('hyphen')
    expect(newNameProblem('a--b')).toContain('hyphen')
    expect(newNameProblem('x'.repeat(65))).toContain('at most 64')
    expect(newNameProblem('synced')).toContain('Claude Code keeps')
    expect(newNameProblem('nul')).toContain('Windows keeps')
    expect(newNameProblem('com1')).toContain('Windows keeps')
  })

  it('knows the folders that are not skills', () => {
    expect(['synced', 'Synced', '.trash', '.mc-new-1', 'anthropic-skills'].map(isReservedSkillDir)).toEqual([true, true, true, true, true])
    expect(isReservedSkillDir('my-skill')).toBe(false)
  })

  it('makes a name from a title', () => {
    expect(slugifySkillName('  Review PR (quick) ')).toBe('review-pr-quick')
    expect(slugifySkillName('İnceleme Işık çğş')).toBe('inceleme-isik-cgs')
    expect(slugifySkillName('!!!')).toBe('')
    expect(slugifySkillName('x'.repeat(80)).length).toBe(64)
  })

  it('finds a free name', () => {
    expect(freeSkillName('review', [])).toBe('review')
    expect(freeSkillName('review', ['Review', 'review-2'])).toBe('review-3')
    expect(freeSkillName('x'.repeat(64), ['x'.repeat(64)])).toBe(`${'x'.repeat(62)}-2`)
  })
})

describe('nameClashes', () => {
  const roots: SkillRoot[] = [
    { id: 'p', kind: 'personal', dir: '/home/.claude/skills', profileIds: ['main', 'work'], exists: true },
    { id: 'j', kind: 'project', dir: '/code/app/.claude/skills', profileIds: [], folder: '/code/app', exists: true }
  ]
  const skill = (rootId: string, dirName: string, name = dirName) => ({ rootId, dirName, name })
  const clashes = (...skills: ReturnType<typeof skill>[]) =>
    Object.fromEntries([...nameClashes(skills, roots, () => 'Main, Work')].map(([key, issues]) => [key, issues.map((i) => i.message)]))

  it('says which project skill a personal one hides', () => {
    const result = clashes(skill('p', 'review'), skill('j', 'review'), skill('j', 'deploy'))
    expect(Object.keys(result)).toEqual(['j/review'])
    expect(result['j/review'][0]).toBe('The personal skill "review" (Main, Work) has the same name and is used instead of this one.')
  })

  it('goes by the folder name and by the name in the file', () => {
    expect(Object.keys(clashes(skill('p', 'folder-a', 'review'), skill('j', 'review')))).toEqual(['j/review'])
    expect(Object.keys(clashes(skill('p', 'Review'), skill('j', 'x', 'review')))).toEqual(['j/x'])
    expect(clashes(skill('p', 'a'), skill('j', 'b'))).toEqual({})
  })

  it('warns on both of two skills in one folder that answer to the same name', () => {
    const result = clashes(skill('p', 'one', 'shared'), skill('p', 'two', 'shared'))
    expect(Object.keys(result).sort()).toEqual(['p/one', 'p/two'])
    expect(result['p/one'][0]).toContain('"two" in the same folder')
  })
})
