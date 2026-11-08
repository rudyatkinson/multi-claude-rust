/**
 * Moves `id` in front of `beforeId`, or to the end when `beforeId` is null. The anchor is an id
 * rather than an index because the order can change between a drag starting and its drop.
 * Returns the same array when nothing moves.
 */
export function reorder(order: readonly string[], id: string, beforeId: string | null): readonly string[] {
  if (!order.includes(id) || id === beforeId) return order
  const rest = order.filter((x) => x !== id)
  const at = beforeId === null ? rest.length : rest.indexOf(beforeId)
  // an anchor that is gone by now: leave the order alone
  if (at < 0) return order
  rest.splice(at, 0, id)
  return rest.every((x, i) => x === order[i]) ? order : rest
}

/** Where a tab starts and ends along its strip: left and right, or top and bottom in a column of tabs. */
export interface TabRect {
  id: string
  left: number
  right: number
}

export interface DropTarget {
  /** The tab the indicator is drawn on. */
  id: string
  side: 'before' | 'after'
  /** What `reorder` takes: the tab to land in front of, or null for the end. */
  beforeId: string | null
}

/**
 * Where a tab dragged to position `x` along the strip would land (the vertical position in a
 * column of tabs). `rects` are the tabs in order. Before a tab's middle means in front of it;
 * past the last tab means the end.
 */
export function dropTarget(rects: readonly TabRect[], x: number): DropTarget | null {
  if (rects.length === 0) return null
  for (const rect of rects) {
    if (x < (rect.left + rect.right) / 2) return { id: rect.id, side: 'before', beforeId: rect.id }
  }
  return { id: rects[rects.length - 1].id, side: 'after', beforeId: null }
}
