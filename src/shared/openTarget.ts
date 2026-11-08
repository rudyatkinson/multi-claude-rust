type AccountRef = { id: string; isDefault?: boolean; account?: { loggedIn: boolean } }

/** The account a new session starts with unless the user picks another: Main when it is logged in. */
export function defaultProfileId(profiles: readonly AccountRef[]): string | null {
  const loggedIn = profiles.filter((p) => p.account?.loggedIn)
  return (loggedIn.find((p) => p.isDefault) ?? loggedIn[0])?.id ?? null
}

export type OpenTarget = { kind: 'direct'; profileId: string } | { kind: 'pick' } | { kind: 'none' }

/**
 * Which account opens a folder that arrives from outside the app: the one the folder is pinned
 * to, or the only logged-in one, directly; otherwise the user picks. Profiles whose status is not
 * known yet may still turn out to be logged in, so they also lead to the picker.
 */
export function chooseOpenTarget(profiles: readonly AccountRef[], pinnedId?: string | null): OpenTarget {
  const loggedIn = profiles.filter((p) => p.account?.loggedIn)
  if (pinnedId && loggedIn.some((p) => p.id === pinnedId)) return { kind: 'direct', profileId: pinnedId }
  const unknown = profiles.filter((p) => !p.account)
  if (loggedIn.length === 1 && unknown.length === 0) return { kind: 'direct', profileId: loggedIn[0].id }
  if (loggedIn.length === 0 && unknown.length === 0) return { kind: 'none' }
  return { kind: 'pick' }
}
