import { useEffect } from 'react'
import { AgentNameDialog, AgentTransferDialog, NewAgentDialog } from './components/AgentDialogs'
import { AddAccountWizard, ConfirmDialog, EditProfileDialog, ReplaceTokenDialog, SettingsDialog, Toasts } from './components/Dialogs'
import { HistoryDialog } from './components/HistoryDialog'
import { NewRuleDialog } from './components/InstructionsSection'
import { LibraryPanel } from './components/LibraryPanel'
import { McpDialog } from './components/McpDialog'
import { AdoptMemoryDialog, NewMemoryDialog } from './components/MemorySection'
import { NewSessionPopover } from './components/NewSessionPopover'
import { OpenFolderDialog } from './components/OpenFolderDialog'
import { RunBars } from './components/RunBar'
import { RunDialog, StepOutputDialog } from './components/RunDialog'
import { Sidebar } from './components/Sidebar'
import { NewSkillDialog, SkillAssistDialog, SkillImportDialog, SkillNameDialog, SkillTransferDialog } from './components/SkillDialogs'
import { TabMenu } from './components/TabMenu'
import { TransferDialog } from './components/TransferDialog'
import { WorkflowsPanel } from './components/WorkflowsPanel'
import { Workspace } from './components/Workspace'
import { init, useStore } from './store'

export function App() {
  const modal = useStore((s) => s.modal)
  const newSession = useStore((s) => s.newSession)
  const panel = useStore((s) => s.panel)

  useEffect(() => {
    void init()
  }, [])

  return (
    <div className="app">
      <Sidebar />
      <main className="main">
        <RunBars />
        <div className="stage">
          {/* a panel lies over the terminals, which stay as they are underneath */}
          <Workspace />
          {panel === 'workflows' && <WorkflowsPanel />}
          {panel === 'library' && <LibraryPanel />}
        </div>
      </main>
      {modal?.type === 'addAccount' && <AddAccountWizard />}
      {modal?.type === 'editProfile' && <EditProfileDialog profileId={modal.profileId} />}
      {modal?.type === 'settings' && <SettingsDialog />}
      {modal?.type === 'replaceToken' && <ReplaceTokenDialog profileId={modal.profileId} />}
      {modal?.type === 'openFolder' && <OpenFolderDialog key={modal.cwd} cwd={modal.cwd} place={modal.place} />}
      {modal?.type === 'mcp' && <McpDialog profileId={modal.profileId} />}
      {modal?.type === 'transfer' && <TransferDialog sessionId={modal.sessionId} />}
      {modal?.type === 'history' && <HistoryDialog />}
      {modal?.type === 'runWorkflow' && <RunDialog key={modal.workflowId} workflowId={modal.workflowId} />}
      {modal?.type === 'stepOutput' && <StepOutputDialog runId={modal.runId} index={modal.index} />}
      {modal?.type === 'skillNew' && <NewSkillDialog rootId={modal.rootId} />}
      {modal?.type === 'skillName' && <SkillNameDialog key={`${modal.mode}:${modal.ref.dirName}`} mode={modal.mode} target={modal.ref} />}
      {modal?.type === 'skillTransfer' && <SkillTransferDialog target={modal.ref} />}
      {modal?.type === 'skillImport' && <SkillImportDialog key={modal.preview.token} preview={modal.preview} />}
      {modal?.type === 'skillAssist' && <SkillAssistDialog key={modal.ref.dirName} target={modal.ref} />}
      {modal?.type === 'agentNew' && <NewAgentDialog rootId={modal.rootId} />}
      {modal?.type === 'agentName' && <AgentNameDialog key={`${modal.mode}:${modal.ref.file}`} mode={modal.mode} target={modal.ref} />}
      {modal?.type === 'agentTransfer' && <AgentTransferDialog key={modal.ref.file} target={modal.ref} />}
      {modal?.type === 'memoryNew' && <NewMemoryDialog key={modal.folderId} folderId={modal.folderId} />}
      {modal?.type === 'memoryAdopt' && <AdoptMemoryDialog key={modal.folderId} folderId={modal.folderId} plan={modal.plan} />}
      {modal?.type === 'instructionRule' && <NewRuleDialog key={modal.scopeId} scopeId={modal.scopeId} />}
      {newSession && <NewSessionPopover key={newSession.groupId} groupId={newSession.groupId} />}
      <TabMenu />
      <ConfirmDialog />
      <Toasts />
    </div>
  )
}
