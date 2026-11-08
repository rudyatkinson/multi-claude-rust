import { useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { FONT_SIZE, PROFILE_COLORS, type AuthKind, type ExplorerMenuStatus } from '@shared/types'
import { mc } from '../api'
import { attempt, dismissToast, mainProfileName, setActive, setModal, toast, useStore } from '../store'

export function Dialog({ title, children, onClose }: { title: string; children: ReactNode; onClose(): void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog" role="dialog" aria-label={title}>
        <div className="dialog-title">{title}</div>
        {children}
      </div>
    </div>
  )
}

function ColorPicker({ value, onChange }: { value: string; onChange(color: string): void }) {
  return (
    <div className="swatches">
      {PROFILE_COLORS.map((color) => (
        <button
          key={color}
          type="button"
          className={`swatch${color === value ? ' selected' : ''}`}
          style={{ background: color }}
          aria-label={color}
          onClick={() => onChange(color)}
        />
      ))}
    </div>
  )
}

const close = (): void => setModal(null)

const SHARE_LABELS: Record<string, string> = {
  agents: 'Agents',
  skills: 'Skills',
  commands: 'Commands',
  plugins: 'Plugins',
  'settings.json': 'Settings (settings.json)',
  'CLAUDE.md': 'Global instructions (CLAUDE.md)',
  'keybindings.json': 'Key bindings (keybindings.json)'
}

function TokenField({ value, onChange }: { value: string; onChange(token: string): void }) {
  return (
    <label className="field">
      <span>
        Token <em>(run "claude setup-token" in a terminal to create one)</em>
      </span>
      <input
        type="password"
        value={value}
        placeholder="sk-ant-..."
        autoComplete="off"
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  )
}

export function AddAccountWizard() {
  const profiles = useStore((s) => s.profiles)
  const shareableDirs = useStore((s) => s.shareableDirs)
  const shareableFiles = useStore((s) => s.shareableFiles)
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [auth, setAuth] = useState<AuthKind>('login')
  const [token, setToken] = useState('')
  const [color, setColor] = useState(() => PROFILE_COLORS.find((c) => !profiles.some((p) => p.color === c)) ?? PROFILE_COLORS[1])
  const [shared, setShared] = useState<Set<string>>(() => new Set([...shareableDirs, ...shareableFiles]))
  const [keepModelSeparate, setKeepModelSeparate] = useState(true)
  const [busy, setBusy] = useState(false)

  const toggle = (item: string): void => {
    const next = new Set(shared)
    if (!next.delete(item)) next.add(item)
    setShared(next)
  }

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    setBusy(true)
    const result = await attempt(
      mc.invoke('profiles:create', {
        name,
        color,
        auth,
        token: auth === 'token' ? token : undefined,
        email: email.trim() || undefined,
        shareDirs: shareableDirs.filter((d) => shared.has(d)),
        shareFiles: shareableFiles.filter((f) => shared.has(f)),
        keepModelSeparate
      })
    )
    setBusy(false)
    if (!result) return
    for (const warning of result.warnings) toast('warn', warning)
    if (result.loginSession) setActive(result.loginSession.id)
    close()
  }

  return (
    <Dialog title="Add account" onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <label className="field">
          <span>Name</span>
          <input autoFocus value={name} maxLength={40} placeholder="e.g. Work, Personal" onChange={(e) => setName(e.target.value)} />
        </label>
        <div className="field">
          <span>Sign-in</span>
          <div className="segmented">
            <button type="button" className={auth === 'login' ? 'selected' : ''} onClick={() => setAuth('login')}>
              Browser login
            </button>
            <button type="button" className={auth === 'token' ? 'selected' : ''} onClick={() => setAuth('token')}>
              OAuth token
            </button>
          </div>
        </div>
        {auth === 'login' ? (
          <label className="field">
            <span>
              Email <em>(optional, pre-fills the login page)</em>
            </span>
            <input type="email" value={email} placeholder="name@example.com" onChange={(e) => setEmail(e.target.value)} />
          </label>
        ) : (
          <TokenField value={token} onChange={setToken} />
        )}
        <div className="field">
          <span>Color</span>
          <ColorPicker value={color} onChange={setColor} />
        </div>

        <div className="field">
          <span>Share from {mainProfileName()}</span>
          <div className="checks">
            {[...shareableDirs, ...shareableFiles].map((item) => (
              <label key={item} className="check">
                <input type="checkbox" checked={shared.has(item)} onChange={() => toggle(item)} />
                {SHARE_LABELS[item] ?? item}
              </label>
            ))}
          </div>
          <label className="check">
            <input
              type="checkbox"
              checked={keepModelSeparate}
              disabled={!shared.has('settings.json')}
              onChange={(e) => setKeepModelSeparate(e.target.checked)}
            />
            Keep model and effort settings separate for this account
          </label>
        </div>

        <p className="note">
          Folders are linked to {mainProfileName()}'s; files are copied. MCP servers and the theme stay separate for each account; MCP
          servers can be copied later from the account's menu.
          Install plugins from {mainProfileName()}.
        </p>
        {auth === 'login' ? (
          <p className="note warn">
            The login opens in your browser. If another Claude account is signed in there, the login uses that one: open the link in
            a private window, or sign out of claude.ai first.
          </p>
        ) : (
          <p className="note warn">
            A token account shows no usage, email or plan, and cannot be opened in an external terminal. The token is only checked
            when a session starts.
          </p>
        )}

        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={busy || !name.trim() || (auth === 'token' && !token.trim())}>
            {auth === 'login' ? 'Create and log in' : 'Create'}
          </button>
        </div>
      </form>
    </Dialog>
  )
}

export function ReplaceTokenDialog({ profileId }: { profileId: string }) {
  const profile = useStore((s) => s.profiles.find((p) => p.id === profileId))
  const [token, setToken] = useState('')
  if (!profile) return null

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    await attempt(mc.invoke('profiles:setToken', profileId, token).then(close))
  }

  return (
    <Dialog title={`Replace the token of ${profile.name}`} onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <TokenField value={token} onChange={setToken} />
        <p className="note">Sessions that are already open keep the old token until they are restarted.</p>
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!token.trim()}>
            Save
          </button>
        </div>
      </form>
    </Dialog>
  )
}

export function EditProfileDialog({ profileId }: { profileId: string }) {
  const profile = useStore((s) => s.profiles.find((p) => p.id === profileId))
  const [name, setName] = useState(profile?.name ?? '')
  const [color, setColor] = useState(profile?.color ?? PROFILE_COLORS[0])
  if (!profile) return null

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    await attempt(mc.invoke('profiles:update', profileId, { name, color }).then(close))
  }

  return (
    <Dialog title="Name and color" onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <label className="field">
          <span>Name</span>
          <input autoFocus value={name} maxLength={40} onChange={(e) => setName(e.target.value)} />
        </label>
        <div className="field">
          <span>Color</span>
          <ColorPicker value={color} onChange={setColor} />
        </div>
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!name.trim()}>
            Save
          </button>
        </div>
      </form>
    </Dialog>
  )
}

function explorerMenuText(status: ExplorerMenuStatus | null): string {
  if (!status) return 'Checking…'
  if (status.tier === 'package') return 'On: shown in the Windows 11 menu and in the classic menu.'
  if (status.tier === 'classic') return 'On: shown in the classic menu ("Show more options").'
  return 'Off.'
}

/** Adds or removes the "Open in Multi Claude" entry of the File Explorer context menu. */
function ExplorerMenuSetting() {
  const [status, setStatus] = useState<ExplorerMenuStatus | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    void attempt(mc.invoke('explorerMenu:status')).then((s) => s && setStatus(s))
  }, [])

  const change = async (enabled: boolean): Promise<void> => {
    setBusy(true)
    const next = await attempt(mc.invoke('explorerMenu:set', enabled))
    setBusy(false)
    if (!next) return
    setStatus(next)
    if (enabled && next.tier) toast('info', 'Added to the context menu. If it does not show up, restart File Explorer.')
  }

  // only worth explaining while the entry is off or stuck on the lesser tier
  const showReason = status?.reason && status.tier !== 'package'
  return (
    <div className="field">
      <span>Explorer context menu</span>
      <div className="setting-row">
        <span className="setting-text">Right-clicking a folder offers "Open in Multi Claude". {explorerMenuText(status)}</span>
        <button type="button" className="btn" disabled={!status || busy} onClick={() => void change(!status?.tier)}>
          {status?.tier ? 'Remove' : 'Add'}
        </button>
      </div>
      {showReason && <p className="note">{status.reason}</p>}
      {status?.detail && <p className="note warn">{status.detail}</p>}
    </div>
  )
}

export function SettingsDialog() {
  const settings = useStore((s) => s.settings)
  const claude = useStore((s) => s.claude)
  const platform = useStore((s) => s.platform)
  const packaged = useStore((s) => s.packaged)
  const [claudePath, setClaudePath] = useState(settings.claudePath ?? '')
  const [fontSize, setFontSize] = useState(String(settings.fontSize))
  // both are on unless they were switched off
  const [restoreTabs, setRestoreTabs] = useState(settings.restoreTabs !== false)
  const [notifications, setNotifications] = useState(settings.notifications !== false)
  const [limitAlerts, setLimitAlerts] = useState(settings.limitAlerts !== false)
  const [autoSwitch, setAutoSwitch] = useState(settings.autoSwitch === true)
  // the .cmd shim cannot hand Claude the message that makes it go on over there
  const canAutoSwitch = claude.path !== null && !(platform === 'win32' && /\.(cmd|bat)$/i.test(claude.path))
  const [tray, setTray] = useState(settings.tray !== false)
  const [openAtLogin, setOpenAtLogin] = useState(settings.openAtLogin === true)
  const canStartWithSystem = packaged && platform !== 'linux'

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    const size = Number(fontSize)
    await attempt(
      mc
        .invoke('settings:update', {
          claudePath,
          fontSize: Number.isFinite(size) ? size : settings.fontSize,
          restoreTabs,
          notifications,
          limitAlerts,
          autoSwitch,
          tray,
          // not sent from a development run, which must not register itself
          ...(canStartWithSystem ? { openAtLogin } : {})
        })
        .then(close)
    )
  }

  return (
    <Dialog title="Settings" onClose={close}>
      <form onSubmit={(e) => void submit(e)}>
        <label className="field">
          <span>
            claude path <em>(found automatically when empty)</em>
          </span>
          <input value={claudePath} placeholder={claude.path ?? ''} spellCheck={false} onChange={(e) => setClaudePath(e.target.value)} />
        </label>
        <p className={`note${claude.path ? '' : ' warn'}`}>
          {claude.path ? `Using: ${claude.path} (${claude.version ?? '?'})` : (claude.error ?? 'claude was not found')}
        </p>
        <label className="field">
          <span>Terminal font size</span>
          <input type="number" min={FONT_SIZE.min} max={FONT_SIZE.max} value={fontSize} onChange={(e) => setFontSize(e.target.value)} />
        </label>
        <div className="field">
          <span>Sessions</span>
          <label className="check">
            <input type="checkbox" checked={restoreTabs} onChange={(e) => setRestoreTabs(e.target.checked)} />
            Reopen tabs on startup and continue their conversations
          </label>
          <label className="check">
            <input type="checkbox" checked={notifications} onChange={(e) => setNotifications(e.target.checked)} />
            Notify when a session in the background needs input
          </label>
        </div>
        <div className="field">
          <span>Accounts</span>
          <label className="check">
            <input type="checkbox" checked={limitAlerts} onChange={(e) => setLimitAlerts(e.target.checked)} />
            Notify when an account nears a usage limit, and when that limit resets
          </label>
          <label className="check" title={canAutoSwitch ? undefined : 'Needs the native claude executable, not the .cmd one'}>
            <input type="checkbox" checked={autoSwitch && canAutoSwitch} disabled={!canAutoSwitch} onChange={(e) => setAutoSwitch(e.target.checked)} />
            Move a conversation to another account by itself when a usage limit cuts it off
          </label>
          <p className="note">
            Claude picks the task up on the logged-in account with the most room; the first message there sends the whole conversation again. Off:
            Claude Code waits in the tab for the reset.
          </p>
        </div>
        <div className="field">
          <span>Window</span>
          <label className="check">
            <input type="checkbox" checked={tray} onChange={(e) => setTray(e.target.checked)} />
            Keep running in the tray when the window is closed
          </label>
          <label className="check" title={canStartWithSystem ? undefined : 'Only for the installed app'}>
            <input type="checkbox" checked={openAtLogin && canStartWithSystem} disabled={!canStartWithSystem} onChange={(e) => setOpenAtLogin(e.target.checked)} />
            Start with {platform === 'win32' ? 'Windows' : 'the system'}
          </label>
        </div>
        {platform === 'win32' && <ExplorerMenuSetting />}
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary">
            Save
          </button>
        </div>
      </form>
    </Dialog>
  )
}

export function ConfirmDialog() {
  const confirm = useStore((s) => s.confirm)
  if (!confirm) return null
  return (
    <Dialog title={confirm.title} onClose={() => confirm.resolve(false)}>
      <p className="confirm-text">{confirm.message}</p>
      <div className="dialog-actions">
        <button className="btn" onClick={() => confirm.resolve(false)}>
          Cancel
        </button>
        <button autoFocus className={`btn ${confirm.danger ? 'danger' : 'primary'}`} onClick={() => confirm.resolve(true)}>
          {confirm.confirmLabel}
        </button>
      </div>
    </Dialog>
  )
}

export function Toasts() {
  const toasts = useStore((s) => s.toasts)
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <button key={t.id} className={`toast ${t.level}`} onClick={() => dismissToast(t.id)}>
          {t.message}
        </button>
      ))}
    </div>
  )
}
