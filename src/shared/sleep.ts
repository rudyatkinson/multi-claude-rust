import type { SessionInfo } from './types'

/**
 * Sleeping tabs. A Claude tab that nobody has used for a while (Settings → Sessions) ends its
 * process but keeps its place, its name, what its terminal last showed and its conversation; it
 * starts again with `claude --resume` once it is clicked into or typed into. The rule for when a
 * tab sleeps is the backend's (`mc_core::sleep`).
 */

type SleepFacts = Pick<SessionInfo, 'kind' | 'status' | 'moving'>

/** The time of day of a moment, as 14:05, in the local time zone. */
export function clockTime(at: number): string {
  const d = new Date(at)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

/** What a sleeping tab's mark says when the pointer rests on it. */
export function sleepLabel(sleptAt: number | undefined): string {
  const since = sleptAt === undefined ? 'Sleeping' : `Sleeping since ${clockTime(sleptAt)}`
  return `${since} · click into the tab or type to wake`
}

export const isAsleep = (session: Pick<SessionInfo, 'status'> | undefined): boolean => session?.status === 'sleeping'

/** "Sleep now" applies to a running Claude tab that is not on its way to another account. */
export const canSleep = (session: SleepFacts | undefined): boolean =>
  session !== undefined && session.kind === 'claude' && session.status === 'running' && session.moving !== true

/** What Ctrl+Shift+Z does to a tab: wakes a sleeping one, puts a running Claude tab to sleep, else nothing. */
export function sleepToggle(session: SleepFacts | undefined): 'sleep' | 'wake' | null {
  if (isAsleep(session)) return 'wake'
  return canSleep(session) ? 'sleep' : null
}

/** Keys that only modify another one: pressing them alone (as on the way to Ctrl+Tab) does not wake a tab. */
const MODIFIERS = new Set(['Shift', 'Control', 'Alt', 'AltGraph', 'Meta', 'OS', 'Super', 'Hyper', 'Fn', 'FnLock', 'CapsLock', 'NumLock', 'ScrollLock'])

/** Whether a key pressed in a sleeping tab's terminal wakes it. */
export const wakesOnKey = (key: string): boolean => !MODIFIERS.has(key) && key !== 'Unidentified' && key !== 'Dead'
