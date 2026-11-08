import { fieldBool, fieldText, frontmatterBlock, nameOf, parseDoc, setField, type ParsedDoc } from './frontmatter'
import { SKILL_LIMITS, SKILL_SWITCHES, UPLOAD_KEYS, newNameProblem, type SkillIssue } from './skills'

/**
 * The text of a SKILL.md: a YAML frontmatter between two `---` lines, then Markdown. Reading and
 * changing such a text is `frontmatter.ts`; what is here knows what a skill is.
 */

export { fieldBool, fieldText, firstBodyLine, nameOf, setBody, setField, type FieldInput, type FieldValue } from './frontmatter'
export { parseDoc as parseSkill, splitDoc as splitSkill, type DocParts as SkillParts, type ParsedDoc as ParsedSkill } from './frontmatter'

/** The text of a skill that was just made. */
export function newSkillText(name: string, description: string): string {
  const title = name
    .split('-')
    .filter(Boolean)
    .map((word) => word[0].toUpperCase() + word.slice(1))
    .join(' ')
  const head = frontmatterBlock({ name, description: description.trim() || 'What this skill does, and when Claude should use it.' })
  return `${head}\n# ${title || name}\n\nWrite here what Claude should do when this skill is used.\n`
}

/**
 * The text of a skill whose folder gets another name. A name in the file follows the folder when
 * it was the folder's name (`always`: whenever the file has one, as for a copy that must not
 * answer to the name of its original).
 */
export function renameInText(text: string, oldDirName: string, newDirName: string, always = false): string {
  const parsed = parseDoc(text)
  const name = nameOf(parsed)
  if (parsed.error || name === null || (!always && name !== oldDirName) || name === newDirName) return text
  const result = setField(text, 'name', newDirName)
  return 'text' in result ? result.text : text
}

/** Who may start the skill, as its two switches say. */
export function invocationOf(parsed: ParsedDoc): { modelInvocable: boolean; userInvocable: boolean } {
  return {
    modelInvocable: !fieldBool(parsed, SKILL_SWITCHES.hideFromModel, false),
    userInvocable: fieldBool(parsed, SKILL_SWITCHES.userInvocable, true)
  }
}

/**
 * What is wrong with a skill, or worth knowing. `claude` issues matter to Claude Code; `upload`
 * issues only when the skill is to be uploaded to claude.ai, whose rules are stricter.
 */
export function inspectSkill(parsed: ParsedDoc, dirName: string): SkillIssue[] {
  const issues: SkillIssue[] = []
  const add = (level: SkillIssue['level'], scope: SkillIssue['scope'], message: string): void => void issues.push({ level, scope, message })

  if (parsed.error) {
    const where = parsed.error.line === undefined ? '' : ` (line ${parsed.error.line})`
    const said = /[.!?]$/.test(parsed.error.message) ? parsed.error.message : `${parsed.error.message}.`
    add('error', 'claude', `The frontmatter cannot be read${where}: ${said} Claude Code then uses the skill without any of its fields.`)
    return issues
  }

  const description = fieldText(parsed, 'description').trim()
  const listed = description.length + fieldText(parsed, 'when_to_use').trim().length
  if (!description) add('warning', 'claude', 'No description. Claude goes by the first line of the text instead, which rarely says when to use the skill.')
  else if (listed > SKILL_LIMITS.listing) add('warning', 'claude', `Description and "when to use" have ${listed} characters together; Claude sees the first ${SKILL_LIMITS.listing}.`)
  if (parsed.lines > SKILL_LIMITS.lines) add('info', 'claude', `SKILL.md has ${parsed.lines} lines. Anthropic advises to stay under ${SKILL_LIMITS.lines} and to move detail into files next to it.`)

  const name = nameOf(parsed)
  if (name === null) add('warning', 'upload', 'No name in the file. Claude Code uses the folder name; claude.ai wants the name written down.')
  else {
    const problem = newNameProblem(name)
    if (problem) add('warning', 'upload', `The name "${name}" does not pass on claude.ai: ${problem}`)
    else if (name !== dirName) add('warning', 'upload', `The name "${name}" differs from the folder "${dirName}". Claude Code answers to both; claude.ai wants them equal.`)
  }
  if (description.length > SKILL_LIMITS.description) add('warning', 'upload', `The description has ${description.length} characters; claude.ai takes at most ${SKILL_LIMITS.description}.`)
  if (/[<>]/.test(description)) add('warning', 'upload', 'The description contains < or >, which claude.ai refuses.')
  if (fieldText(parsed, 'compatibility').length > SKILL_LIMITS.compatibility) add('warning', 'upload', `"compatibility" is longer than ${SKILL_LIMITS.compatibility} characters.`)
  const extra = parsed.keys.filter((key) => !UPLOAD_KEYS.includes(key))
  if (extra.length > 0) add('warning', 'upload', `claude.ai does not accept ${extra.length === 1 ? 'this field' : 'these fields'}: ${extra.join(', ')}. Claude Code uses them.`)
  return issues
}
