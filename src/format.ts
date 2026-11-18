const PLAN_LABELS: Record<string, string> = { ultra: 'Ultra', max: 'Max', pro: 'Pro', team: 'Team', enterprise: 'Enterprise', free: 'Free' }

// English month names with a 24-hour clock
const LOCALE = 'en-GB'

const TIER_LABELS: Record<string, string> = { max_20x: 'Ultra', max_5x: 'Max' }

/** The plan badge of an account: the tier when Claude Code reports one (Ultra is `max_20x` of the `max` subscription), else the subscription. */
export function planLabel(account: { subscriptionType?: string; planTier?: string } | undefined): string | null {
  if (!account) return null
  const tier = account.planTier?.toLowerCase()
  if (tier) return TIER_LABELS[tier] ?? tier.split('_').map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join(' ')
  if (!account.subscriptionType) return null
  return PLAN_LABELS[account.subscriptionType.toLowerCase()] ?? account.subscriptionType
}

export { usageView, type UsageView } from '@shared/usage'

export { countdown } from '@shared/duration'

/** How long ago something happened: "just now", "5 min ago", "2 h ago", "3 d ago". */
export function ago(ms: number): string {
  const minutes = Math.floor(ms / 60_000)
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${minutes} min ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} h ago`
  return `${Math.floor(hours / 24)} d ago`
}

export function clock(ts: number): string {
  const d = new Date(ts)
  const time = d.toLocaleTimeString(LOCALE, { hour: '2-digit', minute: '2-digit' })
  return d.toDateString() === new Date().toDateString() ? time : `${d.toLocaleDateString(LOCALE, { day: 'numeric', month: 'short' })} ${time}`
}

export function dateTime(ts: number): string {
  return new Date(ts).toLocaleString(LOCALE, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

/**
 * The letters that stand for an account in the collapsed sidebar: its initial, or its first two
 * letters when another account starts with the same one ("El" and "Em").
 */
export function accountInitials(name: string, allNames: readonly string[]): string {
  const letters = Array.from(name.trim())
  const first = (letters[0] ?? '?').toUpperCase()
  const shared = allNames.filter((other) => (Array.from(other.trim())[0] ?? '').toUpperCase() === first).length > 1
  return shared && letters[1] ? first + letters[1].toLowerCase() : first
}

/**
 * The topic of a session's conversation: what its terminal title says, else what the app read
 * from the conversation file of a sleeping tab.
 */
export function topicOf(session: { topic?: string }, status?: { topic: string | null } | null): string | null {
  return status?.topic ?? session.topic ?? null
}

/** How much of a sleeping tab's last message its tooltip shows. */
const LAST_PROMPT_CHARS = 200

/** The line a sleeping tab's tooltip has for the last message typed in it; null when there is none. */
export function lastPromptLine(session: { lastPrompt?: string }): string | null {
  const text = session.lastPrompt?.replace(/\s+/g, ' ').trim()
  if (!text) return null
  const letters = Array.from(text)
  return `Last message: ${letters.length > LAST_PROMPT_CHARS ? `${letters.slice(0, LAST_PROMPT_CHARS).join('')}…` : text}`
}

/** What a tab is called: the name the user gave it, else the conversation's topic, else its folder. */
export function tabLabel(session: { name?: string; title: string }, topic?: string | null): string {
  return session.name ?? topic ?? session.title
}

/** Splits a path into its last segment and the rest, for "name — parent" style labels. */
export function splitPath(p: string): { name: string; parent: string } {
  const trimmed = p.replace(/[\\/]+$/, '')
  const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (cut < 0) return { name: trimmed || p, parent: '' }
  return { name: trimmed.slice(cut + 1) || trimmed, parent: trimmed.slice(0, cut) }
}
