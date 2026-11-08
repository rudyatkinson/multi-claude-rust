import { useEffect, type RefObject } from 'react'

/**
 * Closes a popover or menu on Escape or a click outside of it. Clicks on a `data-trigger`
 * element next to the popover are left to that element, which toggles it itself; so are clicks
 * on elements matching `triggers`, for a popover that is not rendered next to what opens it.
 */
export function useDismiss(ref: RefObject<HTMLElement | null>, onClose: () => void, triggers?: string): void {
  useEffect(() => {
    const onDown = (e: MouseEvent): void => {
      const target = e.target as Element
      if (ref.current?.contains(target)) return
      if (triggers && target.closest(triggers)) return
      const trigger = target.closest('[data-trigger]')
      if (trigger && ref.current?.parentElement?.contains(trigger)) return
      onClose()
    }
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [ref, onClose, triggers])
}
