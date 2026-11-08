import type { ReactNode } from 'react'

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg
      width="14"
      height="14"
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

/** A window with its left pane marked: show or hide the sidebar. */
export function SidebarIcon() {
  return (
    <Icon>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16" />
    </Icon>
  )
}
