import { describe, expect, it } from 'vitest'
import { clampAccountsHeight, DEFAULT_SPLIT, isOneCard, ONE_CARD_PX, parseSplit, serializeSplit, snapSplit, TABS_MIN_PX } from '../src/shared/sidebarSplit'

describe('sidebar split', () => {
  const space = 700

  it('keeps the accounts at least one card tall and leaves the tabs their room', () => {
    expect(clampAccountsHeight(300, space)).toBe(300)
    expect(clampAccountsHeight(40, space)).toBe(ONE_CARD_PX)
    expect(clampAccountsHeight(650, space)).toBe(space - TABS_MIN_PX)
    expect(clampAccountsHeight(260.6, space)).toBe(261)
    // a sidebar too short for both: one card wins
    expect(clampAccountsHeight(260, 250)).toBe(ONE_CARD_PX)
    expect(clampAccountsHeight(Number.NaN, space)).toBe(ONE_CARD_PX)
  })

  it('folds to one card when let go shorter than one', () => {
    expect(snapSplit(ONE_CARD_PX - 1, space)).toEqual({ mode: 'one' })
    expect(snapSplit(-50, space)).toEqual({ mode: 'one' })
    expect(snapSplit(ONE_CARD_PX, space)).toEqual({ height: ONE_CARD_PX })
    expect(snapSplit(420, space)).toEqual({ height: 420 })
    expect(snapSplit(900, space)).toEqual({ height: space - TABS_MIN_PX })
  })

  it('reads what it stored, and the default for anything else', () => {
    for (const split of [{ height: 333 }, { mode: 'one' as const }, DEFAULT_SPLIT]) expect(parseSplit(serializeSplit(split))).toEqual(split)
    expect(parseSplit(null)).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('')).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('{')).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('42')).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('null')).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('{"mode":"two"}')).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('{"height":"300"}')).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('{"height":-5}')).toEqual(DEFAULT_SPLIT)
    expect(parseSplit('{"height":1e999}')).toEqual(DEFAULT_SPLIT)
    // a stored height is whole and never less than one card
    expect(parseSplit('{"height":12.4}')).toEqual({ height: ONE_CARD_PX })
    expect(parseSplit('{"height":300.4}')).toEqual({ height: 300 })
    expect(parseSplit('{"height":99999}')).toEqual({ height: 10_000 })
  })

  it('tells the folded split from a height', () => {
    expect(isOneCard({ mode: 'one' })).toBe(true)
    expect(isOneCard(DEFAULT_SPLIT)).toBe(false)
    expect(serializeSplit({ height: 250.7 })).toBe('{"height":251}')
    expect(serializeSplit({ mode: 'one' })).toBe('{"mode":"one"}')
  })
})
