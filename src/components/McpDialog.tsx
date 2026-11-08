import { useCallback, useEffect, useState } from 'react'
import type { McpServerRow } from '@shared/types'
import { errorMessage, mc } from '../api'
import { mainProfileName, setModal, toast, useStore } from '../store'
import { Dialog } from './Dialogs'

const STATUS_LABELS: Record<McpServerRow['status'], string> = {
  missing: 'Not in this account',
  different: 'Different in this account',
  same: 'Already the same'
}

/** Copies user-level MCP servers from the main account into another one. */
export function McpDialog({ profileId }: { profileId: string }) {
  const profile = useStore((s) => s.profiles.find((p) => p.id === profileId))
  const [rows, setRows] = useState<McpServerRow[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const main = mainProfileName()

  const load = useCallback(async (): Promise<void> => {
    try {
      const list = await mc.invoke('mcp:list', profileId)
      setRows(list)
      // everything a copy would change is ticked to begin with
      setChosen(new Set(list.filter((row) => row.status !== 'same').map((row) => row.name)))
      setError(null)
    } catch (err) {
      setError(errorMessage(err))
    }
  }, [profileId])

  useEffect(() => void load(), [load])

  const close = (): void => setModal(null)

  const toggle = (name: string): void => {
    const next = new Set(chosen)
    if (!next.delete(name)) next.add(name)
    setChosen(next)
  }

  const copy = async (): Promise<void> => {
    setBusy(true)
    try {
      const results = await mc.invoke('mcp:copy', profileId, [...chosen])
      const copied = results.filter((r) => r.ok).map((r) => r.name)
      if (copied.length > 0) toast('info', `Copied to "${profile?.name}": ${copied.join(', ')}. Sessions that are open see them after a restart.`)
      for (const failed of results.filter((r) => !r.ok)) toast('error', `${failed.name}: ${failed.error ?? 'could not be copied'}`)
      if (results.every((r) => r.ok)) close()
      else await load()
    } catch (err) {
      toast('error', errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const changeable = rows?.filter((row) => row.status !== 'same') ?? []

  return (
    <Dialog title={`Copy MCP servers to ${profile?.name ?? 'account'}`} onClose={close}>
      {error && <p className="note warn">{error}</p>}
      {!rows && !error && <p className="note">Reading the servers…</p>}
      {rows?.length === 0 && <p className="note">{main} has no user-level MCP servers.</p>}
      {rows && rows.length > 0 && (
        <div className="mcp-list">
          {rows.map((row) => (
            <label key={row.name} className={`mcp-row${row.status === 'same' ? ' same' : ''}`}>
              <input type="checkbox" checked={chosen.has(row.name)} disabled={row.status === 'same' || busy} onChange={() => toggle(row.name)} />
              <span className="mcp-name">{row.name}</span>
              <span className="badge">{row.transport}</span>
              <span className="mcp-status">{STATUS_LABELS[row.status]}</span>
            </label>
          ))}
        </div>
      )}
      {rows && rows.length > 0 && (
        <p className="note">
          Copies the servers {main} has at user level, replacing those of the same name. Servers that sign in through the browser need{' '}
          <code>/mcp</code> again in this account. Servers of a single project are not copied.
        </p>
      )}
      <div className="dialog-actions">
        <button className="btn" onClick={close}>
          {changeable.length > 0 ? 'Cancel' : 'Close'}
        </button>
        {changeable.length > 0 && (
          <button className="btn primary" disabled={busy || chosen.size === 0} onClick={() => void copy()}>
            {busy ? 'Copying…' : chosen.size === 1 ? 'Copy 1 server' : `Copy ${chosen.size} servers`}
          </button>
        )}
      </div>
    </Dialog>
  )
}
