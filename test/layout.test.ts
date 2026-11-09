import { describe, expect, it } from 'vitest'
import {
  activate,
  activeIdOf,
  bodyRect,
  canDrop,
  canSplit,
  cycleTab,
  deserialize,
  emptyLayout,
  flatten,
  focusStep,
  groupOf,
  groupsOf,
  hitTest,
  isPanel,
  layoutRects,
  mergeAll,
  minSize,
  moveTab,
  panelKindOf,
  panelTabId,
  parseLayout,
  parseSavedLayout,
  previewRect,
  ratioAt,
  reconcile,
  remapSaved,
  removeTab,
  serialize,
  setRatio,
  splitRoot,
  splitWith,
  stripRect,
  tabAt,
  visibleIds,
  type Edge,
  type Hit,
  type HitOptions,
  type Layout,
  type Metrics,
  type Node,
  type SessionRef
} from '../src/shared/layout'
import { dropTarget, reorder } from '../src/shared/tabOrder'

const refs = (...ids: string[]): SessionRef[] => ids.map((id) => ({ id }))
/** A layout with one area holding these tabs, the first one in front. */
const one = (...ids: string[]): Layout => reconcile(emptyLayout(), refs(...ids))
/** "a b | c" style picture of a layout: groups in reading order, the shown tab in brackets, * for the keyboard. */
const picture = (l: Layout): string =>
  groupsOf(l.root)
    .map((g) => `${g.id === l.focused ? '*' : ''}${g.tabs.map((t) => (t === g.active ? `[${t}]` : t)).join(' ')}`)
    .join(' | ')
const shape = (node: Node): string => (node.kind === 'group' ? node.tabs.join('') : `${node.dir === 'row' ? 'H' : 'V'}(${shape(node.first)},${shape(node.second)})`)

/** What must hold for every layout the transitions produce. */
function check(l: Layout): void {
  const all = groupsOf(l.root)
  const nodeIds: string[] = []
  const visit = (node: Node): void => {
    nodeIds.push(node.id)
    if (node.kind === 'split') {
      expect(node.ratio).toBeGreaterThanOrEqual(0.05)
      expect(node.ratio).toBeLessThanOrEqual(0.95)
      visit(node.first)
      visit(node.second)
    }
  }
  visit(l.root)
  expect(new Set(nodeIds).size, 'node ids are unique').toBe(nodeIds.length)
  expect(all.some((g) => g.id === l.focused), 'the keyboard is in an existing group').toBe(true)
  const tabs = flatten(l)
  expect(new Set(tabs).size, 'a session has one tab').toBe(tabs.length)
  for (const g of all) {
    if (g.tabs.length === 0) {
      expect(all.length, 'only a lone group may be empty').toBe(1)
      expect(g.active).toBeNull()
    } else {
      expect(g.tabs, 'a group shows one of its tabs').toContain(g.active)
    }
  }
}

describe('reorder', () => {
  const order = ['a', 'b', 'c', 'd']

  it('moves a tab in front of another one or to the end', () => {
    expect(reorder(order, 'd', 'a')).toEqual(['d', 'a', 'b', 'c'])
    expect(reorder(order, 'a', 'd')).toEqual(['b', 'c', 'a', 'd'])
    expect(reorder(order, 'a', null)).toEqual(['b', 'c', 'd', 'a'])
  })

  it('returns the same array when nothing moves', () => {
    expect(reorder(order, 'b', 'c')).toBe(order)
    expect(reorder(order, 'd', null)).toBe(order)
    expect(reorder(order, 'b', 'b')).toBe(order)
  })

  it('ignores a tab or an anchor that no longer exists', () => {
    expect(reorder(order, 'x', 'a')).toBe(order)
    expect(reorder(order, 'a', 'x')).toBe(order)
  })
})

describe('dropTarget', () => {
  const rects = [
    { id: 'a', left: 0, right: 100 },
    { id: 'b', left: 100, right: 300 },
    { id: 'c', left: 300, right: 400 }
  ]

  it('lands in front of the tab whose left half is under the pointer', () => {
    expect(dropTarget(rects, 10)).toEqual({ id: 'a', side: 'before', beforeId: 'a' })
    expect(dropTarget(rects, 60)).toEqual({ id: 'b', side: 'before', beforeId: 'b' })
    expect(dropTarget(rects, 201)).toEqual({ id: 'c', side: 'before', beforeId: 'c' })
  })

  it('lands at the end past the middle of the last tab, also in the empty space behind it', () => {
    expect(dropTarget(rects, 351)).toEqual({ id: 'c', side: 'after', beforeId: null })
    expect(dropTarget(rects, 900)).toEqual({ id: 'c', side: 'after', beforeId: null })
    expect(dropTarget([], 5)).toBeNull()
  })
})

describe('one area', () => {
  it('starts empty and takes sessions as tabs', () => {
    expect(picture(emptyLayout())).toBe('*')
    const l = one('a', 'b', 'c')
    expect(picture(l)).toBe('*[a] b c')
    expect(activeIdOf(l)).toBe('a')
    check(l)
  })

  it('shows the chosen tab', () => {
    const l = activate(one('a', 'b', 'c'), 'c')
    expect(picture(l)).toBe('*a b [c]')
    expect(activate(l, 'c')).toBe(l)
    expect(activate(l, 'nope')).toBe(l)
  })

  it('goes to a tab by position and cycles through them', () => {
    const l = one('a', 'b', 'c')
    expect(activeIdOf(tabAt(l, 2))).toBe('c')
    expect(activeIdOf(tabAt(l, 'last'))).toBe('c')
    expect(tabAt(l, 7)).toBe(l)
    expect(activeIdOf(cycleTab(l, 1))).toBe('b')
    expect(activeIdOf(cycleTab(l, -1))).toBe('c')
    expect(cycleTab(one('a'), 1)).toEqual(one('a'))
  })

  it('reorders its tabs without changing which one is shown', () => {
    const l = one('a', 'b', 'c')
    expect(picture(moveTab(l, 'c', 'g1', 'a'))).toBe('*c [a] b')
    expect(moveTab(l, 'a', 'g1', 'b')).toBe(l)
  })
})

describe('splitting', () => {
  const abc = one('a', 'b', 'c')

  it('gives a tab an area of its own on the side it was dropped on', () => {
    const right = splitWith(abc, 'c', 'g1', 'right')
    expect(picture(right)).toBe('[a] b | *[c]')
    expect(shape(right.root)).toBe('H(ab,c)')
    expect(shape(splitWith(abc, 'c', 'g1', 'left').root)).toBe('H(c,ab)')
    expect(shape(splitWith(abc, 'c', 'g1', 'top').root)).toBe('V(c,ab)')
    expect(shape(splitWith(abc, 'c', 'g1', 'bottom').root)).toBe('V(ab,c)')
    check(right)
  })

  it('shows the neighbour in the area a shown tab left', () => {
    expect(picture(splitWith(abc, 'a', 'g1', 'right'))).toBe('[b] c | *[a]')
    expect(picture(splitWith(activate(abc, 'c'), 'c', 'g1', 'right'))).toBe('a [b] | *[c]')
  })

  it('cannot split off the only tab of an area next to itself', () => {
    const lone = one('a')
    expect(canSplit(lone, 'a', 'g1')).toBe(false)
    expect(splitWith(lone, 'a', 'g1', 'right')).toBe(lone)
    expect(splitRoot(lone, 'a', 'left')).toBe(lone)
  })

  it('nests: areas can be split again, in either direction', () => {
    const l1 = splitWith(one('a', 'b', 'c', 'd'), 'b', 'g1', 'right')
    const right = groupOf(l1, 'b')!.id
    const l2 = splitWith(l1, 'c', right, 'bottom')
    expect(shape(l2.root)).toBe('H(ad,V(b,c))')
    expect(picture(l2)).toBe('[a] d | [b] | *[c]')
    check(l2)
  })

  it('moves the only tab of one area next to another: its old area folds away', () => {
    const l1 = splitWith(splitWith(one('a', 'b', 'c'), 'b', 'g1', 'right'), 'c', 'g1', 'bottom')
    expect(shape(l1.root)).toBe('H(V(a,c),b)')
    const l2 = splitWith(l1, 'c', groupOf(l1, 'b')!.id, 'right')
    expect(shape(l2.root)).toBe('H(a,H(b,c))')
    check(l2)
  })

  it('splits the whole window along one of its sides', () => {
    const l1 = splitWith(one('a', 'b', 'c'), 'b', 'g1', 'right')
    const l2 = splitRoot(l1, 'c', 'bottom')
    expect(shape(l2.root)).toBe('V(H(a,b),c)')
    expect(activeIdOf(l2)).toBe('c')
    // taking the only tab of an area: that area folds, the tab spans the window
    expect(shape(splitRoot(l2, 'b', 'left').root)).toBe('H(b,V(a,c))')
    check(l2)
  })

  it('moves a tab into another area, at a position or at the end', () => {
    const l1 = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')
    const right = groupOf(l1, 'c')!.id
    expect(picture(moveTab(l1, 'a', right, 'c'))).toBe('[b] | *[a] c')
    expect(picture(moveTab(l1, 'a', right, null))).toBe('[b] | *c [a]')
    // the last tab leaving folds the area
    const merged = moveTab(l1, 'c', 'g1', null)
    expect(picture(merged)).toBe('*a b [c]')
    expect(merged.root.kind).toBe('group')
  })

  it('merges every area back into one, keeping the tab that had the keyboard in front', () => {
    const l = splitWith(splitWith(one('a', 'b', 'c'), 'b', 'g1', 'right'), 'c', 'g1', 'bottom')
    expect(picture(mergeAll(l))).toBe('*a [c] b')
    expect(mergeAll(one('a'))).toEqual(one('a'))
  })

  it('moves the keyboard from area to area, and through all tabs in reading order', () => {
    const l = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')
    expect(picture(focusStep(l, 1))).toBe('*[a] b | [c]')
    expect(picture(focusStep(focusStep(l, 1), -1))).toBe('[a] b | *[c]')
    // c (right) -> a (left) -> b (left) -> c
    expect(picture(cycleTab(l, 1))).toBe('*[a] b | [c]')
    expect(picture(cycleTab(cycleTab(l, 1), 1))).toBe('*a [b] | [c]')
    expect(focusStep(one('a', 'b'), 1)).toEqual(one('a', 'b'))
  })

  it('goes by position within the area that has the keyboard only', () => {
    const l = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')
    expect(tabAt(l, 1)).toBe(l)
    expect(activeIdOf(tabAt(focusStep(l, 1), 1))).toBe('b')
  })

  it('changes the share of a split, within limits', () => {
    const l = splitWith(one('a', 'b'), 'b', 'g1', 'right')
    const id = l.root.id
    expect((setRatio(l, id, 0.3).root as { ratio: number }).ratio).toBe(0.3)
    expect((setRatio(l, id, 2).root as { ratio: number }).ratio).toBe(0.95)
    expect(setRatio(l, id, 0.5)).toBe(l)
    expect(setRatio(l, 'nope', 0.3)).toBe(l)
  })

  it('knows what is on screen', () => {
    const l = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')
    expect(visibleIds(l)).toEqual(['a', 'c'])
  })
})

describe('reconcile', () => {
  const split = splitWith(one('a', 'b', 'c', 'd'), 'd', 'g1', 'right')

  it('returns the same layout when the sessions are the same', () => {
    expect(reconcile(split, refs('a', 'b', 'c', 'd'))).toBe(split)
    // the list arrives again whenever a session's model or context use changes
    expect(reconcile(split, refs('d', 'c', 'b', 'a'))).toBe(split)
    const empty = emptyLayout()
    expect(reconcile(empty, [])).toBe(empty)
  })

  it('shows the tab that took a closed tab’s position, else the last one', () => {
    expect(picture(reconcile(activate(split, 'b'), refs('a', 'c', 'd')))).toBe('*a [c] | [d]')
    expect(picture(reconcile(activate(split, 'c'), refs('a', 'b', 'd')))).toBe('*a [b] | [d]')
    // a closed tab in the background changes nothing on screen
    expect(picture(reconcile(activate(split, 'b'), refs('a', 'b', 'd')))).toBe('*a [b] | [d]')
  })

  it('folds an area whose last tab closed and hands the keyboard on', () => {
    const next = reconcile(split, refs('a', 'b', 'c'))
    expect(picture(next)).toBe('*[a] b c')
    expect(next.root.kind).toBe('group')
  })

  it('survives closing tabs across areas at once', () => {
    const three = splitWith(split, 'c', 'g1', 'bottom')
    const next = reconcile(three, refs('b'))
    expect(picture(next)).toBe('*[b]')
    check(next)
    expect(picture(reconcile(three, []))).toBe('*')
  })

  it('a restarted session keeps its tab under its new id, shown or not', () => {
    const shown = reconcile(split, [...refs('a', 'b', 'c'), { id: 'd2', replaces: 'd' }])
    expect(picture(shown)).toBe('[a] b c | *[d2]')
    const hidden = reconcile(split, [...refs('a', 'c', 'd'), { id: 'b2', replaces: 'b' }])
    expect(picture(hidden)).toBe('[a] b2 c | *[d]')
    const front = reconcile(activate(split, 'b'), [...refs('a', 'c', 'd'), { id: 'b2', replaces: 'b' }])
    expect(picture(front)).toBe('*a [b2] c | [d]')
  })

  it('puts a new session into the area that has the keyboard, without bringing it to the front', () => {
    expect(picture(reconcile(split, refs('a', 'b', 'c', 'd', 'e')))).toBe('[a] b c | *[d] e')
    expect(picture(reconcile(emptyLayout(), refs('x')))).toBe('*[x]')
  })

  it('follows where a new session asks to be placed', () => {
    const after = reconcile(split, [...refs('a', 'b', 'c', 'd'), { id: 'e', place: { afterId: 'a' } }])
    expect(picture(after)).toBe('[a] e b c | *[d]')
    const inGroup = reconcile(split, [...refs('a', 'b', 'c', 'd'), { id: 'e', place: { groupId: 'g1' } }])
    expect(picture(inGroup)).toBe('[a] b c e | *[d]')
  })

  it('falls back when the wished place is gone', () => {
    const gone = reconcile(split, [...refs('a', 'b', 'c', 'd'), { id: 'e', place: { groupId: 'closed', afterId: 'closed-tab' } }])
    expect(picture(gone)).toBe('[a] b c | *[d] e')
  })
})

describe('layout invariants', () => {
  it('hold through long random sequences of everything a user can do', () => {
    // a small deterministic generator, so a failure can be reproduced
    let seed = 20261007
    const random = (n: number): number => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff
      return seed % n
    }
    const pick = <T>(items: readonly T[]): T => items[random(items.length)]
    const edges = ['left', 'right', 'top', 'bottom'] as const

    for (let run = 0; run < 40; run++) {
      let live = ['s1', 's2', 's3']
      let made = 3
      let l = reconcile(emptyLayout(), refs(...live))
      for (let step = 0; step < 120; step++) {
        const groups = groupsOf(l.root).map((g) => g.id)
        const op = random(10)
        if (live.length === 0 || op === 0) {
          live = [...live, `s${++made}`]
          l = reconcile(l, live.map((id, i) => (i === live.length - 1 && random(2) ? { id, place: { afterId: pick(live) } } : { id })))
        } else if (op === 1) {
          const closing = pick(live)
          live = live.filter((id) => id !== closing)
          l = reconcile(l, refs(...live))
        } else if (op === 2) {
          const old = pick(live)
          const fresh = `s${++made}`
          live = live.map((id) => (id === old ? fresh : id))
          l = reconcile(l, live.map((id) => (id === fresh ? { id, replaces: old } : { id })))
        } else if (op === 3) l = activate(l, pick(live))
        else if (op === 4) l = moveTab(l, pick(live), pick(groups), random(2) ? pick(live) : null)
        else if (op === 5) l = splitWith(l, pick(live), pick(groups), pick(edges))
        else if (op === 6) l = splitRoot(l, pick(live), pick(edges))
        else if (op === 7) l = random(4) === 0 ? mergeAll(l) : focusStep(l, random(2) ? 1 : -1)
        else if (op === 8) l = cycleTab(l, random(2) ? 1 : -1)
        else l = tabAt(l, random(4))

        check(l)
        expect([...flatten(l)].sort(), `run ${run} step ${step}`).toEqual([...live].sort())
        // what was saved comes back the same
        const order = flatten(l)
        const saved = serialize(l, (id) => order.indexOf(id))
        if (saved) {
          const back = deserialize(parseSavedLayout(JSON.parse(JSON.stringify(saved)))!, (i) => order[i], order)
          expect(picture(back)).toBe(picture(l))
          expect(shape(back.root)).toBe(shape(l.root))
        }
      }
    }
  })
})

describe('geometry', () => {
  const metrics = { gap: 1, minW: 200, minH: 120 }
  const row = splitWith(one('a', 'b'), 'b', 'g1', 'right')
  const three = splitWith(splitWith(one('a', 'b', 'c'), 'b', 'g1', 'right'), 'c', groupOf(splitWith(one('a', 'b', 'c'), 'b', 'g1', 'right'), 'b')!.id, 'bottom')

  it('fills the window exactly, with the gap between areas', () => {
    const { groups, dividers } = layoutRects(row.root, 1001, 600, metrics)
    expect(groups).toEqual([
      { id: 'g1', x: 0, y: 0, w: 500, h: 600 },
      { id: groups[1].id, x: 501, y: 0, w: 500, h: 600 }
    ])
    expect(dividers[0]).toMatchObject({ x: 500, y: 0, w: 1, h: 600, dir: 'row' })
    // odd sizes: the second area takes what is left, nothing is lost to rounding
    const odd = layoutRects(row.root, 1000, 600, metrics).groups
    expect(odd[0].w + 1 + odd[1].w).toBe(1000)
  })

  it('lays out nested splits', () => {
    const { groups, dividers } = layoutRects(three.root, 1001, 601, metrics)
    expect(groups.map((g) => `${g.x},${g.y} ${g.w}x${g.h}`)).toEqual(['0,0 500x601', '501,0 500x300', '501,301 500x300'])
    expect(dividers.map((d) => d.dir)).toEqual(['row', 'column'])
  })

  it('keeps every area at its minimum size while there is room', () => {
    const squeezed = setRatio(row, row.root.id, 0.05)
    const { groups, dividers } = layoutRects(squeezed.root, 1001, 600, metrics)
    expect(groups[0].w).toBe(200)
    expect(dividers[0].min).toBeCloseTo(0.2)
    expect(dividers[0].max).toBeCloseTo(0.8)
    expect(minSize(three.root, metrics)).toEqual({ w: 401, h: 241 })
    // no room for both minimums: share what there is instead of overflowing
    const tiny = layoutRects(row.root, 301, 600, metrics).groups
    expect(tiny[0].w + 1 + tiny[1].w).toBe(301)
  })

  it('turns a pointer position on a divider into a ratio within its limits', () => {
    const { dividers } = layoutRects(row.root, 1001, 600, metrics)
    expect(ratioAt(dividers[0], 300.5, 10)).toBeCloseTo(0.3)
    expect(ratioAt(dividers[0], 5, 10)).toBeCloseTo(0.2)
    expect(ratioAt(dividers[0], 995, 10)).toBeCloseTo(0.8)
  })
})

describe('drop zones', () => {
  const metrics = { gap: 1, minW: 200, minH: 120 }
  const options = { strip: 44, rootBand: 24, edgeShare: 0.25 }
  const lone = one('a', 'b')
  const row = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')
  const right = groupOf(row, 'c')!.id
  const W = 1001
  const H = 644

  it('reads a single area: its tab strip, its four sides and its middle', () => {
    const geo = layoutRects(lone.root, W, H, metrics)
    expect(hitTest(geo, W, H, 500, 10, options)).toEqual({ kind: 'strip', groupId: 'g1' })
    expect(hitTest(geo, W, H, 500, 344, options)).toEqual({ kind: 'move', groupId: 'g1' })
    expect(hitTest(geo, W, H, 30, 344, options)).toEqual({ kind: 'split', groupId: 'g1', edge: 'left' })
    expect(hitTest(geo, W, H, 990, 344, options)).toEqual({ kind: 'split', groupId: 'g1', edge: 'right' })
    expect(hitTest(geo, W, H, 500, 60, options)).toEqual({ kind: 'split', groupId: 'g1', edge: 'top' })
    expect(hitTest(geo, W, H, 500, 630, options)).toEqual({ kind: 'split', groupId: 'g1', edge: 'bottom' })
    // a corner goes to the nearer side
    expect(hitTest(geo, W, H, 20, 400, options)).toEqual({ kind: 'split', groupId: 'g1', edge: 'left' })
    expect(hitTest(geo, W, H, 5000, 10, options)).toBeNull()
  })

  it('with several areas, the outer band of the window means the whole window', () => {
    const geo = layoutRects(row.root, W, H, metrics)
    expect(hitTest(geo, W, H, 10, 344, options)).toEqual({ kind: 'root', edge: 'left' })
    expect(hitTest(geo, W, H, 995, 344, options)).toEqual({ kind: 'root', edge: 'right' })
    expect(hitTest(geo, W, H, 300, 640, options)).toEqual({ kind: 'root', edge: 'bottom' })
    // just inside the band it is the area's own side again
    expect(hitTest(geo, W, H, 40, 344, options)).toEqual({ kind: 'split', groupId: 'g1', edge: 'left' })
    expect(hitTest(geo, W, H, 520, 344, options)).toEqual({ kind: 'split', groupId: right, edge: 'left' })
    // the top of the window belongs to the tab strips
    expect(hitTest(geo, W, H, 700, 5, options)).toEqual({ kind: 'strip', groupId: right })
  })

  it('previews the half that the new area would take', () => {
    const geo = layoutRects(row.root, W, H, metrics)
    expect(previewRect(geo, W, H, { kind: 'split', groupId: 'g1', edge: 'right' }, options)).toEqual({ x: 250, y: 44, w: 250, h: 600 })
    expect(previewRect(geo, W, H, { kind: 'move', groupId: right }, options)).toEqual({ x: 501, y: 44, w: 500, h: 600 })
    expect(previewRect(geo, W, H, { kind: 'root', edge: 'bottom' }, options)).toEqual({ x: 0, y: 322, w: W, h: 322 })
    expect(previewRect(geo, W, H, { kind: 'strip', groupId: 'g1' }, options)).toBeNull()
  })

  it('refuses drops that change nothing or leave an area too small', () => {
    const geo = layoutRects(row.root, W, H, metrics)
    // onto its own area
    expect(canDrop(row, 'a', { kind: 'move', groupId: 'g1' }, geo, W, H, metrics)).toBe(false)
    expect(canDrop(row, 'a', { kind: 'move', groupId: right }, geo, W, H, metrics)).toBe(true)
    // the only tab of an area beside itself
    expect(canDrop(row, 'c', { kind: 'split', groupId: right, edge: 'left' }, geo, W, H, metrics)).toBe(false)
    expect(canDrop(row, 'a', { kind: 'split', groupId: 'g1', edge: 'left' }, geo, W, H, metrics)).toBe(true)
    expect(canDrop(row, 'a', { kind: 'strip', groupId: 'g1' }, geo, W, H, metrics)).toBe(true)
    // a narrow window: no room for a third column, still room to stack
    const narrow = layoutRects(row.root, 601, H, metrics)
    expect(canDrop(row, 'a', { kind: 'split', groupId: 'g1', edge: 'left' }, narrow, 601, H, metrics)).toBe(false)
    expect(canDrop(row, 'a', { kind: 'split', groupId: 'g1', edge: 'bottom' }, narrow, 601, H, metrics)).toBe(true)
    expect(canDrop(row, 'a', { kind: 'root', edge: 'left' }, narrow, 601, H, metrics)).toBe(false)
    expect(canDrop(row, 'a', { kind: 'root', edge: 'bottom' }, narrow, 601, H, metrics)).toBe(true)
  })
})

describe('saving a layout', () => {
  const l = splitWith(splitWith(one('a', 'b', 'c', 'd'), 'c', 'g1', 'right'), 'd', 'g1', 'bottom')
  const order = flatten(l)

  it('stores tab positions instead of session ids and comes back the same', () => {
    const saved = serialize(l, (id) => order.indexOf(id))!
    expect(saved).toEqual({
      root: { dir: 'row', ratio: 0.5, first: { dir: 'column', ratio: 0.5, first: { tabs: [0, 1], active: 0 }, second: { tabs: [2], active: 2 } }, second: { tabs: [3], active: 3 } },
      focused: 1
    })
    const ids = ['A', 'B', 'D', 'C']
    const back = deserialize(saved, (i) => ids[i], ids)
    expect(picture(back)).toBe('[A] B | *[D] | [C]')
    expect(shape(back.root)).toBe('H(V(AB,D),C)')
    check(back)
  })

  it('leaves out sessions that are not saved, and the areas that only held those', () => {
    // b is a login tab, d has ended: neither is saved
    const saved = serialize(l, (id) => ({ a: 0, c: 1 })[id])!
    expect(saved).toEqual({ root: { dir: 'row', ratio: 0.5, first: { tabs: [0], active: 0 }, second: { tabs: [1], active: 1 } }, focused: 0 })
    expect(serialize(l, () => undefined)).toBeNull()
  })

  it('restores what could be reopened; tabs that could not are dropped with their areas', () => {
    const saved = serialize(l, (id) => order.indexOf(id))!
    // the tab in the bottom-left area was not reopened (its account is gone)
    const back = deserialize(saved, (i) => ['A', 'B', undefined, 'C'][i], ['A', 'B', 'C'])
    expect(shape(back.root)).toBe('H(AB,C)')
    check(back)
    // nothing reopened at all
    expect(picture(deserialize(saved, () => undefined, []))).toBe('*')
  })

  it('puts reopened tabs the saved layout does not mention into the area with the keyboard', () => {
    const saved = serialize(l, (id) => order.indexOf(id))!
    const back = deserialize(saved, (i) => ['A', 'B', 'D', 'C'][i], ['A', 'B', 'D', 'C', 'E'])
    expect(picture(back)).toBe('[A] B | *[D] E | [C]')
  })

  it('renumbers a saved layout when entries of the tab list were skipped', () => {
    const saved = serialize(l, (id) => order.indexOf(id))!
    // entry 1 was unreadable: the ones after it move up
    const moved = remapSaved(saved, (i) => (i === 1 ? null : i > 1 ? i - 1 : i))!
    expect(moved.root).toMatchObject({ first: { first: { tabs: [0], active: 0 }, second: { tabs: [1], active: 1 } }, second: { tabs: [2], active: 2 } })
    expect(remapSaved(saved, () => null)).toBeNull()
  })

  it('rejects a saved layout that is not one', () => {
    expect(parseSavedLayout(null)).toBeNull()
    expect(parseSavedLayout({ root: { tabs: [] } })).toBeNull()
    expect(parseSavedLayout({ root: { tabs: [0, 0], active: 0 } })).toBeNull()
    expect(parseSavedLayout({ root: { tabs: [-1], active: 0 } })).toBeNull()
    expect(parseSavedLayout({ root: { dir: 'diagonal', first: { tabs: [0] }, second: { tabs: [1] } } })).toBeNull()
    // the same tab in two areas
    expect(parseSavedLayout({ root: { dir: 'row', ratio: 0.5, first: { tabs: [0] }, second: { tabs: [0] } } })).toBeNull()
    // a bad ratio or focus is repaired rather than fatal
    expect(parseSavedLayout({ root: { dir: 'row', ratio: 9, first: { tabs: [0], active: 0 }, second: { tabs: [1], active: 1 } }, focused: 7 })).toEqual({
      root: { dir: 'row', ratio: 0.95, first: { tabs: [0], active: 0 }, second: { tabs: [1], active: 1 } },
      focused: 0
    })
    let deep: unknown = { tabs: [0], active: 0 }
    for (let i = 1; i < 40; i++) deep = { dir: 'row', ratio: 0.5, first: deep, second: { tabs: [i], active: i } }
    expect(parseSavedLayout({ root: deep })).toBeNull()
  })
})

describe('panels', () => {
  it('reconcile keeps panel tabs that no session has', () => {
    const l = one('a', 'panel:usage', 'b')
    expect(isPanel('panel:usage') && !isPanel('a')).toBe(true)
    // the session list never names a panel
    expect(reconcile(l, refs('a', 'b'))).toBe(l)
    expect(picture(reconcile(l, refs('a')))).toBe('*[a] panel:usage')
    // with every session gone, a shown panel stays, and so does an area that holds one
    const shown = activate(l, 'panel:usage')
    expect(picture(reconcile(shown, []))).toBe('*[panel:usage]')
    const apart = splitWith(l, 'panel:usage', 'g1', 'right')
    expect(picture(reconcile(apart, []))).toBe('*[panel:usage]')
    expect(shape(reconcile(apart, refs('b')).root)).toBe('H(b,panel:usage)')
    // a tab that merely looks like one is still a session when it lacks the prefix
    expect(picture(reconcile(one('a', 'panel-usage'), refs('a')))).toBe('*[a]')
  })

  it('removeTab takes a panel out and shows the neighbour', () => {
    const l = activate(one('a', 'panel:library', 'b'), 'panel:library')
    expect(picture(removeTab(l, 'panel:library'))).toBe('*a [b]')
    expect(removeTab(l, 'panel:workflows')).toBe(l)
    // the area of a panel alone folds away, and the keyboard goes to an area that is left
    const apart = splitWith(l, 'panel:library', 'g1', 'right')
    expect(picture(apart)).toBe('a [b] | *[panel:library]')
    const closed = removeTab(apart, 'panel:library')
    expect(picture(closed)).toBe('*a [b]')
    check(closed)
    // the last tab of all leaves one empty area
    expect(picture(removeTab(one('panel:workflows'), 'panel:workflows'))).toBe('*')
  })

  it('knows the panels of this version by their tab ids', () => {
    expect(panelTabId('workflows')).toBe('panel:workflows')
    expect(panelKindOf('panel:library')).toBe('library')
    expect(panelKindOf('panel:usage')).toBeNull()
    expect(panelKindOf('library')).toBeNull()
  })
})

describe('strip on the left', () => {
  const W = 1001
  const H = 600
  const metrics: Metrics = { gap: 1, minW: 200, minH: 120, stripSide: 'left', strip: 160 }
  const options: HitOptions = { strip: 160, stripSide: 'left', rootBand: 24, edgeShare: 0.25 }
  const row = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')
  const right = groupOf(row, 'c')!.id

  it('an area needs room for its strip beside it', () => {
    expect(minSize(one('a').root, metrics)).toEqual({ w: 360, h: 120 })
    // with the strip on top it is the area's own minimum
    expect(minSize(one('a').root, { gap: 1, minW: 200, minH: 120 })).toEqual({ w: 200, h: 120 })
    const two = splitWith(one('a', 'b'), 'b', 'g1', 'right')
    expect(minSize(two.root, metrics)).toEqual({ w: 721, h: 120 })
    const squeezed = setRatio(two, two.root.id, 0.05)
    const { groups, dividers } = layoutRects(squeezed.root, W, H, metrics)
    expect(groups[0].w).toBe(360)
    expect(dividers[0].min).toBeCloseTo(0.36)
    expect(dividers[0].max).toBeCloseTo(0.64)
  })

  it('reads the strip column and measures the sides in the body', () => {
    const geo = layoutRects(one('a', 'b').root, W, H, metrics)
    const hit = (x: number, y: number): Hit | null => hitTest(geo, W, H, x, y, options)
    const split = (edge: Edge): Hit => ({ kind: 'split', groupId: 'g1', edge })
    expect(hit(100, 300)).toEqual({ kind: 'strip', groupId: 'g1' })
    // the top left corner is the strip, not the top side
    expect(hit(100, 10)).toEqual({ kind: 'strip', groupId: 'g1' })
    expect(hit(600, 300)).toEqual({ kind: 'move', groupId: 'g1' })
    expect(hit(170, 300)).toEqual(split('left'))
    expect(hit(990, 300)).toEqual(split('right'))
    expect(hit(600, 10)).toEqual(split('top'))
    expect(hit(600, 590)).toEqual(split('bottom'))
  })

  it('with several areas, the band runs along the sides the strips do not take', () => {
    const geo = layoutRects(row.root, W, H, metrics)
    const hit = (x: number, y: number): Hit | null => hitTest(geo, W, H, x, y, options)
    // the left of the window belongs to the strips
    expect(hit(10, 300)).toEqual({ kind: 'strip', groupId: 'g1' })
    expect(hit(995, 300)).toEqual({ kind: 'root', edge: 'right' })
    expect(hit(800, 5)).toEqual({ kind: 'root', edge: 'top' })
    expect(hit(800, 590)).toEqual({ kind: 'root', edge: 'bottom' })
    // a corner goes to the right before the top
    expect(hit(995, 5)).toEqual({ kind: 'root', edge: 'right' })
    // the right area's strip, then its body
    expect(hit(520, 300)).toEqual({ kind: 'strip', groupId: right })
    expect(hit(700, 300)).toEqual({ kind: 'split', groupId: right, edge: 'left' })
  })

  it('previews and checks drops in the body', () => {
    const geo = layoutRects(row.root, W, H, metrics)
    expect(previewRect(geo, W, H, { kind: 'move', groupId: right }, options)).toEqual({ x: 661, y: 0, w: 340, h: 600 })
    expect(previewRect(geo, W, H, { kind: 'split', groupId: 'g1', edge: 'right' }, options)).toEqual({ x: 330, y: 0, w: 170, h: 600 })
    expect(previewRect(geo, W, H, { kind: 'root', edge: 'left' }, options)).toEqual({ x: 0, y: 0, w: 500.5, h: 600 })
    // two columns with their strips do not fit in an area 500 wide; with the strip on top they would
    const leftOfG1: Hit = { kind: 'split', groupId: 'g1', edge: 'left' }
    expect(canDrop(row, 'a', leftOfG1, geo, W, H, metrics)).toBe(false)
    expect(canDrop(row, 'a', leftOfG1, geo, W, H, { gap: 1, minW: 200, minH: 120 })).toBe(true)
    expect(canDrop(row, 'a', { kind: 'split', groupId: 'g1', edge: 'bottom' }, geo, W, H, metrics)).toBe(true)
    expect(canDrop(row, 'a', { kind: 'root', edge: 'left' }, geo, W, H, metrics)).toBe(false)
    expect(canDrop(row, 'a', { kind: 'root', edge: 'bottom' }, geo, W, H, metrics)).toBe(true)
  })
})

describe('no strip (the tabs in the sidebar)', () => {
  const W = 1000
  const H = 600
  const metrics: Metrics = { gap: 1, minW: 260, minH: 170 }
  const options: HitOptions = { strip: 0, stripSide: 'top', rootBand: 28, edgeShare: 0.25 }
  const row = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')
  const right = groupOf(row, 'c')!.id

  it('has no strip zone: the top of an area is a side of its body', () => {
    const geo = layoutRects(one('a', 'b').root, W, H, metrics)
    const hit = (x: number, y: number): Hit | null => hitTest(geo, W, H, x, y, options)
    for (const [x, y] of [
      [500, 0],
      [500, 5],
      [10, 0],
      [990, 599]
    ]) {
      expect(hit(x, y)?.kind).not.toBe('strip')
    }
    expect(hit(500, 0)).toEqual({ kind: 'split', groupId: 'g1', edge: 'top' })
    expect(hit(500, 300)).toEqual({ kind: 'move', groupId: 'g1' })
    expect(hit(500, 590)).toEqual({ kind: 'split', groupId: 'g1', edge: 'bottom' })
  })

  it('gives every area its full height as its body', () => {
    const geo = layoutRects(row.root, W, H, metrics)
    for (const g of geo.groups) {
      expect(bodyRect(g, 0)).toEqual({ x: g.x, y: g.y, w: g.w, h: g.h })
      expect(stripRect(g, 0).h).toBe(0)
    }
    const r = geo.groups.find((g) => g.id === right)!
    expect(previewRect(geo, W, H, { kind: 'move', groupId: right }, options)).toEqual({ x: r.x, y: r.y, w: r.w, h: r.h })
    // along the top of the window there is no band: the strips are gone, but the top keeps belonging to the areas
    const hit = (x: number, y: number): Hit | null => hitTest(geo, W, H, x, y, options)
    expect(hit(800, 3)).toEqual({ kind: 'split', groupId: right, edge: 'top' })
    expect(hit(5, 300)).toEqual({ kind: 'root', edge: 'left' })
    expect(hit(800, 595)).toEqual({ kind: 'root', edge: 'bottom' })
  })
})

describe('parseLayout', () => {
  const l = splitWith(one('a', 'b', 'c'), 'c', 'g1', 'right')

  it('accepts a layout that went through JSON', () => {
    const back = parseLayout(JSON.parse(JSON.stringify(l)))!
    expect(back).toEqual(l)
    // new areas made afterwards do not reuse an id
    const next = splitWith(back, 'b', 'g1', 'bottom')
    check(next)
  })

  it('rejects malformed trees', () => {
    expect(parseLayout(null)).toBeNull()
    expect(parseLayout({ root: { kind: 'group', id: 'g1', tabs: ['a', 'a'], active: 'a' }, focused: 'g1' })).toBeNull()
    expect(parseLayout({ root: { kind: 'split', id: 's1', dir: 'row', ratio: 0.5, first: { kind: 'group', id: 'x', tabs: ['a'] }, second: { kind: 'group', id: 'x', tabs: ['b'] } } })).toBeNull()
    expect(parseLayout({ root: { kind: 'group', id: 'g1', tabs: [1, 2] } })).toBeNull()
  })

  it('repairs what can be repaired: a missing focus, a tab shown that is not there, empty areas', () => {
    const repaired = parseLayout({
      root: {
        kind: 'split',
        id: 's9',
        dir: 'row',
        ratio: 0.5,
        first: { kind: 'group', id: 'g4', tabs: ['a', 'b'], active: 'zzz' },
        second: { kind: 'group', id: 'g7', tabs: [], active: null }
      },
      focused: 'nope'
    })!
    expect(picture(repaired)).toBe('*a [b]')
    expect(repaired.seq).toBeGreaterThanOrEqual(9)
    check(repaired)
  })
})
