import { useEffect, useRef, type CSSProperties, type KeyboardEvent } from 'react'
import type { ProfileView } from '@shared/types'
import { planLabel } from '../format'
import { useStore } from '../store'
import { UsageRows } from './UsageBar'

/** Accounts to choose one from, each with its plan and current usage. Enter picks the first. */
export function AccountChoices({ accounts, onChoose }: { accounts: readonly ProfileView[]; onChoose(profileId: string): void }) {
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const first = useRef<HTMLDivElement>(null)

  useEffect(() => first.current?.focus(), [])

  return (
    <div className="account-choices">
      {accounts.map((profile, index) => {
        const plan = planLabel(profile.account)
        return (
          // a div, not a <button>: the usage bars inside are block content
          <div
            key={profile.id}
            role="button"
            tabIndex={0}
            ref={index === 0 ? first : undefined}
            className="account-choice"
            style={{ '--accent': profile.color } as CSSProperties}
            onClick={() => onChoose(profile.id)}
            onKeyDown={(e: KeyboardEvent) => {
              if (e.key !== 'Enter' && e.key !== ' ') return
              e.preventDefault()
              onChoose(profile.id)
            }}
          >
            <div className="card-head">
              <span className="dot" />
              <span className="card-name">{profile.name}</span>
              {plan && <span className="badge">{plan}</span>}
            </div>
            <div className="card-email">{profile.account?.email}</div>
            <UsageRows usage={usage[profile.id]} now={now} />
          </div>
        )
      })}
    </div>
  )
}
