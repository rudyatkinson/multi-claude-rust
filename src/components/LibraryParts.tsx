import { useState, type ReactNode } from 'react'
import { fieldBool, fieldText, splitItems, type FieldInput, type FormField, type ParsedDoc } from '@shared/frontmatter'
import type { SkillRoot } from '@shared/skills'
import { rootLabel } from '../library'
import { useStore } from '../store'

/** What the sections of the Library are built from. */

export function Row({ label, hint, note, children }: { label: string; hint?: string; note?: ReactNode; children: ReactNode }) {
  return (
    <div className="field" title={hint}>
      <span>
        {label}
        {note}
      </span>
      {children}
    </div>
  )
}

/**
 * An input whose text is not the value itself: while it has the keyboard it shows what was
 * typed, so that a comma or a digit does not vanish because the file reads differently.
 */
function TypedInput({ value, placeholder, className, clean, onType }: { value: string; placeholder?: string; className?: string; clean?(typed: string): string; onType(typed: string): void }) {
  const [typed, setTyped] = useState<string | null>(null)
  return (
    <input
      className={className}
      value={typed ?? value}
      placeholder={placeholder}
      spellCheck={false}
      onFocus={() => setTyped(value)}
      onBlur={() => setTyped(null)}
      onChange={(e) => {
        const next = clean ? clean(e.target.value) : e.target.value
        setTyped(next)
        onType(next)
      }}
    />
  )
}

/** One field of a frontmatter. What is typed goes into the text of the file at once. */
export function FieldInputs({ field, parsed, onChange, countOver }: { field: FormField; parsed: ParsedDoc; onChange(key: string, value: FieldInput): void; countOver?: number }) {
  const value = fieldText(parsed, field.key, field.kind === 'list' || field.kind === 'names')
  if (field.kind === 'switch') {
    return (
      <div className="field" title={field.hint}>
        <label className="check">
          <input type="checkbox" checked={fieldBool(parsed, field.key, false)} onChange={(e) => onChange(field.key, e.target.checked ? true : null)} />
          {field.label}
        </label>
      </div>
    )
  }
  if (field.kind === 'choice') {
    const choices = field.choices ?? []
    return (
      <Row label={field.label} hint={field.hint}>
        <div className="segmented wrap">
          {choices.map((choice) => (
            <button key={choice.value} type="button" className={choice.value === value ? 'selected' : ''} onClick={() => onChange(field.key, choice.value)}>
              {choice.label}
            </button>
          ))}
          {!choices.some((c) => c.value === value) && (
            <button type="button" className="selected" title="This is what the file says. It is none of the values the form offers.">
              {value}
            </button>
          )}
        </div>
      </Row>
    )
  }
  if (field.kind === 'long') {
    const counted = countOver !== undefined
    return (
      <Row label={field.label} hint={field.hint} note={counted && value.length > 0 ? <em className={value.length > countOver ? 'over' : ''}> {value.length} characters</em> : undefined}>
        <textarea className="sk-long" value={value} rows={counted || field.key === 'description' ? 4 : 2} placeholder={field.placeholder} spellCheck={false} onChange={(e) => onChange(field.key, e.target.value)} />
      </Row>
    )
  }
  if (field.kind === 'number') {
    return (
      <Row label={field.label} hint={field.hint}>
        <TypedInput className="lib-number" value={value} placeholder={field.placeholder} clean={(typed) => typed.replace(/\D/g, '').slice(0, 6)} onType={(typed) => onChange(field.key, typed === '' ? null : Number(typed))} />
      </Row>
    )
  }
  if (field.kind === 'names') {
    return (
      <Row label={field.label} hint={field.hint}>
        <TypedInput value={value} placeholder={field.placeholder} onType={(typed) => onChange(field.key, splitItems(typed))} />
      </Row>
    )
  }
  return (
    <Row label={field.label} hint={field.hint}>
      <input value={value} placeholder={field.placeholder} spellCheck={false} onChange={(e) => onChange(field.key, e.target.value)} />
    </Row>
  )
}

export function Issues({ issues }: { issues: readonly { level: 'error' | 'warning' | 'info'; message: string }[] }) {
  return (
    <>
      {issues.map((issue) => (
        <p key={issue.message} className={`note ${issue.level === 'error' ? 'bad' : issue.level === 'warning' ? 'warn' : ''}`}>
          {issue.message}
        </p>
      ))}
    </>
  )
}

/** The worst of a list of issues, for the mark in a list; notes do not count. */
export function worstOf(issues: readonly { level: 'error' | 'warning' | 'info' }[]): 'error' | 'warning' | null {
  return issues.some((i) => i.level === 'error') ? 'error' : issues.some((i) => i.level === 'warning') ? 'warning' : null
}

/** Shown when the file in the editor changed on disk while it has changes of its own. */
export function ConflictBanner({ onLoad, onOverwrite }: { onLoad(): void; onOverwrite(): void }) {
  return (
    <div className="sk-banner">
      <span>This file was changed on disk while you were editing, perhaps by a Claude session.</span>
      <span className="spacer" />
      <button className="btn" onClick={onLoad}>
        Load the version on disk
      </button>
      <button className="btn" onClick={onOverwrite}>
        Keep mine and overwrite
      </button>
    </div>
  )
}

/** The line above an editor: its tabs when it has two, whether it is saved, Revert and Save. */
export function SaveBar(props: {
  tab?: 'form' | 'source'
  onTab?(tab: 'form' | 'source'): void
  sourceTitle?: string
  dirty: boolean
  saving: boolean
  saveTitle: string
  onRevert(): void
  onSave(): void
  children?: ReactNode
}) {
  const { tab, onTab, dirty, saving } = props
  return (
    <div className="sk-tabs">
      {tab && onTab && (
        <div className="segmented">
          <button type="button" className={tab === 'form' ? 'selected' : ''} onClick={() => onTab('form')}>
            Form
          </button>
          <button type="button" className={tab === 'source' ? 'selected' : ''} title={props.sourceTitle} onClick={() => onTab('source')}>
            Source
          </button>
        </div>
      )}
      {props.children}
      <span className="spacer" />
      <span className={`sk-state${dirty ? ' dirty' : ''}`}>{saving ? 'Saving…' : dirty ? 'Unsaved changes' : 'Saved'}</span>
      <button className="btn" disabled={!dirty || saving} onClick={props.onRevert}>
        Revert
      </button>
      <button className="btn primary" disabled={!dirty || saving} title={props.saveTitle} onClick={props.onSave}>
        Save
      </button>
    </div>
  )
}

/** Lower case, with a hyphen for what a name cannot hold: typing a title gives a name that passes. */
export const asName = (typed: string, max: number): string =>
  typed
    .toLowerCase()
    .replace(/[\s_]+/g, '-')
    .replace(/[^a-z0-9-]/g, '')
    .slice(0, max)

export function NameField({
  value,
  onChange,
  problem,
  max,
  label = 'Name',
  note,
  placeholder,
  focus = true
}: {
  value: string
  onChange(name: string): void
  problem: string | null
  max: number
  label?: string
  note: string
  placeholder: string
  /** Takes the keyboard when it appears. */
  focus?: boolean
}) {
  return (
    <label className="field">
      <span>
        {label} <em>({note})</em>
      </span>
      <input autoFocus={focus} value={value} spellCheck={false} placeholder={placeholder} onChange={(e) => onChange(asName(e.target.value, max))} />
      {problem && <span className="note bad">{problem}</span>}
    </label>
  )
}

/** Which folder something goes into: an account's own, or a project's. */
export function RootChoice({ roots, value, onChange, onAddFolder }: { roots: readonly SkillRoot[]; value: string | null; onChange(rootId: string): void; onAddFolder(): Promise<string | null> }) {
  const profiles = useStore((s) => s.profiles)
  return (
    <div className="field">
      <span>Where</span>
      <div className="sk-roots">
        {roots.map((root) => (
          <button key={root.id} type="button" className={`sk-root-choice${root.id === value ? ' selected' : ''}`} title={root.dir} onClick={() => onChange(root.id)}>
            <span>{rootLabel(root, profiles)}</span>
            <span className="muted">{root.kind === 'personal' ? 'in every project' : (root.folder ?? root.dir)}</span>
          </button>
        ))}
        <button
          type="button"
          className="sk-root-choice"
          onClick={() =>
            void onAddFolder().then((rootId) => {
              if (rootId) onChange(rootId)
            })
          }
        >
          <span>Another project folder…</span>
        </button>
      </div>
    </div>
  )
}
