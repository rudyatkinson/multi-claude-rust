/** Work shorter than this is not worth an alert: the user was probably still looking. */
const MIN_BUSY_MS = 3000

/**
 * Whether a session that just stopped working should call for attention. It does when the user
 * is not looking at it: its tab is in the background, or the whole window is.
 */
export function shouldAlert(o: { busyMs: number; tabActive: boolean; windowFocused: boolean }): boolean {
  return o.busyMs >= MIN_BUSY_MS && !(o.tabActive && o.windowFocused)
}
