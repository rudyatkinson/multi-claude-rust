import { describe, expect, it } from 'vitest'
import { folderKey, pinnedAccount } from '../src/shared/folders'
import { chooseOpenTarget } from '../src/shared/openTarget'

const account = (loggedIn: boolean) => ({ loggedIn })
const both = [
  { id: 'main', account: account(true) },
  { id: 'work', account: account(true) }
]

describe('folderKey', () => {
  it('gives every spelling of a Windows folder the same key', () => {
    const key = folderKey('C:\\Code\\My App', 'win32')
    expect(key).toBe('c:\\code\\my app')
    expect(folderKey('c:/code/my app/', 'win32')).toBe(key)
    expect(folderKey('C:\\Code\\\\My App\\', 'win32')).toBe(key)
    expect(folderKey('C:\\Code/My App', 'win32')).toBe(key)
  })

  it('keeps the separator of a drive root', () => {
    expect(folderKey('C:\\', 'win32')).toBe('c:\\')
    expect(folderKey('D:/', 'win32')).toBe('d:\\')
  })

  it('keeps case and backslashes on other systems', () => {
    expect(folderKey('/home/Me/Code/', 'linux')).toBe('/home/Me/Code')
    expect(folderKey('/home//Me/Code', 'darwin')).toBe('/home/Me/Code')
    expect(folderKey('/', 'linux')).toBe('/')
    expect(folderKey('/home/me/odd\\name', 'linux')).toBe('/home/me/odd\\name')
  })
})

describe('pinnedAccount', () => {
  const pins = { [folderKey('C:\\Code\\App', 'win32')]: 'work' }

  it('finds the account a folder is pinned to, however the folder is spelled', () => {
    expect(pinnedAccount(pins, 'c:/code/app/', 'win32', both)).toBe('work')
    expect(pinnedAccount(pins, 'C:\\Code\\Other', 'win32', both)).toBeNull()
    // a subfolder is its own folder
    expect(pinnedAccount(pins, 'C:\\Code\\App\\src', 'win32', both)).toBeNull()
  })

  it('ignores the pin of an account that no longer exists', () => {
    expect(pinnedAccount(pins, 'C:\\Code\\App', 'win32', [both[0]])).toBeNull()
  })
})

describe('chooseOpenTarget with a pinned folder', () => {
  it('opens with the pinned account without asking', () => {
    expect(chooseOpenTarget(both, 'work')).toEqual({ kind: 'direct', profileId: 'work' })
    expect(chooseOpenTarget(both, null)).toEqual({ kind: 'pick' })
    expect(chooseOpenTarget(both)).toEqual({ kind: 'pick' })
  })

  it('asks when the pinned account is logged out or gone', () => {
    const loggedOut = [both[0], { id: 'work', account: account(false) }, { id: 'third', account: account(true) }]
    expect(chooseOpenTarget(loggedOut, 'work')).toEqual({ kind: 'pick' })
    expect(chooseOpenTarget(both, 'deleted')).toEqual({ kind: 'pick' })
  })

  it('still opens directly when only one account is logged in', () => {
    const one = [both[0], { id: 'work', account: account(false) }]
    expect(chooseOpenTarget(one, 'work')).toEqual({ kind: 'direct', profileId: 'main' })
  })
})
