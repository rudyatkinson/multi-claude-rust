import type { UsageState, UsageWindow } from '@shared/types'
import { clock, countdown, dateTime, usageView } from '../format'

interface BarProps {
  label: string
  /** What the row measures, for the tooltip. */
  title: string
  window: UsageWindow | undefined
  now: number
  asOf?: string
}

/** One usage row: label, bar, percentage and the time left until the window resets. */
function UsageBar({ label, title, window, now, asOf }: BarProps) {
  if (!window) {
    return (
      <div className="usage" title={`${title}: no data`}>
        <span className="usage-label">{label}</span>
        <div className="bar" />
        <span className="usage-value muted">—</span>
        <span className="usage-reset" />
      </div>
    )
  }
  const { resetsAt } = window
  const { pct, tone, reset } = usageView(window, now)
  const when = resetsAt === undefined ? '' : reset ? ' · has reset' : ` · resets ${dateTime(resetsAt)}`
  return (
    <div className="usage" title={`${title}${when}${asOf ? ` · as of ${asOf}` : ''}`}>
      <span className="usage-label">{label}</span>
      <div className="bar">
        <div className={`fill ${tone}`} style={{ width: `${pct}%` }} />
      </div>
      <span className="usage-value">{pct}%</span>
      <span className="usage-reset">{reset ? 'reset' : resetsAt !== undefined ? countdown(resetsAt - now) : ''}</span>
    </div>
  )
}

/** An account whose plan has no 5-hour limit (Ultra) reports weekly numbers only; its 5h row is left out. */
export function hasFiveHourRow(usage: UsageState | undefined): boolean {
  if (!usage) return true
  return usage.fiveHour !== undefined || (usage.sevenDay === undefined && !usage.models?.length)
}

/** The usage rows of an account: 5-hour session (when the plan has one), week, and one row per model-specific weekly limit. */
export function UsageRows({ usage, now }: { usage: UsageState | undefined; now: number }) {
  const asOf = usage?.observedAt ? clock(usage.observedAt) : undefined
  return (
    <div className="usage-block">
      {hasFiveHourRow(usage) && <UsageBar label="5h" title="5-hour session limit" window={usage?.fiveHour} now={now} asOf={asOf} />}
      <UsageBar label="Week" title="Weekly limit (all models)" window={usage?.sevenDay} now={now} asOf={asOf} />
      {usage?.models?.map((model) => (
        <UsageBar key={model.name} label={model.name} title={`Weekly ${model.name} limit`} window={model} now={now} asOf={asOf} />
      ))}
    </div>
  )
}
