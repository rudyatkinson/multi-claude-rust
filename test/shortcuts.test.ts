import { describe, expect, it } from 'vitest'
import { matchShortcut, type KeyLike } from '../src/shared/shortcuts'

/** key("t", "KeyT", "ctrl+shift") */
function key(k: string, code: string, mods = ''): KeyLike {
  const has = (m: string): boolean => mods.split('+').includes(m)
  return { key: k, code, ctrlKey: has('ctrl'), shiftKey: has('shift'), altKey: has('alt'), metaKey: has('meta') }
}

describe('matchShortcut', () => {
  it('keeps the existing tab and sidebar shortcuts', () => {
    expect(matchShortcut(key('T', 'KeyT', 'ctrl+shift'))).toEqual({ type: 'new' })
    expect(matchShortcut(key('W', 'KeyW', 'ctrl+shift'))).toEqual({ type: 'close' })
    expect(matchShortcut(key('B', 'KeyB', 'ctrl+shift'))).toEqual({ type: 'sidebar' })
    expect(matchShortcut(key('Tab', 'Tab', 'ctrl'))).toEqual({ type: 'next' })
    expect(matchShortcut(key('Tab', 'Tab', 'ctrl+shift'))).toEqual({ type: 'prev' })
  })

  it('puts the active tab to sleep or wakes it with Ctrl+Shift+Z, leaving Ctrl+Z to the terminal', () => {
    expect(matchShortcut(key('Z', 'KeyZ', 'ctrl+shift'))).toEqual({ type: 'sleep' })
    // suspend in a shell, and Claude Code's own use of it
    expect(matchShortcut(key('z', 'KeyZ', 'ctrl'))).toBeNull()
    expect(matchShortcut(key('Z', 'KeyZ', 'ctrl+alt+shift'))).toBeNull()
  })

  it('leaves the keys Claude Code uses to the terminal', () => {
    for (const [k, code] of [['t', 'KeyT'], ['w', 'KeyW'], ['b', 'KeyB'], ['f', 'KeyF'], ['d', 'KeyD'], ['c', 'KeyC'], ['v', 'KeyV']]) {
      expect(matchShortcut(key(k, code, 'ctrl'))).toBeNull()
    }
    // undo: Ctrl+_ is Ctrl+Shift+-
    expect(matchShortcut(key('_', 'Minus', 'ctrl+shift'))).toBeNull()
    expect(matchShortcut(key('Tab', 'Tab', 'shift'))).toBeNull()
    expect(matchShortcut(key('a', 'KeyA'))).toBeNull()
    expect(matchShortcut(key('Enter', 'Enter', 'shift'))).toBeNull()
  })

  it('goes to a tab by its position, with 9 as the last one', () => {
    expect(matchShortcut(key('1', 'Digit1', 'ctrl'))).toEqual({ type: 'tab', index: 0 })
    expect(matchShortcut(key('8', 'Digit8', 'ctrl'))).toEqual({ type: 'tab', index: 7 })
    expect(matchShortcut(key('9', 'Digit9', 'ctrl'))).toEqual({ type: 'tab', index: 'last' })
    // AZERTY: the digit row gives "&" without Shift, but it is still the first key
    expect(matchShortcut(key('&', 'Digit1', 'ctrl'))).toEqual({ type: 'tab', index: 0 })
  })

  it('does not fire for AltGr, which Windows reports as Ctrl+Alt', () => {
    // Turkish Q: { [ ] } are AltGr+7, 8, 9, 0
    expect(matchShortcut(key('{', 'Digit7', 'ctrl+alt'))).toBeNull()
    expect(matchShortcut(key('[', 'Digit8', 'ctrl+alt'))).toBeNull()
    expect(matchShortcut(key(']', 'Digit9', 'ctrl+alt'))).toBeNull()
    expect(matchShortcut(key('}', 'Digit0', 'ctrl+alt'))).toBeNull()
    expect(matchShortcut(key('T', 'KeyT', 'ctrl+alt+shift'))).toBeNull()
  })

  it('zooms by the character typed, wherever the layout puts it', () => {
    // US: "=" unshifted, "+" with Shift, both on the Equal key
    expect(matchShortcut(key('=', 'Equal', 'ctrl'))).toEqual({ type: 'zoom', step: 1 })
    expect(matchShortcut(key('+', 'Equal', 'ctrl+shift'))).toEqual({ type: 'zoom', step: 1 })
    expect(matchShortcut(key('-', 'Minus', 'ctrl'))).toEqual({ type: 'zoom', step: -1 })
    expect(matchShortcut(key('0', 'Digit0', 'ctrl'))).toEqual({ type: 'zoom', step: 0 })
    // Turkish Q: "-" is on the Equal key, "=" is Shift+0, "+" is Shift+4
    expect(matchShortcut(key('-', 'Equal', 'ctrl'))).toEqual({ type: 'zoom', step: -1 })
    expect(matchShortcut(key('=', 'Digit0', 'ctrl+shift'))).toEqual({ type: 'zoom', step: 1 })
    expect(matchShortcut(key('+', 'Digit4', 'ctrl+shift'))).toEqual({ type: 'zoom', step: 1 })
    // number pad
    expect(matchShortcut(key('+', 'NumpadAdd', 'ctrl'))).toEqual({ type: 'zoom', step: 1 })
    expect(matchShortcut(key('-', 'NumpadSubtract', 'ctrl'))).toEqual({ type: 'zoom', step: -1 })
  })

  it('has search, splitting and moving between areas', () => {
    expect(matchShortcut(key('F', 'KeyF', 'ctrl+shift'))).toEqual({ type: 'search' })
    expect(matchShortcut(key('D', 'KeyD', 'ctrl+shift'))).toEqual({ type: 'split' })
    expect(matchShortcut(key('F6', 'F6'))).toEqual({ type: 'area', step: 1 })
    expect(matchShortcut(key('F6', 'F6', 'shift'))).toEqual({ type: 'area', step: -1 })
    expect(matchShortcut(key('F6', 'F6', 'alt'))).toBeNull()
  })

  it('opens the conversation history', () => {
    expect(matchShortcut(key('H', 'KeyH', 'ctrl+shift'))).toEqual({ type: 'history' })
    // Ctrl+H is backspace in a terminal
    expect(matchShortcut(key('h', 'KeyH', 'ctrl'))).toBeNull()
  })

  it('ignores keys of an input method and the Windows or Command key', () => {
    expect(matchShortcut({ ...key('T', 'KeyT', 'ctrl+shift'), isComposing: true })).toBeNull()
    expect(matchShortcut(key('T', 'KeyT', 'ctrl+shift+meta'))).toBeNull()
  })
})
