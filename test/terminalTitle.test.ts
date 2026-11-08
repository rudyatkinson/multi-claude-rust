import { describe, expect, it } from 'vitest'
import { parseTerminalTitle } from '../src/shared/terminalTitle'

describe('parseTerminalTitle', () => {
  it('reads the topic of a waiting session', () => {
    expect(parseTerminalTitle('✳ translate-ui-refresh-feature')).toEqual({ topic: 'translate-ui-refresh-feature', busy: false })
  })

  it('treats either half-circle mark as working', () => {
    expect(parseTerminalTitle('◐ Fix the login flow')).toEqual({ topic: 'Fix the login flow', busy: true })
    expect(parseTerminalTitle('◑ Fix the login flow')).toEqual({ topic: 'Fix the login flow', busy: true })
  })

  it('has no topic for the default title, a bare mark or an empty title', () => {
    expect(parseTerminalTitle('✳ Claude Code')).toEqual({ topic: null, busy: false })
    expect(parseTerminalTitle('◐ Claude Code')).toEqual({ topic: null, busy: true })
    expect(parseTerminalTitle('✳')).toEqual({ topic: null, busy: false })
  })

  it('accepts the mark with an emoji variation selector', () => {
    expect(parseTerminalTitle('✳️  review plan ')).toEqual({ topic: 'review plan', busy: false })
  })

  it('ignores titles that do not come from Claude Code', () => {
    // the process name before the first draw, and programs started in the same console
    expect(parseTerminalTitle('claude')).toBeNull()
    expect(parseTerminalTitle('npm run dev')).toBeNull()
    expect(parseTerminalTitle('')).toBeNull()
  })
})
