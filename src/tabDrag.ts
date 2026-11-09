import type { PointerEvent } from 'react'
import type { Rect } from '@shared/layout'
import { followDrag } from './pointerDrag'
import { endTabDrag, refocus, showDrop, startTabDrag, useStore } from './store'

/** What lights up in the workspace while something is dragged over it, in the workspace's own coordinates. */
export interface DropPreview {
  rect: Rect
  /** Files from outside: the highlight says what dropping them does. */
  files?: 'paste' | 'open'
}

/** The place among the tabs that a drop would go to. */
export type DropMark =
  /** In front of or behind a tab, in an area's strip or in the sidebar's list. */
  | { kind: 'tab'; tabId: string; side: 'before' | 'after' }
  /** The header of an area in the sidebar's list: to the end of that area's tabs. */
  | { kind: 'area'; groupId: string }
  /** Files over the sidebar's list: their folders open in the area that has the keyboard. */
  | { kind: 'list' }

/** What letting go of a dragged tab at a point would do. */
export interface TabDrop {
  preview: Rect | null
  mark: DropMark | null
  /** Carries the drop out for the tab `id`. */
  commit(id: string): void
}

/**
 * Where a dragged tab would go at a point (client coordinates); null where it cannot be dropped.
 * With `scroll`, a list of tabs under the pointer near one of its ends scrolls a step first.
 */
export type DropFinder = (clientX: number, clientY: number, scroll: boolean) => TabDrop | null

/** The same for one part of the window only: undefined for a point outside that part. */
export type ListTarget = (clientX: number, clientY: number, scroll: boolean) => TabDrop | null | undefined

// how close to an end of a list of tabs a dragged tab starts scrolling it, by how much, and how often
const SCROLL_EDGE = { row: 48, column: 32 } as const
const SCROLL_STEP = 14
const SCROLL_TICK_MS = 50

let workspaceDrop: DropFinder | null = null

/**
 * The workspace says what a tab dropped on it does: on an area's strip, in an area's body, along
 * a side of the window. A drag that starts anywhere, in a strip or in the sidebar, asks it.
 */
export function registerWorkspaceDrop(fn: DropFinder | null): void {
  workspaceDrop = fn
}

/** Scrolls a list of tabs a step while the pointer is near one of its ends: a strip along x, a column along y. */
export function scrollAtEdge(el: HTMLElement, clientX: number, clientY: number, column: boolean): void {
  const { left, right, top, bottom } = el.getBoundingClientRect()
  const edge = column ? SCROLL_EDGE.column : SCROLL_EDGE.row
  const [at, start, end] = column ? [clientY, top, bottom] : [clientX, left, right]
  const step = at < start + edge ? -SCROLL_STEP : at > end - edge ? SCROLL_STEP : 0
  if (step === 0) return
  if (column) el.scrollTop += step
  else el.scrollLeft += step
}

/**
 * A button went down on a tab: a few pixels on, it drags the tab. `listTarget` is asked first
 * (the sidebar's list, for a tab that starts there); everywhere else the workspace decides.
 */
export function startTabPointerDrag(e: PointerEvent<HTMLElement>, id: string, listTarget?: ListTarget): void {
  const find = (x: number, y: number, scroll: boolean): TabDrop | null => {
    // the tab closed during the drag
    if (!useStore.getState().tabDrag) return null
    const here = listTarget?.(x, y, scroll)
    return here !== undefined ? here : (workspaceDrop?.(x, y, scroll) ?? null)
  }
  let at: { x: number; y: number } | null = null
  let ticker: ReturnType<typeof setInterval> | undefined
  const show = (scroll: boolean): TabDrop | null => {
    const next = at ? find(at.x, at.y, scroll) : null
    showDrop(next?.preview ? { rect: next.preview } : null, next?.mark ?? null)
    return next
  }
  const finish = (): void => {
    clearInterval(ticker)
    showDrop(null, null)
    endTabDrag()
  }
  followDrag(e, {
    start() {
      startTabDrag(id)
      // near an end of a list of tabs it keeps scrolling while the pointer rests there
      ticker = setInterval(() => show(true), SCROLL_TICK_MS)
    },
    move(x, y) {
      at = { x, y }
      return show(false) !== null
    },
    drop(x, y) {
      const dragged = useStore.getState().tabDrag
      const next = find(x, y, false)
      finish()
      if (!dragged || !next) {
        refocus()
        return
      }
      next.commit(dragged)
    },
    cancel() {
      finish()
      refocus()
    }
  })
}
