import { useEffect, useMemo } from 'react'
import { AGENT_FIELDS, AGENT_HANDLED_KEYS, agentKey, agentStem, inspectAgent, type AgentSummary } from '@shared/agents'
import { foldText } from '@shared/conversations'
import { nameOf, parseDoc } from '@shared/frontmatter'
import {
  addAgentFolder,
  agentLabel,
  changeBody,
  changeField,
  changeText,
  checkDisk,
  deleteAgent,
  isDirty,
  loadDisk,
  openAgent,
  openKey,
  refreshAgents,
  removeAgentFolder,
  revertAgent,
  saveAgent,
  setQuery,
  setTab,
  useAgents
} from '../agentsStore'
import { mc } from '../api'
import { rootLabel } from '../library'
import { attempt, setModal, useStore } from '../store'
import { ConflictBanner, FieldInputs, Issues, SaveBar, worstOf } from './LibraryParts'

/**
 * The agent manager: the folders that hold subagents and their agents on the left, the editor of
 * the one that is chosen on the right.
 */
export function AgentsSection() {
  const listing = useAgents((s) => s.listing)
  const query = useAgents((s) => s.query)
  const open = useAgents(openKey)
  const dirty = useAgents(isDirty)
  const hasDraft = useAgents((s) => s.draft !== null)
  const profiles = useStore((s) => s.profiles)

  useEffect(() => {
    void refreshAgents().then(checkDisk)
  }, [])

  const words = useMemo(() => foldText(query).split(/\s+/).filter(Boolean), [query])
  const matches = (agent: AgentSummary): boolean => {
    const hay = foldText(`${agent.file} ${agent.name} ${agent.description}`)
    return words.every((word) => hay.includes(word))
  }
  const firstRoot = listing?.roots[0]?.id

  return (
    <div className="wf-body">
      <nav className="wf-list sk-list">
        <input className="sk-search" autoFocus value={query} placeholder="Search agents" spellCheck={false} onChange={(e) => setQuery(e.target.value)} />
        {!listing && <p className="note">Reading the agents…</p>}
        {listing?.roots.map((root) => {
          const agents = listing.agents.filter((a) => a.rootId === root.id)
          const shown = agents.filter(matches)
          // while searching, folders without a match are out of the way
          if (words.length > 0 && shown.length === 0) return null
          return (
            <section key={root.id} className="sk-root">
              <div className="sk-root-head" title={root.dir}>
                <span className="sk-root-name">{rootLabel(root, profiles)}</span>
                <button className="icon-btn" title="New agent in this folder" onClick={() => setModal({ type: 'agentNew', rootId: root.id })}>
                  +
                </button>
                {root.pinned && (
                  <button className="icon-btn" title="Take this folder off the list (its agents stay)" onClick={() => void removeAgentFolder(root.id)}>
                    ×
                  </button>
                )}
              </div>
              {shown.map((agent) => {
                const worst = worstOf(agent.issues)
                const key = agentKey(agent)
                return (
                  <button key={key} className={`wf-item${key === open ? ' selected' : ''}`} onClick={() => void openAgent(agent)}>
                    <span className="sk-item-head">
                      <span className="wf-item-name">{agentLabel(agent)}</span>
                      {key === open && dirty && <span className="sk-dot" title="Unsaved changes" />}
                      {agent.model && (
                        <span className="sk-mark" title="The model this agent runs on">
                          {agent.model}
                        </span>
                      )}
                      {agent.effort && (
                        <span className="sk-mark" title="The effort this agent works with">
                          {agent.effort}
                        </span>
                      )}
                      {!agent.loads && (
                        <span className="sk-mark" title="Claude Code does not load this file as an agent">
                          not loaded
                        </span>
                      )}
                      {worst && (
                        <span className={`sk-flag ${worst}`} title={agent.issues.map((i) => i.message).join('\n')}>
                          !
                        </span>
                      )}
                    </span>
                    <span className="wf-item-steps">{agent.description || 'No description'}</span>
                  </button>
                )
              })}
              {agents.length === 0 && <div className="sk-none">{root.exists ? 'No agents yet' : 'No agents folder yet; the first agent makes it'}</div>}
            </section>
          )
        })}
        {listing && words.length > 0 && !listing.agents.some(matches) && <p className="note">Nothing matches.</p>}
        <div className="wf-list-actions">
          <button className="btn" disabled={!firstRoot} onClick={() => setModal({ type: 'agentNew' })}>
            + New agent
          </button>
          <button className="btn" title="Show the agents of a project folder that is not in the list" onClick={() => void addAgentFolder()}>
            Add project folder…
          </button>
        </div>
      </nav>

      {hasDraft ? (
        <AgentEditor />
      ) : (
        <section className="wf-design wf-empty">
          <div className="empty-title">{listing && listing.agents.length === 0 ? 'No agents yet' : 'Choose an agent'}</div>
          <div className="muted">
            A subagent is one Markdown file: a description that tells Claude when to hand it a task, the tools and the model it works with, and its instructions.
            Personal agents are there in every project; a project's agents belong to its folder and win over personal ones of the same name.
          </div>
          <button className="btn primary" disabled={!firstRoot} onClick={() => setModal({ type: 'agentNew' })}>
            New agent
          </button>
        </section>
      )}
    </div>
  )
}

/**
 * The editor of one agent. Its text is the whole file: the form reads that text and writes every
 * change straight back into it, and the Source tab shows the same text.
 */
function AgentEditor() {
  const draft = useAgents((s) => s.draft)!
  const onDisk = useAgents((s) => s.onDisk)
  const tab = useAgents((s) => s.tab)
  const saving = useAgents((s) => s.saving)
  const dirty = useAgents(isDirty)
  const listing = useAgents((s) => s.listing)
  const profiles = useStore((s) => s.profiles)

  const { detail, text } = draft
  const parsed = useMemo(() => parseDoc(text), [text])
  const issues = useMemo(() => inspectAgent(parsed), [parsed])
  const root = listing?.roots.find((r) => r.id === detail.rootId)
  // what only the whole list can tell: another agent of the same name
  const clashes = (listing?.agents.find((a) => agentKey(a) === agentKey(detail))?.issues ?? []).filter((i) => i.clash)
  const name = nameOf(parsed)
  const stem = agentStem(detail.file)
  const others = parsed.keys.filter((key) => !AGENT_HANDLED_KEYS.includes(key))
  const readOnly = detail.readOnly !== undefined
  // what works on the file goes by what is on disk
  const busy = dirty ? 'Save or revert the changes first' : undefined

  return (
    <section className="wf-design sk-editor">
      <div className="wf-toolbar">
        <div className="sk-title">
          <span className="sk-name">{name ?? stem}</span>
          {name !== null && name !== stem && (
            <span className="muted" title="The file has a name of its own. Claude Code goes by the name inside it.">
              in {detail.file}
            </span>
          )}
        </div>
        <button className="btn" disabled={dirty || readOnly} title={busy ?? 'Give the agent another name'} onClick={() => setModal({ type: 'agentName', mode: 'rename', ref: detail })}>
          Rename…
        </button>
        <button className="btn" disabled={dirty || readOnly} title={busy} onClick={() => setModal({ type: 'agentName', mode: 'duplicate', ref: detail })}>
          Duplicate…
        </button>
        <button className="btn" disabled={dirty} title={busy ?? 'To a project, or to the personal agents'} onClick={() => setModal({ type: 'agentTransfer', ref: detail })}>
          Copy / Move…
        </button>
        <button className="btn" onClick={() => void deleteAgent(detail)}>
          Delete
        </button>
      </div>
      <div className="sk-where">
        <span>{root ? rootLabel(root, profiles) : ''}</span>
        <span className="sk-path" title={detail.path}>
          {detail.path}
        </span>
        <button className="link-btn" onClick={() => void attempt(mc.invoke('agents:reveal', { rootId: detail.rootId, file: detail.file }))}>
          Show file
        </button>
      </div>

      {onDisk && <ConflictBanner onLoad={loadDisk} onOverwrite={() => void saveAgent(true)} />}

      {readOnly ? (
        <p className="note warn sk-readonly">{detail.readOnly} Show the file to work on it with another program.</p>
      ) : (
        <>
          <SaveBar
            tab={tab}
            onTab={setTab}
            sourceTitle="The file as it is written"
            dirty={dirty}
            saving={saving}
            saveTitle="Ctrl+S. Claude Code uses the new text the next time it starts this agent."
            onRevert={() => void revertAgent()}
            onSave={() => void saveAgent()}
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
                    {AGENT_FIELDS.map((field) => (
                      <FieldInputs key={`${agentKey(detail)}:${field.key}`} field={field} parsed={parsed} onChange={changeField} />
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
                  <span>System prompt</span>
                  <span className="spacer" />
                  <span>{parsed.lines} lines</span>
                </div>
                <textarea
                  className="sk-body"
                  value={parsed.body}
                  disabled={parsed.parts.unclosed}
                  spellCheck={false}
                  placeholder="Who the agent is, how it works, and what it hands back. This text is all it knows about its job."
                  onChange={(e) => changeBody(e.target.value)}
                />
              </div>
            </div>
          )}
        </>
      )}

      <div className="sk-foot">
        <Issues issues={[...issues, ...clashes]} />
      </div>
    </section>
  )
}
