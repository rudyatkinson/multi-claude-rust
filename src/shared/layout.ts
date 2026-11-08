import { reorder } from './tabOrder'

/** `row`: the first child is left of the second; `column`: it is above it. */
export type Dir = 'row' | 'column'
export type Edge = 'left' | 'right' | 'top' | 'bottom'

/** An area of the window with its own tab strip. It shows its active tab. */
export interface Group {
  kind: 'group'
  id: string
  /** Session ids, in tab order. */
  tabs: string[]
  active: string | null
}

export interface Split {
  kind: 'split'
  id: string
  dir: Dir
  /** Share of the first child, 0..1. */
  ratio: number
  first: Node
  second: Node
}

export type Node = Group | Split

/** How the sessions are arranged in the window: a tree of areas, one of which has the keyboard. */
export interface Layout {
  root: Node
  /** Id of the group that has the keyboard. */
  focused: string
  /** Last number used for a node id. */
  seq: number
}

/** Where a new session's tab should appear. */
export interface Place {
  groupId?: string
  /** Right after this session's tab, in its group. */
  afterId?: string
}

export interface SessionRef {
  id: string
  /** The session this one took the place of (a restart, a move to another account). */
  replaces?: string
  place?: Place
}

export const RATIO_MIN = 0.05
export const RATIO_MAX = 0.95
/** Deeper trees than this are refused when they come from outside; far more than anyone nests by hand. */
const MAX_DEPTH = 32

export function emptyLayout(): Layout {
  return { root: { kind: 'group', id: 'g1', tabs: [], active: null }, focused: 'g1', seq: 1 }
}

const clampRatio = (ratio: number): number => (Number.isFinite(ratio) ? Math.min(RATIO_MAX, Math.max(RATIO_MIN, ratio)) : 0.5)
const dirOf = (edge: Edge): Dir => (edge === 'left' || edge === 'right' ? 'row' : 'column')
const leads = (edge: Edge): boolean => edge === 'left' || edge === 'top'

// ---------- reading ----------

/** The groups in reading order: left before right, top before bottom. */
export function groupsOf(node: Node): Group[] {
  return node.kind === 'group' ? [node] : [...groupsOf(node.first), ...groupsOf(node.second)]
}

export function findGroup(l: Layout, groupId: string): Group | null {
  return groupsOf(l.root).find((g) => g.id === groupId) ?? null
}

export function groupOf(l: Layout, sessionId: string): Group | null {
  return groupsOf(l.root).find((g) => g.tabs.includes(sessionId)) ?? null
}

/** Every tab, group after group. */
export function flatten(l: Layout): string[] {
  return groupsOf(l.root).flatMap((g) => g.tabs)
}

/** The sessions on screen: the active tab of every group. */
export function visibleIds(l: Layout): string[] {
  return groupsOf(l.root).flatMap((g) => (g.active ? [g.active] : []))
}

export function isVisible(l: Layout, sessionId: string): boolean {
  return visibleIds(l).includes(sessionId)
}

/** The session that has the keyboard. */
export function activeIdOf(l: Layout): string | null {
  return findGroup(l, l.focused)?.active ?? null
}

// ---------- rewriting the tree ----------

/** Rebuilds the tree with every group passed through `fn`; untouched parts keep their identity. */
function mapGroups(node: Node, fn: (group: Group) => Group): Node {
  if (node.kind === 'group') return fn(node)
  const first = mapGroups(node.first, fn)
  const second = mapGroups(node.second, fn)
  return first === node.first && second === node.second ? node : { ...node, first, second }
}

function replaceGroup(node: Node, groupId: string, fn: (group: Group) => Node): Node {
  if (node.kind === 'group') return node.id === groupId ? fn(node) : node
  const first = replaceGroup(node.first, groupId, fn)
  const second = replaceGroup(node.second, groupId, fn)
  return first === node.first && second === node.second ? node : { ...node, first, second }
}

/** Removes empty groups: a split with one gives its place to its other child. A lone empty root stays. */
function collapse(node: Node): Node | null {
  if (node.kind === 'group') return node.tabs.length > 0 ? node : null
  const first = collapse(node.first)
  const second = collapse(node.second)
  if (!first) return second
  if (!second) return first
  return first === node.first && second === node.second ? node : { ...node, first, second }
}

/** The tab that stands in for a tab that left: the one now at its position, else the last one. */
function fallback(tabs: readonly string[], index: number): string | null {
  return tabs[Math.min(Math.max(index, 0), tabs.length - 1)] ?? null
}

function withoutTab(group: Group, sessionId: string): Group {
  const index = group.tabs.indexOf(sessionId)
  if (index < 0) return group
  const tabs = group.tabs.filter((id) => id !== sessionId)
  return { ...group, tabs, active: group.active === sessionId ? fallback(tabs, index) : group.active }
}

/**
 * Brings a tree back into shape after a change: no empty groups, every group shows one of its
 * tabs, and the keyboard is in a group that exists (`prefer` if it does).
 */
function settle(l: Layout, root: Node, prefer: string = l.focused, seq: number = l.seq): Layout {
  // with no tab left anywhere, one empty group remains
  const lone: Group = root.kind === 'group' ? root : { kind: 'group', id: groupsOf(root)[0].id, tabs: [], active: null }
  let next = collapse(root) ?? lone
  next = mapGroups(next, (g) => {
    const shown = g.tabs.length === 0 ? g.active === null : g.active !== null && g.tabs.includes(g.active)
    return shown ? g : { ...g, active: fallback(g.tabs, g.tabs.length - 1) }
  })
  const all = groupsOf(next)
  const focused = all.some((g) => g.id === prefer) ? prefer : all.some((g) => g.id === l.focused) ? l.focused : all[0].id
  return next === l.root && focused === l.focused && seq === l.seq ? l : { root: next, focused, seq }
}

// ---------- transitions ----------

/** A tab was chosen: its group shows it and gets the keyboard. */
export function activate(l: Layout, sessionId: string): Layout {
  const group = groupOf(l, sessionId)
  if (!group) return l
  if (group.active === sessionId && l.focused === group.id) return l
  const root = group.active === sessionId ? l.root : replaceGroup(l.root, group.id, (g) => ({ ...g, active: sessionId }))
  return { ...l, root, focused: group.id }
}

export function focusGroup(l: Layout, groupId: string): Layout {
  return l.focused === groupId || !findGroup(l, groupId) ? l : { ...l, focused: groupId }
}

/** Gives the keyboard to the next or the previous group. */
export function focusStep(l: Layout, step: 1 | -1): Layout {
  const all = groupsOf(l.root)
  if (all.length < 2) return l
  const index = all.findIndex((g) => g.id === l.focused)
  return { ...l, focused: all[(index + step + all.length) % all.length].id }
}

/** Goes to the next or the previous tab, through all groups in reading order. */
export function cycleTab(l: Layout, step: 1 | -1): Layout {
  const all = flatten(l)
  if (all.length < 2) return l
  const index = all.indexOf(activeIdOf(l) ?? '')
  return activate(l, all[(index + step + all.length) % all.length])
}

/** Goes to a tab of the group that has the keyboard, by position; `last` is its last tab. */
export function tabAt(l: Layout, index: number | 'last'): Layout {
  const tabs = findGroup(l, l.focused)?.tabs ?? []
  const target = index === 'last' ? tabs[tabs.length - 1] : tabs[index]
  return target ? activate(l, target) : l
}

/**
 * Moves a tab in front of `beforeId` in a group (to its end with null). Within its own group
 * this only reorders; into another group the tab is shown there and takes the keyboard along.
 */
export function moveTab(l: Layout, sessionId: string, toGroupId: string, beforeId: string | null): Layout {
  const from = groupOf(l, sessionId)
  const to = findGroup(l, toGroupId)
  if (!from || !to) return l
  if (from.id === to.id) {
    const tabs = reorder(from.tabs, sessionId, beforeId)
    return tabs === from.tabs ? l : { ...l, root: replaceGroup(l.root, from.id, (g) => ({ ...g, tabs: [...tabs] })) }
  }
  const root = mapGroups(l.root, (g) => {
    if (g.id === from.id) return withoutTab(g, sessionId)
    if (g.id !== to.id) return g
    const at = beforeId === null ? -1 : g.tabs.indexOf(beforeId)
    const tabs = at < 0 ? [...g.tabs, sessionId] : [...g.tabs.slice(0, at), sessionId, ...g.tabs.slice(at)]
    return { ...g, tabs, active: sessionId }
  })
  return settle(l, root, to.id)
}

/** Whether a tab can get an area of its own next to `targetGroupId`: not when it is that group's only tab. */
export function canSplit(l: Layout, sessionId: string, targetGroupId: string): boolean {
  const from = groupOf(l, sessionId)
  const target = findGroup(l, targetGroupId)
  return !!from && !!target && !(from.id === target.id && from.tabs.length === 1)
}

/** Gives a tab a new area on one side of a group; the keyboard goes with it. */
export function splitWith(l: Layout, sessionId: string, targetGroupId: string, edge: Edge): Layout {
  if (!canSplit(l, sessionId, targetGroupId)) return l
  const fresh: Group = { kind: 'group', id: `g${l.seq + 1}`, tabs: [sessionId], active: sessionId }
  const root = mapGroups(l.root, (g) => withoutTab(g, sessionId))
  const split = (target: Group): Split => ({
    kind: 'split',
    id: `s${l.seq + 2}`,
    dir: dirOf(edge),
    ratio: 0.5,
    first: leads(edge) ? fresh : target,
    second: leads(edge) ? target : fresh
  })
  return settle(l, replaceGroup(root, targetGroupId, split), fresh.id, l.seq + 2)
}

/** Gives a tab a new area along one side of the whole window. */
export function splitRoot(l: Layout, sessionId: string, edge: Edge): Layout {
  if (!groupOf(l, sessionId) || flatten(l).length < 2) return l
  const fresh: Group = { kind: 'group', id: `g${l.seq + 1}`, tabs: [sessionId], active: sessionId }
  const rest = collapse(mapGroups(l.root, (g) => withoutTab(g, sessionId)))
  if (!rest) return l
  const root: Split = { kind: 'split', id: `s${l.seq + 2}`, dir: dirOf(edge), ratio: 0.5, first: leads(edge) ? fresh : rest, second: leads(edge) ? rest : fresh }
  return settle(l, root, fresh.id, l.seq + 2)
}

export function setRatio(l: Layout, splitId: string, ratio: number): Layout {
  const next = clampRatio(ratio)
  const visit = (node: Node): Node => {
    if (node.kind === 'group') return node
    if (node.id === splitId) return node.ratio === next ? node : { ...node, ratio: next }
    const first = visit(node.first)
    const second = visit(node.second)
    return first === node.first && second === node.second ? node : { ...node, first, second }
  }
  const root = visit(l.root)
  return root === l.root ? l : { ...l, root }
}

/** Puts every tab back into one area, in reading order. */
export function mergeAll(l: Layout): Layout {
  const all = groupsOf(l.root)
  if (all.length < 2) return l
  const id = all[0].id
  return { root: { kind: 'group', id, tabs: flatten(l), active: activeIdOf(l) ?? all[0].active }, focused: id, seq: l.seq }
}

/**
 * Brings the layout in line with a new session list. A session that replaces another takes its
 * tab; tabs of sessions that are gone disappear, and the tab that took an active tab's position
 * is shown instead; new sessions get a tab where their `place` says, else in the group that has
 * the keyboard. Returns the same object when nothing changes.
 */
export function reconcile(l: Layout, sessions: readonly SessionRef[]): Layout {
  const live = new Set(sessions.map((s) => s.id))
  const successor = new Map(sessions.filter((s) => s.replaces).map((s) => [s.replaces!, s.id]))

  let root = mapGroups(l.root, (g) => {
    if (g.tabs.every((id) => live.has(id) && !successor.has(id))) return g
    const now = g.tabs.map((id) => successor.get(id) ?? id)
    const tabs = now.filter((id) => live.has(id))
    const index = g.active === null ? -1 : g.tabs.indexOf(g.active)
    const active = index >= 0 && live.has(now[index]) ? now[index] : fallback(tabs, now.slice(0, Math.max(index, 0)).filter((id) => live.has(id)).length)
    return { ...g, tabs, active }
  })

  const placed = new Set(groupsOf(root).flatMap((g) => g.tabs))
  for (const session of sessions) {
    if (placed.has(session.id)) continue
    placed.add(session.id)
    const all = groupsOf(root)
    const anchor = session.place?.afterId
    const beside = anchor ? all.find((g) => g.tabs.includes(anchor)) : undefined
    const target = beside ?? all.find((g) => g.id === session.place?.groupId) ?? all.find((g) => g.id === l.focused) ?? all[0]
    root = replaceGroup(root, target.id, (g) => {
      const at = beside ? g.tabs.indexOf(anchor!) + 1 : g.tabs.length
      const tabs = [...g.tabs.slice(0, at), session.id, ...g.tabs.slice(at)]
      // a group that showed nothing shows its first tab; otherwise the caller decides what comes to the front
      return { ...g, tabs, active: g.active ?? session.id }
    })
  }
  return settle(l, root)
}

// ---------- geometry ----------

export interface Rect {
  x: number
  y: number
  w: number
  h: number
}

export interface GroupRect extends Rect {
  id: string
}

export interface DividerRect extends Rect {
  splitId: string
  dir: Dir
  /** The box of the split it belongs to, and the ratios that keep both sides at their minimum size. */
  box: Rect
  min: number
  max: number
}

export interface Geometry {
  groups: GroupRect[]
  dividers: DividerRect[]
}

export interface Metrics {
  /** Pixels between two areas. */
  gap: number
  /** The smallest an area may get. */
  minW: number
  minH: number
}

/** The smallest size a part of the tree needs so that each of its areas keeps its minimum. */
export function minSize(node: Node, m: Metrics): { w: number; h: number } {
  if (node.kind === 'group') return { w: m.minW, h: m.minH }
  const a = minSize(node.first, m)
  const b = minSize(node.second, m)
  return node.dir === 'row' ? { w: a.w + m.gap + b.w, h: Math.max(a.h, b.h) } : { w: Math.max(a.w, b.w), h: a.h + m.gap + b.h }
}

/**
 * Where each area and each divider goes, in whole pixels. The second child of a split gets what
 * the first leaves, so the parts always add up to the whole.
 */
export function layoutRects(root: Node, w: number, h: number, m: Metrics): Geometry {
  const geometry: Geometry = { groups: [], dividers: [] }
  const place = (node: Node, box: Rect): void => {
    if (node.kind === 'group') {
      geometry.groups.push({ id: node.id, ...box })
      return
    }
    const row = node.dir === 'row'
    const total = Math.max(0, (row ? box.w : box.h) - m.gap)
    const a = minSize(node.first, m)
    const b = minSize(node.second, m)
    const least = row ? a.w : a.h
    const most = total - (row ? b.w : b.h)
    // too small for both minimums: share what there is
    const size = most >= least ? Math.min(most, Math.max(least, Math.round(total * node.ratio))) : Math.round(total * node.ratio)
    const firstBox = row ? { ...box, w: size } : { ...box, h: size }
    const secondBox = row ? { ...box, x: box.x + size + m.gap, w: total - size } : { ...box, y: box.y + size + m.gap, h: total - size }
    place(node.first, firstBox)
    geometry.dividers.push({
      splitId: node.id,
      dir: node.dir,
      ...(row ? { x: box.x + size, y: box.y, w: m.gap, h: box.h } : { x: box.x, y: box.y + size, w: box.w, h: m.gap }),
      box,
      min: total > 0 && most >= least ? least / total : RATIO_MIN,
      max: total > 0 && most >= least ? most / total : RATIO_MAX
    })
    place(node.second, secondBox)
  }
  place(root, { x: 0, y: 0, w: Math.max(0, w), h: Math.max(0, h) })
  return geometry
}

/** The ratio a divider would get if dragged to a pointer position, kept within its limits. */
export function ratioAt(divider: DividerRect, x: number, y: number): number {
  const row = divider.dir === 'row'
  const total = (row ? divider.box.w : divider.box.h) - (row ? divider.w : divider.h)
  if (total <= 0) return 0.5
  const offset = (row ? x - divider.box.x : y - divider.box.y) - (row ? divider.w : divider.h) / 2
  return Math.min(divider.max, Math.max(divider.min, offset / total))
}

export type Hit =
  | { kind: 'strip'; groupId: string }
  | { kind: 'move'; groupId: string }
  | { kind: 'split'; groupId: string; edge: Edge }
  | { kind: 'root'; edge: Edge }

export interface HitOptions {
  /** Height of a group's tab strip. */
  strip: number
  /** Width of the band along the window's sides that stands for the whole window. */
  rootBand: number
  /** Share of an area, from each side, that means "a new area on this side". */
  edgeShare: number
}

/**
 * What a tab dropped at a point would do. The tab strips come first; then, with more than one
 * area, a band along the left, right and bottom of the window splits the whole window (the top
 * belongs to the tab strips); in an area, near a side means a new area there, the middle means
 * joining that area.
 */
export function hitTest(geometry: Geometry, w: number, h: number, x: number, y: number, o: HitOptions): Hit | null {
  const group = geometry.groups.find((g) => x >= g.x && x < g.x + g.w && y >= g.y && y < g.y + g.h)
  if (!group) return null
  if (y < group.y + o.strip) return { kind: 'strip', groupId: group.id }
  if (geometry.groups.length > 1) {
    if (x < o.rootBand) return { kind: 'root', edge: 'left' }
    if (x >= w - o.rootBand) return { kind: 'root', edge: 'right' }
    if (y >= h - o.rootBand) return { kind: 'root', edge: 'bottom' }
  }
  const bodyH = group.h - o.strip
  if (group.w <= 0 || bodyH <= 0) return { kind: 'move', groupId: group.id }
  const fx = (x - group.x) / group.w
  const fy = (y - group.y - o.strip) / bodyH
  const sides: [Edge, number][] = [
    ['left', fx],
    ['right', 1 - fx],
    ['top', fy],
    ['bottom', 1 - fy]
  ]
  const [edge, distance] = sides.reduce((best, side) => (side[1] < best[1] ? side : best))
  return distance < o.edgeShare ? { kind: 'split', groupId: group.id, edge } : { kind: 'move', groupId: group.id }
}

function half(box: Rect, edge: Edge): Rect {
  if (edge === 'left') return { ...box, w: box.w / 2 }
  if (edge === 'right') return { ...box, x: box.x + box.w / 2, w: box.w / 2 }
  if (edge === 'top') return { ...box, h: box.h / 2 }
  return { ...box, y: box.y + box.h / 2, h: box.h / 2 }
}

/** The part of the window to highlight for a drop; a drop on a tab strip is shown in the strip itself. */
export function previewRect(geometry: Geometry, w: number, h: number, hit: Hit, strip: number): Rect | null {
  if (hit.kind === 'root') return half({ x: 0, y: 0, w, h }, hit.edge)
  if (hit.kind === 'strip') return null
  const group = geometry.groups.find((g) => g.id === hit.groupId)
  if (!group) return null
  const body = { x: group.x, y: group.y + strip, w: group.w, h: group.h - strip }
  return hit.kind === 'move' ? body : half(body, hit.edge)
}

/** Whether a drop would change anything and leave every area at least its minimum size. */
export function canDrop(l: Layout, sessionId: string, hit: Hit, geometry: Geometry, w: number, h: number, m: Metrics): boolean {
  const from = groupOf(l, sessionId)
  if (!from) return false
  if (hit.kind === 'strip') return true
  if (hit.kind === 'move') return from.id !== hit.groupId
  const row = dirOf(hit.edge) === 'row'
  if (hit.kind === 'root') {
    if (flatten(l).length < 2) return false
    const rest = collapse(mapGroups(l.root, (g) => withoutTab(g, sessionId)))
    if (!rest) return false
    const need = minSize(rest, m)
    return row ? w >= need.w + m.gap + m.minW : h >= need.h + m.gap + m.minH
  }
  if (!canSplit(l, sessionId, hit.groupId)) return false
  const box = geometry.groups.find((g) => g.id === hit.groupId)
  return !!box && (row ? box.w >= 2 * m.minW + m.gap : box.h >= 2 * m.minH + m.gap)
}

// ---------- saving and checking ----------

/** A layout as it is saved: tabs are positions in the saved tab list instead of session ids. */
export type SavedNode = { tabs: number[]; active: number } | { dir: Dir; ratio: number; first: SavedNode; second: SavedNode }

export interface SavedLayout {
  root: SavedNode
  /** Position of the group with the keyboard, counting the groups in reading order. */
  focused: number
}

const isSavedGroup = (node: SavedNode): node is { tabs: number[]; active: number } => 'tabs' in node

/** Rewrites the tab numbers of a saved tree; tabs mapped to null go, and with them groups left empty. */
export function remapSaved(saved: SavedLayout, map: (index: number) => number | null): SavedLayout | null {
  let groupsSeen = 0
  let focused = -1
  let kept = 0
  const visit = (node: SavedNode): SavedNode | null => {
    if (isSavedGroup(node)) {
      const wasFocused = groupsSeen++ === saved.focused
      const pairs = node.tabs.map((index) => [index, map(index)] as const).filter((pair): pair is readonly [number, number] => pair[1] !== null)
      if (pairs.length === 0) return null
      if (wasFocused) focused = kept
      kept++
      const active = pairs.find(([index]) => index === node.active)?.[1] ?? pairs[pairs.length - 1][1]
      return { tabs: pairs.map(([, index]) => index), active }
    }
    const first = visit(node.first)
    const second = visit(node.second)
    if (!first) return second
    if (!second) return first
    return { dir: node.dir, ratio: node.ratio, first, second }
  }
  const root = visit(saved.root)
  return root ? { root, focused: Math.max(focused, 0) } : null
}

/** The layout with session ids replaced by their numbers; sessions without a number are left out. */
export function serialize(l: Layout, indexOf: (sessionId: string) => number | undefined): SavedLayout | null {
  const all = groupsOf(l.root)
  const visit = (node: Node): SavedNode =>
    node.kind === 'group'
      ? { tabs: node.tabs.map((id) => indexOf(id) ?? -1), active: node.active === null ? -1 : (indexOf(node.active) ?? -1) }
      : { dir: node.dir, ratio: node.ratio, first: visit(node.first), second: visit(node.second) }
  const raw: SavedLayout = { root: visit(l.root), focused: Math.max(0, all.findIndex((g) => g.id === l.focused)) }
  return remapSaved(raw, (index) => (index >= 0 ? index : null))
}

/**
 * A layout from a saved one. `idAt` gives the session that now stands for a saved tab, if it was
 * reopened. Sessions in `all` that the saved layout does not mention join the focused group.
 */
export function deserialize(saved: SavedLayout, idAt: (index: number) => string | undefined, all: readonly string[]): Layout {
  let seq = 0
  let groupsSeen = 0
  let focused = ''
  const visit = (node: SavedNode): Node | null => {
    if (isSavedGroup(node)) {
      const position = groupsSeen++
      const tabs = node.tabs.map(idAt).filter((id): id is string => id !== undefined)
      if (tabs.length === 0) return null
      const group: Group = { kind: 'group', id: `g${++seq}`, tabs, active: idAt(node.active) ?? tabs[tabs.length - 1] }
      if (position === saved.focused) focused = group.id
      return group
    }
    const first = visit(node.first)
    const second = visit(node.second)
    if (!first) return second
    if (!second) return first
    return { kind: 'split', id: `s${++seq}`, dir: node.dir, ratio: clampRatio(node.ratio), first, second }
  }
  const root = visit(saved.root) ?? emptyLayout().root
  const base: Layout = { root, focused: focused || groupsOf(root)[0].id, seq: Math.max(seq, 1) }
  return reconcile(
    base,
    all.map((id) => ({ id }))
  )
}

/** Checks a saved layout read from a file. Null for anything malformed. */
export function parseSavedLayout(raw: unknown): SavedLayout | null {
  const seen = new Set<number>()
  let groups = 0
  const visit = (value: unknown, depth: number): SavedNode | null => {
    if (!value || typeof value !== 'object' || depth > MAX_DEPTH) return null
    const node = value as Record<string, unknown>
    if (Array.isArray(node.tabs)) {
      const tabs = node.tabs as unknown[]
      if (tabs.length === 0 || !tabs.every((t) => Number.isInteger(t) && (t as number) >= 0 && !seen.has(t as number) && !!seen.add(t as number))) return null
      groups++
      return { tabs: tabs as number[], active: Number.isInteger(node.active) ? (node.active as number) : -1 }
    }
    if (node.dir !== 'row' && node.dir !== 'column') return null
    const first = visit(node.first, depth + 1)
    const second = visit(node.second, depth + 1)
    return first && second ? { dir: node.dir, ratio: clampRatio(Number(node.ratio)), first, second } : null
  }
  const saved = (raw ?? {}) as Record<string, unknown>
  const root = visit(saved.root, 0)
  if (!root) return null
  const focused = Number.isInteger(saved.focused) && (saved.focused as number) >= 0 && (saved.focused as number) < groups ? (saved.focused as number) : 0
  return { root, focused }
}

/** Checks a layout that arrives from another process. Null for anything malformed. */
export function parseLayout(raw: unknown): Layout | null {
  const nodeIds = new Set<string>()
  const tabIds = new Set<string>()
  const isId = (value: unknown): value is string => typeof value === 'string' && value.length > 0 && value.length <= 64
  const fresh = (set: Set<string>, id: string): boolean => !set.has(id) && !!set.add(id)
  const visit = (value: unknown, depth: number): Node | null => {
    if (!value || typeof value !== 'object' || depth > MAX_DEPTH) return null
    const node = value as Record<string, unknown>
    if (!isId(node.id) || !fresh(nodeIds, node.id)) return null
    if (node.kind === 'group') {
      if (!Array.isArray(node.tabs) || !node.tabs.every((t) => isId(t) && fresh(tabIds, t))) return null
      const tabs = node.tabs as string[]
      const active = isId(node.active) && tabs.includes(node.active) ? node.active : (tabs[tabs.length - 1] ?? null)
      return { kind: 'group', id: node.id, tabs, active }
    }
    if (node.kind !== 'split' || (node.dir !== 'row' && node.dir !== 'column')) return null
    const first = visit(node.first, depth + 1)
    const second = visit(node.second, depth + 1)
    return first && second ? { kind: 'split', id: node.id, dir: node.dir, ratio: clampRatio(Number(node.ratio)), first, second } : null
  }
  const layout = (raw ?? {}) as Record<string, unknown>
  const root = visit(layout.root, 0)
  if (!root) return null
  const all = groupsOf(root)
  const focused = isId(layout.focused) && all.some((g) => g.id === layout.focused) ? layout.focused : all[0].id
  // new node ids are numbered from `seq`, so it must be past every number in use
  const used = Math.max(0, ...[...nodeIds].map((id) => Number(/\d+$/.exec(id)?.[0] ?? 0)))
  const seq = Math.max(used, Number.isInteger(layout.seq) && (layout.seq as number) >= 0 ? (layout.seq as number) : 0)
  // an empty group is only allowed as the whole layout
  return settle({ root, focused, seq }, root)
}
