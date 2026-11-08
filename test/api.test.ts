import { beforeEach, describe, expect, it, vi } from 'vitest'
import { INVOKE_CHANNELS, SEND_CHANNELS } from '../src/shared/ipc'

const calls: { cmd: string; args: unknown; done(): void }[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: unknown) =>
    new Promise<void>((resolve) => {
      calls.push({ cmd, args, done: resolve })
    })
}))

vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { INVOKE_PARAMS, commandName, errorMessage, mc, namedArgs } = await import('../src/api')

const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0))

beforeEach(() => {
  calls.length = 0
})

describe('INVOKE_PARAMS', () => {
  it('names the arguments of every channel, and of nothing else', () => {
    expect(Object.keys(INVOKE_PARAMS).sort()).toEqual([...INVOKE_CHANNELS].sort())
  })

  it('uses camelCase names that are unique within a channel', () => {
    for (const [channel, names] of Object.entries(INVOKE_PARAMS)) {
      expect(new Set(names).size, channel).toBe(names.length)
      for (const name of names) expect(name, channel).toMatch(/^[a-z][A-Za-z0-9]*$/)
    }
  })
})

describe('commands', () => {
  it('replaces the colon of a channel', () => {
    expect(commandName('profiles:create')).toBe('profiles_create')
    expect(commandName('app:getState')).toBe('app_get_state')
    expect(commandName('skills:pickImport')).toBe('skills_pick_import')
    expect(commandName('pty:write')).toBe('pty_write')
    expect(new Set([...INVOKE_CHANNELS, ...SEND_CHANNELS].map(commandName)).size).toBe(INVOKE_CHANNELS.length + SEND_CHANNELS.length)
  })

  it('passes positional arguments by name and leaves out missing ones', () => {
    expect(namedArgs('skills:save', [{ rootId: 'r', dirName: 'd' }, 'text', null])).toEqual({ ref: { rootId: 'r', dirName: 'd' }, text: 'text', baseHash: null })
    expect(namedArgs('dialog:pickFolder', [])).toEqual({})
    expect(namedArgs('workflows:run', [{ workflowId: 'w', cwd: '/', task: '' }, undefined])).toEqual({ input: { workflowId: 'w', cwd: '/', task: '' } })
  })

  it('invokes the command with named arguments', () => {
    void mc.invoke('workflows:output', 'run1', 2)
    expect(calls).toEqual([{ cmd: 'workflows_output', args: { runId: 'run1', index: 2 }, done: expect.any(Function) }])
  })
})

describe('send', () => {
  it('keeps the text typed into one terminal in order, adding up what waits', async () => {
    mc.send('pty:write', { id: 'a', data: 'h' })
    mc.send('pty:write', { id: 'a', data: 'e' })
    mc.send('pty:write', { id: 'a', data: 'y' })
    mc.send('pty:write', { id: 'b', data: '!' })
    expect(calls.map((c) => [c.cmd, c.args])).toEqual([
      ['pty_write', { payload: { id: 'a', data: 'h' } }],
      ['pty_write', { payload: { id: 'b', data: '!' } }]
    ])
    calls[0].done()
    await settle()
    expect(calls[2].args).toEqual({ payload: { id: 'a', data: 'ey' } })
    calls[2].done()
    await settle()
    expect(calls).toHaveLength(3)
  })

  it('sends only the latest of messages that replace each other', async () => {
    mc.send('pty:resize', { id: 'a', cols: 80, rows: 24 })
    mc.send('pty:resize', { id: 'a', cols: 100, rows: 30 })
    mc.send('pty:resize', { id: 'a', cols: 120, rows: 40 })
    calls[0].done()
    await settle()
    expect(calls.map((c) => c.args)).toEqual([{ payload: { id: 'a', cols: 80, rows: 24 } }, { payload: { id: 'a', cols: 120, rows: 40 } }])
    calls[1].done()
    await settle()
  })
})

describe('errorMessage', () => {
  it('takes the text of a string, an Error or an object with a message', () => {
    expect(errorMessage('No such account')).toBe('No such account')
    expect(errorMessage(new Error('boom'))).toBe('boom')
    expect(errorMessage({ message: 'nope' })).toBe('nope')
  })
})
