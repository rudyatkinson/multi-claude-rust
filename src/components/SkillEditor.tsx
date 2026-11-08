import { useMemo } from 'react'
import { HANDLED_KEYS, SKILL_FIELDS, SKILL_LIMITS, SKILL_SWITCHES, skillKey } from '@shared/skills'
import { fieldBool, fieldText, inspectSkill, nameOf, parseSkill } from '@shared/skillText'
import { mc } from '../api'
import { changeBody, changeField, changeText, deleteSkill, exportSkill, isDirty, loadDisk, revertSkill, rootLabel, saveSkill, setTab, useSkills } from '../skillsStore'
import { attempt, setModal, useStore } from '../store'
import { ConflictBanner, FieldInputs, Issues, Row, SaveBar } from './LibraryParts'

const sizeText = (bytes: number): string => (bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`)

/**
 * The editor of one skill. Its text is the whole SKILL.md: the form reads that text and writes
 * every change straight back into it, and the Source tab shows the same text, so the two can
 * never say different things.
 */
export function SkillEditor() {
  const draft = useSkills((s) => s.draft)!
  const onDisk = useSkills((s) => s.onDisk)
  const tab = useSkills((s) => s.tab)
  const saving = useSkills((s) => s.saving)
  const dirty = useSkills(isDirty)
  const listing = useSkills((s) => s.listing)
  const profiles = useStore((s) => s.profiles)
  const claudeFound = useStore((s) => s.claude.path !== null)

  const { detail, text } = draft
  const parsed = useMemo(() => parseSkill(text), [text])
  const issues = useMemo(() => inspectSkill(parsed, detail.dirName), [parsed, detail.dirName])
  const root = listing?.roots.find((r) => r.id === detail.rootId)
  // what only the whole list can tell: another skill that answers to the same name
  const clashes = (listing?.skills.find((s) => skillKey(s) === skillKey(detail))?.issues ?? []).filter((i) => i.clash)
  const forClaude = [...issues.filter((i) => i.scope === 'claude'), ...clashes]
  const forUpload = issues.filter((i) => i.scope === 'upload')
  const ownName = nameOf(parsed)
  const others = parsed.keys.filter((key) => !HANDLED_KEYS.includes(key))
  const forked = fieldText(parsed, 'context') === 'fork'
  const fields = SKILL_FIELDS.filter((field) => field.key !== 'agent' || forked || fieldText(parsed, 'agent') !== '')
  const readOnly = detail.readOnly !== undefined
  // what works on the folder goes by what is on disk
  const busy = dirty ? 'Save or revert the changes first' : undefined

  return (
    <section className="wf-design sk-editor">
      <div className="wf-toolbar">
        <div className="sk-title">
          <span className="sk-name">/{detail.dirName}</span>
          {ownName && ownName !== detail.dirName && (
            <span className="muted" title="The name in the file. Claude Code answers to both.">
              also /{ownName}
            </span>
          )}
          {detail.linked && (
            <span className="badge" title="This folder is a link to a folder elsewhere">
              link
            </span>
          )}
        </div>
        <button className="btn" disabled={dirty} title={busy ?? 'Give the folder another name'} onClick={() => setModal({ type: 'skillName', mode: 'rename', ref: detail })}>
          Rename…
        </button>
        <button className="btn" disabled={dirty} title={busy} onClick={() => setModal({ type: 'skillName', mode: 'duplicate', ref: detail })}>
          Duplicate…
        </button>
        <button className="btn" disabled={dirty} title={busy ?? 'To a project, or to the personal skills'} onClick={() => setModal({ type: 'skillTransfer', ref: detail })}>
          Copy / Move…
        </button>
        <button className="btn" disabled={dirty} title={busy ?? 'As a .zip, which claude.ai takes as well'} onClick={() => void exportSkill(detail)}>
          Export…
        </button>
        <button
          className="btn"
          disabled={dirty || !claudeFound}
          title={busy ?? 'Open a session that writes or improves this skill. It spends messages.'}
          onClick={() => setModal({ type: 'skillAssist', ref: detail })}
        >
          Ask Claude…
        </button>
        <button className="btn" onClick={() => void deleteSkill(detail)}>
          Delete
        </button>
      </div>
      <div className="sk-where">
        <span>{root ? rootLabel(root, profiles) : ''}</span>
        <span className="sk-path" title={detail.path}>
          {detail.path}
        </span>
        <button className="link-btn" onClick={() => void attempt(mc.invoke('skills:reveal', detail))}>
          Open folder
        </button>
      </div>

      {onDisk && <ConflictBanner onLoad={loadDisk} onOverwrite={() => void saveSkill(true)} />}

      {readOnly ? (
        <p className="note warn sk-readonly">{detail.readOnly} Open the folder to work on it with another program.</p>
      ) : (
        <>
          <SaveBar
            tab={tab}
            onTab={setTab}
            sourceTitle="SKILL.md as it is written"
            dirty={dirty}
            saving={saving}
            saveTitle="Ctrl+S. Running sessions see the new text at once."
            onRevert={() => void revertSkill()}
            onSave={() => void saveSkill()}
          />

          {tab === 'source' ? (
            <textarea className="sk-source" value={text} spellCheck={false} onChange={(e) => changeText(e.target.value)} />
          ) : (
            <div className="wf-editor sk-form">
              <div className="wf-settings">
                {parsed.error ? (
                  <p className="note bad">
                    The fields cannot be shown{parsed.error.line === undefined ? '' : ` (line ${parsed.error.line})`}: {parsed.error.message} Fix it in the Source tab.
                  </p>
                ) : (
                  <>
                    {fields.slice(0, 3).map((field) => (
                      <FieldInputs key={field.key} field={field} parsed={parsed} onChange={changeField} countOver={field.key === 'description' ? SKILL_LIMITS.description : undefined} />
                    ))}
                    <Row label="Who starts it">
                      <label className="check" title="Off: the skill only runs when you type its name; Claude does not even see its description.">
                        <input
                          type="checkbox"
                          checked={!fieldBool(parsed, SKILL_SWITCHES.hideFromModel, false)}
                          onChange={(e) => changeField(SKILL_SWITCHES.hideFromModel, e.target.checked ? null : true)}
                        />
                        Claude may use it by itself
                      </label>
                      <label className="check" title="Off: it is not offered in the / menu and /name does not run it; Claude can still use it.">
                        <input
                          type="checkbox"
                          checked={fieldBool(parsed, SKILL_SWITCHES.userInvocable, true)}
                          onChange={(e) => changeField(SKILL_SWITCHES.userInvocable, e.target.checked ? null : false)}
                        />
                        Show it in the / menu
                      </label>
                    </Row>
                    {fields.slice(3).map((field) => (
                      <FieldInputs key={field.key} field={field} parsed={parsed} onChange={changeField} countOver={field.key === 'description' ? SKILL_LIMITS.description : undefined} />
                    ))}
                    {others.length > 0 && (
                      <p className="note" title="The form leaves these as they are">
                        Also in the file: {others.join(', ')}. Change them in the Source tab.
                      </p>
                    )}
                  </>
                )}
              </div>
              <div className="wf-prompt">
                <div className="wf-prompt-head">
                  <span>Instructions</span>
                  <span className="spacer" />
                  <span title={`Anthropic advises to stay under ${SKILL_LIMITS.lines} lines`}>{parsed.lines} lines</span>
                </div>
                <textarea
                  className="sk-body"
                  value={parsed.body}
                  disabled={parsed.parts.unclosed}
                  spellCheck={false}
                  placeholder="What Claude should do when this skill is used. Markdown."
                  onChange={(e) => changeBody(e.target.value)}
                />
              </div>
            </div>
          )}
        </>
      )}

      <div className="sk-foot">
        <Issues issues={forClaude} />
        {forUpload.length > 0 && (
          <details className="sk-upload">
            <summary>
              For an upload to claude.ai: {forUpload.length} {forUpload.length === 1 ? 'note' : 'notes'}
            </summary>
            <Issues issues={forUpload} />
          </details>
        )}
        {(detail.files.length > 0 || detail.moreFiles) && (
          <div className="sk-files">
            <span className="sk-files-title">Files next to SKILL.md</span>
            {detail.files.map((file) => (
              <button key={file.path} className="sk-file" title="Text files open in their program; anything else is shown in its folder" onClick={() => void attempt(mc.invoke('skills:openFile', detail, file.path))}>
                {file.path}
                <span className="muted">{sizeText(file.size)}</span>
              </button>
            ))}
            {detail.moreFiles && <span className="muted">and more; open the folder to see them all</span>}
          </div>
        )}
      </div>
    </section>
  )
}
