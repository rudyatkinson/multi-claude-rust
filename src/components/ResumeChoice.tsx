import type { ResumeMode } from '@shared/types'

const OPTIONS: { value: ResumeMode | undefined; label: string; hint: string }[] = [
  { value: undefined, label: 'New', hint: 'Start a new conversation' },
  { value: 'last', label: 'Continue last', hint: "Continue the folder's most recent conversation" },
  { value: 'pick', label: 'From history', hint: "Choose from the folder's earlier conversations in Claude Code's own list" }
]

/** How a session starts: a new conversation, the most recent one, or one picked from history. */
export function ResumeChoice({ value, onChange }: { value: ResumeMode | undefined; onChange(value: ResumeMode | undefined): void }) {
  return (
    <div className="segmented">
      {OPTIONS.map((option) => (
        <button
          key={option.label}
          type="button"
          className={option.value === value ? 'selected' : ''}
          title={option.hint}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  )
}
