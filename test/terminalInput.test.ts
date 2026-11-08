import { describe, expect, it } from 'vitest'
import { isTyping } from '../src/shared/terminalInput'

const ESC = '\x1b'

describe('isTyping', () => {
  it('takes keys and pastes for typing', () => {
    for (const data of ['a', '\r', ' ', '\x03', ESC, `${ESC}[A`, `${ESC}OP`, `${ESC}[15~`, `${ESC}\r`, `${ESC}[200~some text${ESC}[201~`, 'ış']) {
      expect(isTyping(data), JSON.stringify(data)).toBe(true)
    }
  })

  it('does not take what the terminal reports by itself for typing', () => {
    const reports = [
      `${ESC}[I`,
      `${ESC}[O`,
      `${ESC}[<0;12;7M`,
      `${ESC}[<0;12;7m`,
      `${ESC}[<64;40;3M`,
      `${ESC}[M !!`,
      `${ESC}[32;12;7M`,
      `${ESC}[24;80R`,
      `${ESC}[?1;2c`,
      `${ESC}[>0;276;0c`,
      `${ESC}[0n`,
      `${ESC}[?2026;2$y`,
      `${ESC}[?0u`,
      `${ESC}[8;30;120t`,
      `${ESC}]11;rgb:1414/1414/1313${ESC}\\`,
      `${ESC}]10;rgb:e8e8/e6e6/dcdc\x07`,
      `${ESC}P>|xterm.js(6.0.0)${ESC}\\`
    ]
    for (const data of reports) expect(isTyping(data), JSON.stringify(data)).toBe(false)
    expect(isTyping(reports.join(''))).toBe(false)
    expect(isTyping('')).toBe(false)
  })

  it('finds a key between reports', () => {
    expect(isTyping(`${ESC}[Iy${ESC}[O`)).toBe(true)
    expect(isTyping(`${ESC}[<0;12;7M\r`)).toBe(true)
  })
})
