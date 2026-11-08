// What a terminal sends without anyone touching the keyboard: that it gained or lost the focus,
// where the mouse is, and its answers to what the program in it asked.
const REPORTS = new RegExp(
  [
    '\\x1b\\[[IO]', // focus in, focus out
    '\\x1b\\[<\\d+;\\d+;\\d+[Mm]', // mouse
    '\\x1b\\[M[\\s\\S]{3}', // mouse, the old encoding
    '\\x1b\\[\\d+;\\d+;\\d+M', // mouse, the urxvt encoding
    '\\x1b\\[\\??\\d+;\\d+R', // cursor position
    '\\x1b\\[[?>=][\\d;]*c', // device attributes
    '\\x1b\\[\\??\\d+n', // device status
    '\\x1b\\[\\??[\\d;]+\\$y', // state of a mode
    '\\x1b\\[\\?[\\d;]*u', // keyboard protocol
    '\\x1b\\[[\\d;]+t', // window size
    '\\x1b\\][^\\x07\\x1b]*(?:\\x07|\\x1b\\\\)', // colours and other OSC answers
    '\\x1bP[^\\x1b]*\\x1b\\\\' // version and settings
  ].join('|'),
  'g'
)

/**
 * Whether what a terminal sends to its program holds anything a person typed or pasted. The
 * reports above travel the same way, and a tab that is merely clicked sends some of them.
 */
export function isTyping(data: string): boolean {
  return data.replace(REPORTS, '') !== ''
}
