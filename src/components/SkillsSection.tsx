import { useEffect, useMemo } from 'react'
import { foldText } from '@shared/conversations'
import { skillKey, type SkillSummary } from '@shared/skills'
import { addSkillFolder, checkDisk, isDirty, openKey, openSkill, pickImport, refreshSkills, removeSkillFolder, rootLabel, setQuery, useSkills } from '../skillsStore'
import { setModal, useStore } from '../store'
import { worstOf } from './LibraryParts'
import { SkillEditor } from './SkillEditor'

/** Who may start a skill, when that is not everyone. */
function invocationMark(skill: SkillSummary): { text: string; hint: string } | null {
  if (!skill.modelInvocable && !skill.userInvocable) return { text: 'off', hint: 'Neither Claude nor the / menu starts it' }
  if (!skill.modelInvocable) return { text: 'manual', hint: 'Only you start it, with /name' }
  if (!skill.userInvocable) return { text: 'Claude only', hint: 'Not in the / menu; Claude uses it by itself' }
  return null
}

/**
 * The skill manager: the folders that hold skills and their skills on the left, the editor of
 * the one that is chosen on the right.
 */
export function SkillsSection() {
  const listing = useSkills((s) => s.listing)
  const query = useSkills((s) => s.query)
  const open = useSkills(openKey)
  const dirty = useSkills(isDirty)
  const hasDraft = useSkills((s) => s.draft !== null)
  const profiles = useStore((s) => s.profiles)

  useEffect(() => {
    void refreshSkills()
    void checkDisk()
  }, [])

  const words = useMemo(() => foldText(query).split(/\s+/).filter(Boolean), [query])
  const matches = (skill: SkillSummary): boolean => {
    const hay = foldText(`${skill.dirName} ${skill.name} ${skill.description}`)
    return words.every((word) => hay.includes(word))
  }
  const firstRoot = listing?.roots[0]?.id

  return (
    <div className="wf-body">
      <nav className="wf-list sk-list">
        <input className="sk-search" autoFocus value={query} placeholder="Search skills" spellCheck={false} onChange={(e) => setQuery(e.target.value)} />
        {!listing && <p className="note">Reading the skills…</p>}
        {listing?.roots.map((skillRoot) => {
          const skills = listing.skills.filter((s) => s.rootId === skillRoot.id)
          const shown = skills.filter(matches)
          // while searching, folders without a match are out of the way
          if (words.length > 0 && shown.length === 0) return null
          return (
            <section key={skillRoot.id} className="sk-root">
              <div className="sk-root-head" title={skillRoot.dir}>
                <span className="sk-root-name">{rootLabel(skillRoot, profiles)}</span>
                <button className="icon-btn" title="New skill in this folder" onClick={() => setModal({ type: 'skillNew', rootId: skillRoot.id })}>
                  +
                </button>
                {skillRoot.pinned && (
                  <button className="icon-btn" title="Take this folder off the list (its skills stay)" onClick={() => void removeSkillFolder(skillRoot.id)}>
                    ×
                  </button>
                )}
              </div>
              {shown.map((skill) => {
                const mark = invocationMark(skill)
                const worst = worstOf(skill.issues)
                const key = skillKey(skill)
                return (
                  <button key={key} className={`wf-item${key === open ? ' selected' : ''}`} onClick={() => void openSkill(skill)}>
                    <span className="sk-item-head">
                      <span className="wf-item-name">{skill.dirName}</span>
                      {key === open && dirty && <span className="sk-dot" title="Unsaved changes" />}
                      {skill.linked && (
                        <span className="sk-mark" title="A link to a folder elsewhere">
                          link
                        </span>
                      )}
                      {mark && (
                        <span className="sk-mark" title={mark.hint}>
                          {mark.text}
                        </span>
                      )}
                      {worst && (
                        <span className={`sk-flag ${worst}`} title={skill.issues.map((i) => i.message).join('\n')}>
                          !
                        </span>
                      )}
                    </span>
                    <span className="wf-item-steps">{skill.description || 'No description'}</span>
                  </button>
                )
              })}
              {skills.length === 0 && <div className="sk-none">{skillRoot.exists ? 'No skills yet' : 'No skills folder yet; the first skill makes it'}</div>}
            </section>
          )
        })}
        {listing && words.length > 0 && !listing.skills.some(matches) && <p className="note">Nothing matches.</p>}
        <div className="wf-list-actions">
          <button className="btn" disabled={!firstRoot} onClick={() => setModal({ type: 'skillNew' })}>
            + New skill
          </button>
          <button className="btn" disabled={!firstRoot} title="Bring in a skill from a .zip or .skill file" onClick={() => void pickImport()}>
            Import…
          </button>
          <button className="btn" title="Show the skills of a project folder that is not in the list" onClick={() => void addSkillFolder()}>
            Add project folder…
          </button>
        </div>
      </nav>

      {hasDraft ? (
        <SkillEditor />
      ) : (
        <section className="wf-design wf-empty">
          <div className="empty-title">{listing && listing.skills.length === 0 ? 'No skills yet' : 'Choose a skill'}</div>
          <div className="muted">
            A skill is a folder with a SKILL.md: a description that tells Claude when to use it, and instructions it follows then. Personal skills work in every
            project; project skills belong to one folder and travel with it.
          </div>
          <button className="btn primary" disabled={!firstRoot} onClick={() => setModal({ type: 'skillNew' })}>
            New skill
          </button>
        </section>
      )}
    </div>
  )
}
