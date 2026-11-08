import type { ReactNode } from 'react'

/** 18 px as drawn; the small buttons of cards and panel headers keep them at 14 (`.icon-btn svg` in the stylesheet). */
function Icon({ children }: { children: ReactNode }) {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  )
}

/** A circular arrow: fetch fresh data. */
export function RefreshIcon() {
  return (
    <Icon>
      <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
    </Icon>
  )
}

/** Two opposing arrows: copy shared settings over from the main profile. */
export function SyncIcon() {
  return (
    <Icon>
      <path d="M8 3 4 7l4 4" />
      <path d="M4 7h16" />
      <path d="m16 21 4-4-4-4" />
      <path d="M20 17H4" />
    </Icon>
  )
}

/** A clock running backwards: past conversations. */
export function HistoryIcon() {
  return (
    <Icon>
      <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
      <path d="M3 3v5h5" />
      <path d="M12 7v5l3 2" />
    </Icon>
  )
}

/** Three boxes in a chain: workflows. */
export function WorkflowIcon() {
  return (
    <Icon>
      <rect x="2" y="3" width="7" height="6" rx="1.5" />
      <rect x="15" y="9" width="7" height="6" rx="1.5" />
      <rect x="2" y="15" width="7" height="6" rx="1.5" />
      <path d="M9 6h3a3 3 0 0 1 3 3v3" />
      <path d="M15 12h-3a3 3 0 0 0-3 3v3" />
    </Icon>
  )
}

/** An open book: the Library. */
export function LibraryIcon() {
  return (
    <Icon>
      <path d="M12 6.5C10.2 5 7.6 4.3 3 4.3v14c4.6 0 7.2.7 9 2.2 1.8-1.5 4.4-2.2 9-2.2v-14c-4.6 0-7.2.7-9 2.2Z" />
      <path d="M12 6.5v14" />
    </Icon>
  )
}

/** A sparkle with two small glints: what is new in this version. */
export function SparkleIcon() {
  return (
    <Icon>
      <path d="M10 4 11.6 8.4 16 10 11.6 11.6 10 16 8.4 11.6 4 10 8.4 8.4Z" />
      <path d="M18 3v4" />
      <path d="M16 5h4" />
      <path d="M18 16v4" />
      <path d="M16 18h4" />
    </Icon>
  )
}

/** A cogwheel: the settings. */
export function SettingsIcon() {
  return (
    <Icon>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
    </Icon>
  )
}

/** A window with its left pane marked: show or hide the sidebar. */
export function SidebarIcon() {
  return (
    <Icon>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16" />
    </Icon>
  )
}
