import { describe, expect, it } from 'vitest'
import { canSleep, clockTime, isAsleep, sleepLabel, sleepToggle, wakesOnKey } from '../src/shared/sleep'
import { tabMark } from '../src/shared/turn'
import { sleepIdleMinutesOf } from '../src/shared/types'

// a moment of the local clock, whatever the time zone of the machine running the tests
const at = (hours: number, minutes: number): number => new Date(2026, 10, 8, hours, minutes, 30).getTime()

describe('the mark of a sleeping tab', () => {
  it('says since when it sleeps and how to wake it', () => {
    expect(clockTime(at(14, 5))).toBe('14:05')
    expect(clockTime(at(9, 0))).toBe('09:00')
    expect(sleepLabel(at(14, 5))).toBe('Sleeping since 14:05 · click into the tab or type to wake')
    expect(sleepLabel(undefined)).toBe('Sleeping · click into the tab or type to wake')
  })

  it('is the crescent whatever the title or the turn said last', () => {
    const sign = tabMark({ exited: false, asleep: true, sleptAt: at(14, 5), busy: true, turn: { kind: 'permission', at: 1 } })
    expect(sign).toEqual({ state: 'asleep', label: 'Sleeping since 14:05 · click into the tab or type to wake' })
    // awake tabs keep their marks
    expect(tabMark({ exited: false, busy: true }).state).toBe('busy')
    expect(tabMark({ exited: true, busy: false }).state).toBe('ended')
    expect(tabMark({ exited: false, asleep: false, busy: false }).state).toBe('idle')
  })
})

describe('sleeping and waking', () => {
  const claude = { kind: 'claude' as const, status: 'running' as const }

  it('puts only a running Claude tab to sleep', () => {
    expect(canSleep(claude)).toBe(true)
    expect(canSleep({ ...claude, kind: 'login' })).toBe(false)
    expect(canSleep({ ...claude, status: 'exited' })).toBe(false)
    expect(canSleep({ ...claude, status: 'sleeping' })).toBe(false)
    expect(canSleep({ ...claude, moving: true })).toBe(false)
    expect(canSleep(undefined)).toBe(false)
  })

  it('toggles with the shortcut', () => {
    expect(sleepToggle(claude)).toBe('sleep')
    expect(sleepToggle({ ...claude, status: 'sleeping' })).toBe('wake')
    expect(sleepToggle({ ...claude, status: 'exited' })).toBeNull()
    expect(sleepToggle({ ...claude, kind: 'login' })).toBeNull()
    expect(sleepToggle(undefined)).toBeNull()
    expect(isAsleep({ status: 'sleeping' })).toBe(true)
    expect(isAsleep({ status: 'running' })).toBe(false)
  })

  it('wakes on a key, but not on a modifier pressed on the way to a shortcut', () => {
    for (const key of ['a', 'Enter', ' ', 'Escape', 'ArrowUp', 'Tab', 'F2']) expect(wakesOnKey(key), key).toBe(true)
    for (const key of ['Control', 'Shift', 'Alt', 'AltGraph', 'Meta', 'CapsLock', 'Dead', 'Unidentified']) expect(wakesOnKey(key), key).toBe(false)
  })
})

describe('sleepIdleMinutes', () => {
  it('is 15 unless set, 0 for never, and held to a day', () => {
    expect(sleepIdleMinutesOf({})).toBe(15)
    expect(sleepIdleMinutesOf({ sleepIdleMinutes: 0 })).toBe(0)
    expect(sleepIdleMinutesOf({ sleepIdleMinutes: 2.6 })).toBe(3)
    expect(sleepIdleMinutesOf({ sleepIdleMinutes: -4 })).toBe(0)
    expect(sleepIdleMinutesOf({ sleepIdleMinutes: 99999 })).toBe(1440)
  })
})
