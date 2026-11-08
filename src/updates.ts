import { create } from 'zustand'
import { installQuestion, type UpdateInfo, type UpdateProgress } from '@shared/updates'
import { isActiveRun } from '@shared/workflows'
import { errorMessage, mc } from './api'
import { useUnsaved } from './library'
import { ask, toast, useStore } from './store'

/**
 * A new version of the app: the bar above the tabs, "Check now" in Settings, the mark next to the
 * version in the sidebar footer. The backend looks at start and once a day and announces what it
 * finds (`updater:available`); installing ends the app and starts the new version.
 */
interface UpdateState {
  /** The newest version found in this run of the app; null while none is known. */
  info: UpdateInfo | null
  /** "Later" hid the bar; the version in the sidebar footer brings it back. */
  hidden: boolean
  /** "Check now" is on its way. */
  checking: boolean
  /** From "Install and restart" until the app ends, or until that failed. */
  installing: boolean
  progress: UpdateProgress | null
}

export const useUpdate = create<UpdateState>(() => ({ info: null, hidden: false, checking: false, installing: false, progress: null }))

const set = useUpdate.setState
const get = useUpdate.getState

function show(info: UpdateInfo): void {
  set({ info, hidden: false })
}

let subscribed = false

/** Listens to the updater's events. Called before `init` waits for the listeners to be registered. */
export function listenForUpdates(): void {
  if (subscribed) return
  subscribed = true
  mc.on('updater:available', ({ info }) => show(info))
  mc.on('updater:progress', (progress) => {
    if (get().installing) set({ progress })
  })
}

/** The mark in the sidebar footer: shows the bar again after "Later". */
export function showUpdateBar(): void {
  if (get().info) set({ hidden: false })
}

/** "Later": hides the bar, and the version is not announced again until the app starts anew. */
export function later(): void {
  const { info } = get()
  if (!info) return
  set({ hidden: true })
  mc.invoke('updater:dismiss', info.version).catch((err: unknown) => console.warn('updater:dismiss failed:', errorMessage(err)))
}

/** "Check now": shows the bar when there is a newer version, else says that this one is the newest. */
export async function checkNow(): Promise<void> {
  if (get().checking || get().installing) return
  set({ checking: true })
  try {
    const info = await mc.invoke('updater:check')
    if (info) show(info)
    else {
      set({ info: null, hidden: false })
      toast('info', `You are up to date (${useStore.getState().appVersion ?? 'this version'}).`)
    }
  } catch (err) {
    toast('error', errorMessage(err))
  } finally {
    set({ checking: false })
  }
}

/** "Install and restart", asking first when that would end something that is going on. */
export async function installUpdate(): Promise<void> {
  const { info, installing } = get()
  if (!info || installing) return
  const s = useStore.getState()
  const question = installQuestion({
    runningSessions: s.sessions.filter((session) => session.status === 'running').length,
    activeRuns: s.runs.filter(isActiveRun).length,
    unsavedDrafts: Object.keys(useUnsaved.getState().items).length,
    restoreTabs: s.settings.restoreTabs !== false
  })
  if (question && !(await ask(question))) return
  set({ installing: true, hidden: false, progress: null })
  try {
    // on success this process ends before an answer comes back
    await mc.invoke('updater:install')
  } catch (err) {
    toast('error', errorMessage(err))
  }
  set({ installing: false, progress: null })
}
