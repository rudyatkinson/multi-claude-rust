import type { PointerEvent as ReactPointerEvent } from 'react'

/** How far the pointer travels with the button held before a press counts as a drag, in CSS pixels. */
export const DRAG_THRESHOLD = 4

export interface DragCallbacks {
  /** The pointer travelled far enough: the drag begins. */
  start(): void
  /** The pointer is at a point, in client coordinates. True when letting go there would drop something. */
  move(x: number, y: number): boolean
  /** The button was let go at a point, in client coordinates. */
  drop(x: number, y: number): void
  /** Escape, a lost pointer, or the window losing the focus ended the drag. */
  cancel(): void
}

/** Swallows the click that follows letting go of a drag: it is not a click on what was dragged. */
function swallowClick(): void {
  const swallow = (e: MouseEvent): void => {
    e.preventDefault()
    e.stopPropagation()
  }
  window.addEventListener('click', swallow, { capture: true, once: true })
  // the click comes right after the pointerup, or not at all
  setTimeout(() => window.removeEventListener('click', swallow, true), 0)
}

/**
 * Follows a press of the main button that may become a drag of the element it went down on.
 *
 * In-page drags use pointer events: while the webview takes files dropped from outside (Tauri's
 * `dragDropEnabled`), WebView2 does not deliver HTML5 drag events for drags within the page.
 */
export function followDrag(e: ReactPointerEvent<HTMLElement>, callbacks: DragCallbacks): void {
  if (e.button !== 0 || !e.isPrimary) return
  const el = e.currentTarget
  const { pointerId } = e
  const startX = e.clientX
  const startY = e.clientY
  let dragging = false
  // Escape ended the drag; the button is still down
  let cancelled = false

  const cursor = (droppable: boolean): void => {
    document.body.classList.add('pointer-drag')
    document.body.classList.toggle('no-drop', !droppable)
  }

  const stop = (): void => {
    window.removeEventListener('pointermove', onMove, true)
    window.removeEventListener('pointerup', onUp, true)
    window.removeEventListener('pointercancel', onLost, true)
    window.removeEventListener('keydown', onKey, true)
    window.removeEventListener('blur', onLost)
    document.body.classList.remove('pointer-drag', 'no-drop')
    try {
      if (el.hasPointerCapture(pointerId)) el.releasePointerCapture(pointerId)
    } catch {
      // the element is gone, and the capture with it
    }
  }

  const cancel = (): void => {
    if (!dragging || cancelled) return
    cancelled = true
    document.body.classList.remove('pointer-drag', 'no-drop')
    callbacks.cancel()
  }

  function onMove(ev: PointerEvent): void {
    if (ev.pointerId !== pointerId || cancelled) return
    // the button was let go where no pointerup reached the window
    if (ev.buttons === 0) {
      stop()
      cancel()
      return
    }
    if (!dragging) {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD) return
      dragging = true
      try {
        el.setPointerCapture(pointerId)
      } catch {
        // without the capture the window still hears the pointer while it is over the window
      }
      callbacks.start()
    }
    cursor(callbacks.move(ev.clientX, ev.clientY))
  }

  function onUp(ev: PointerEvent): void {
    if (ev.pointerId !== pointerId) return
    stop()
    if (!dragging) return
    swallowClick()
    if (!cancelled) callbacks.drop(ev.clientX, ev.clientY)
  }

  function onLost(ev: Event): void {
    if (ev instanceof PointerEvent && ev.pointerId !== pointerId) return
    stop()
    cancel()
  }

  function onKey(ev: KeyboardEvent): void {
    if (ev.key !== 'Escape' || !dragging || cancelled) return
    // the key ends the drag and nothing else: no panel or dialog closes with it
    ev.preventDefault()
    ev.stopPropagation()
    cancel()
  }

  window.addEventListener('pointermove', onMove, true)
  window.addEventListener('pointerup', onUp, true)
  window.addEventListener('pointercancel', onLost, true)
  window.addEventListener('keydown', onKey, true)
  window.addEventListener('blur', onLost)
}
