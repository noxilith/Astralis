import { useState } from 'react'
import { Check, ChevronsUpDown, Cloud, Copy, FolderOpen, KeyRound, Lock, LockOpen, LogOut, Mail, Pencil, Plus, Shapes, ShieldPlus, Trash2, Users } from 'lucide-react'
import { api, type Vault } from '../api'
import { useApp } from '../context'
import { Modal, Spinner, useConfirm, usePrompt } from './ui'
import { DuplicatesModal, OrganizeModal } from './VaultTools'
import { useCollab } from '../lib/collab'
import { CloudInvitesModal, useGoogle } from './GoogleDrive'

const CLOUD_PROVIDERS = ['Google Drive', 'Microsoft OneDrive', 'Box', 'Dropbox', 'pCloud']

/** Shared vaults (shared with you, or shared by you): a folder with two people in it. Encrypted
 *  ones get a small padlock (closed while locked). Distinct from your own (folder) and cloud. */
export function SharedVaultIcon({ size = 15, lock }: { size?: number; lock?: 'open' | 'closed' }) {
  return (
    <svg className="shared-vault-icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      {/* The folder (its corner left open for the padlock on encrypted vaults). */}
      <path d={'M20 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4' + (lock ? '' : 'v7a2 2 0 0 1-2 2')} />
      <circle cx="9" cy="11.5" r="2" />
      <path d="M5.5 17.5a3.5 3.5 0 0 1 7 0" />
      <circle cx="15" cy="11.5" r="1.6" />
      <path d="M14 15.2a3 3 0 0 1 3.6 1.1" />
      {lock && (
        <g strokeWidth={1.8}>
          <rect x="17" y="17" width="6" height="5" rx="1" fill="var(--bg-elev, #111)" />
          <path d={lock === 'closed' ? 'M18.3 17v-1.3a1.7 1.7 0 0 1 3.4 0V17' : 'M18.3 17v-1.3a1.7 1.7 0 0 1 3.3-.6'} />
        </g>
      )}
    </svg>
  )
}

export const isShared = (v: Vault) => !!v.owner || v.sharedWith.length > 0

export const vaultIcon = (v: Vault, size = 15) =>
  isShared(v) ? (
    <SharedVaultIcon size={size} lock={v.encrypted ? (v.unlocked ? 'open' : 'closed') : undefined} />
  ) : v.encrypted ? (
    v.unlocked ? <LockOpen size={size} /> : <Lock size={size} />
  ) : v.cloud ? (
    <Cloud size={size} />
  ) : (
    <FolderOpen size={size} />
  )

/** Shared with/by someone: a small people badge (with who, on hover). */
function SharedBadge({ v, names }: { v: Vault; names: (id: string) => string }) {
  if (v.owner) return <span className="vault-shared" title={`Shared with you by ${names(v.owner)}`}><Users size={11} /> {names(v.owner)}</span>
  if (v.sharedWith.length)
    return <span className="vault-shared" title={`Shared with ${v.sharedWith.map(names).join(', ')}`}><Users size={11} /> {v.sharedWith.length}</span>
  return null
}

/** Dropdown above the note list: switch, create, rename, lock and delete vaults. */
export function VaultSwitcher() {
  const { vaultId, setVaultId, setNoteId, vaults, refreshVaults, fail, toast, bump } = useApp()
  const [open, setOpen] = useState(false)
  const [form, setForm] = useState<{ encrypted: boolean; name: string; password: string; confirm: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [cloudOpen, setCloudOpen] = useState(false)
  const [tool, setTool] = useState<'organize' | 'duplicates' | null>(null)
  const [sharing, setSharing] = useState(false)
  const [people, setPeople] = useState(false)
  const [cloudName, setCloudName] = useState('')
  const google = useGoogle()

  const createCloud = async () => {
    setBusy(true)
    try {
      const v = await api.createCloudVault(cloudName)
      setCloudOpen(false)
      setCloudName('')
      await afterChange()
      switchTo(v.id)
      toast(`“${v.name}” is on Google Drive. Invite people from its menu.`)
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  const join = async () => {
    setOpen(false)
    toast('Pick the vault folder in your browser…', { timeout: 6000 })
    try {
      const name = await api.cloudJoin()
      await afterChange()
      toast(`Joined “${name}”`)
    } catch (e) {
      fail(e)
    }
  }
  const { peers } = useCollab()
  const nameOf = (id: string) => peers.find((p) => p.id === id)?.name ?? 'another device'
  const { setView } = useApp()
  const prompt = usePrompt()
  const confirm = useConfirm()
  const current = vaults.find((v) => v.id === vaultId)

  const switchTo = (id: string) => {
    setOpen(false)
    if (id === vaultId) return
    setNoteId(null)
    setVaultId(id)
  }

  const afterChange = async () => {
    await refreshVaults()
    bump()
  }

  const create = async () => {
    if (!form) return
    if (form.encrypted && form.password !== form.confirm) return toast('Passwords do not match', { kind: 'error' })
    setBusy(true)
    try {
      const v = await api.createVault(form.name, form.encrypted ? form.password : null)
      setForm(null)
      await afterChange()
      switchTo(v.id)
      if (v.encrypted) toast("Encrypted vault created. A forgotten password can't be recovered — keep it safe.", { timeout: 8000 })
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  const leave = async () => {
    if (!current?.owner) return
    setOpen(false)
    const ok = await confirm.ask(
      `Leave “${current.name}”? It's removed from this device; ${nameOf(current.owner)} keeps it. They can share it with you again.`,
      'Leave vault',
    )
    if (!ok) return
    try {
      await api.leaveVault(current.id)
      setNoteId(null)
      setVaultId(await api.defaultVault())
      await afterChange()
      toast(`Left “${current.name}”`)
    } catch (e) {
      fail(e)
    }
  }

  const toggleShare = async (peer: string, on: boolean) => {
    if (!current) return
    try {
      await (on ? api.shareVault(current.id, peer) : api.unshareVault(current.id, peer))
      await afterChange()
      toast(on ? `Sharing “${current.name}” with ${nameOf(peer)}` : `Stopped sharing with ${nameOf(peer)} — it's removed from their device`)
    } catch (e) {
      fail(e)
    }
  }

  const remove = async () => {
    if (!current) return
    setOpen(false)
    if (vaults.length <= 1) return toast("You can't delete your only vault", { kind: 'error' })
    const ok = await confirm.ask(
      `Delete “${current.name}” and ${current.noteCount === 1 ? 'its 1 note' : `all ${current.noteCount} notes`} inside it forever? This can't be undone.`,
      'Delete vault',
    )
    if (!ok) return
    let password: string | null = null
    if (current.encrypted) {
      password = await prompt.ask('Delete vault', 'Enter the vault password to confirm', '', true)
      if (!password) return
    }
    try {
      await api.deleteVault(current.id, password)
      setNoteId(null)
      setVaultId(await api.defaultVault())
      await afterChange()
      toast('Vault deleted')
    } catch (e) {
      fail(e)
    }
  }

  return (
    <div style={{ position: 'relative' }} className="grow">
      <button className="vault-btn" onClick={() => setOpen((o) => !o)} title="Switch vault">
        {current && vaultIcon(current)}
        <span className="grow ellipsis">{current?.name ?? 'Loading…'}</span>
        {current && <SharedBadge v={current} names={nameOf} />}
        <ChevronsUpDown size={14} className="faint" />
      </button>
      {open && (
        <>
          <div className="menu-backdrop" onClick={() => setOpen(false)} />
          <div className="menu vault-menu">
            <div className="menu-label">Vaults</div>
            {vaults.map((v) => (
              <button key={v.id} onClick={() => switchTo(v.id)}>
                {vaultIcon(v, 14)}
                <span className="grow ellipsis">{v.name}</span>
                <SharedBadge v={v} names={nameOf} />
                <span className="faint small">{v.noteCount}</span>
                {v.id === vaultId ? <Check size={14} /> : <span style={{ width: 14 }} />}
              </button>
            ))}
            <div className="menu-sep" />
            <button onClick={() => (setOpen(false), setForm({ encrypted: false, name: '', password: '', confirm: '' }))}>
              <Plus size={14} /> New vault
            </button>
            <button onClick={() => (setOpen(false), setForm({ encrypted: true, name: '', password: '', confirm: '' }))}>
              <ShieldPlus size={14} /> New encrypted vault
            </button>
            <button onClick={() => (setOpen(false), setCloudOpen(true))}>
              <Cloud size={14} /> New cloud vault…
            </button>
            {google?.email && (
              <button onClick={join}>
                <Users size={14} /> Join shared vault from Google Drive…
              </button>
            )}
            {current && (
              <>
                <div className="menu-sep" />
                <div className="menu-label ellipsis">{current.name}</div>
                {!current.encrypted && (
                  <>
                    <button onClick={() => (setOpen(false), setTool('organize'))}>
                      <Shapes size={14} /> Organize notes by topic…
                    </button>
                    <button onClick={() => (setOpen(false), setTool('duplicates'))}>
                      <Copy size={14} /> Find duplicate notes…
                    </button>
                  </>
                )}
                {!current.owner && current.cloud && (
                  <button onClick={() => (setOpen(false), setPeople(true))}>
                    <Mail size={14} /> Invite by email…
                  </button>
                )}
                {!current.owner && (
                  <button onClick={() => (setOpen(false), setSharing(true))}>
                    <Users size={14} /> Share with paired device…
                  </button>
                )}
                <button
                  onClick={async () => {
                    setOpen(false)
                    const name = await prompt.ask('Rename vault', 'Name', current.name)
                    if (name?.trim()) api.renameVault(current.id, name).then(afterChange).catch(fail)
                  }}
                >
                  <Pencil size={14} /> Rename
                </button>
                {current.encrypted && current.unlocked && (
                  <>
                    <button
                      onClick={() => {
                        setOpen(false)
                        setNoteId(null)
                        api.lockVault(current.id).then(afterChange).catch(fail)
                      }}
                    >
                      <Lock size={14} /> Lock now
                    </button>
                    <button
                      onClick={async () => {
                        setOpen(false)
                        const old = await prompt.ask('Change password', 'Current password', '', true)
                        if (!old) return
                        const next = await prompt.ask('Change password', 'New password (8+ characters)', '', true)
                        if (!next) return
                        api.changeVaultPassword(current.id, old, next).then(() => toast('Password changed')).catch(fail)
                      }}
                    >
                      <KeyRound size={14} /> Change password
                    </button>
                  </>
                )}
                {current.owner ? (
                  <button className="danger" onClick={leave}>
                    <LogOut size={14} /> Leave vault
                  </button>
                ) : (
                  <button className="danger" onClick={remove}>
                    <Trash2 size={14} /> Delete vault
                  </button>
                )}
              </>
            )}
          </div>
        </>
      )}

      {people && current && <CloudInvitesModal vault={current} onClose={() => (setPeople(false), void afterChange())} />}
      {sharing && current && (
        <Modal title={`Share “${current.name}”`} onClose={() => setSharing(false)} footer={<button className="btn" onClick={() => setSharing(false)}>Done</button>}>
          <p className="small muted" style={{ marginTop: 0 }}>
            It shows up as its own vault on their device, and you edit it together live. Your other vaults stay private.
            Stop sharing any time: it's removed from their device.
          </p>
          {current.encrypted && <p className="small muted">They'll need this vault's password to open it.</p>}
          {peers.filter((p) => p.trusted).length === 0 ? (
            <p className="small faint">No paired devices yet: pair one in Settings → Devices.</p>
          ) : (
            peers
              .filter((p) => p.trusted)
              .map((p) => {
                const on = current.sharedWith.includes(p.id)
                return (
                  <label key={p.id} className="share-row">
                    <input type="checkbox" checked={on} onChange={(e) => toggleShare(p.id, e.target.checked)} />
                    <span className="device-dot" style={{ background: p.color }} />
                    <span className="grow ellipsis">{p.name}</span>
                    <span className="small faint">{on ? (p.connected ? 'Shared · syncing' : 'Shared · syncs when online') : p.connected ? 'Online' : 'Offline'}</span>
                  </label>
                )
              })
          )}
        </Modal>
      )}
      {form && (
        <Modal
          title={form.encrypted ? 'New encrypted vault' : 'New vault'}
          onClose={() => setForm(null)}
          footer={
            <>
              <button className="btn" onClick={() => setForm(null)}>Cancel</button>
              <button
                className="btn primary"
                disabled={busy || !form.name.trim() || (form.encrypted && form.password.length < 8)}
                onClick={create}
              >
                {busy && <Spinner />} Create
              </button>
            </>
          }
        >
          <div className="field">
            <label>Name</label>
            <input
              className="input"
              autoFocus
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              onKeyDown={(e) => e.key === 'Enter' && !form.encrypted && create()}
            />
          </div>
          <label className="checkbox" style={{ marginBottom: 12 }}>
            <input type="checkbox" checked={form.encrypted} onChange={(e) => setForm({ ...form, encrypted: e.target.checked })} />
            Encrypt this vault with a password
          </label>
          {form.encrypted && (
            <>
              <div className="field">
                <label>Password (at least 8 characters)</label>
                <input className="input" type="password" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} />
              </div>
              <div className="field">
                <label>Confirm password</label>
                <input
                  className="input"
                  type="password"
                  value={form.confirm}
                  onChange={(e) => setForm({ ...form, confirm: e.target.value })}
                  onKeyDown={(e) => e.key === 'Enter' && create()}
                />
              </div>
              <p className="small muted">
                Notes are encrypted with XChaCha20-Poly1305 using a key derived from your password (Argon2id). Encrypted notes are
                never indexed, searched, tagged or seen by the AI.
              </p>
              <p className="small" style={{ color: 'var(--warning)' }}>If you forget this password, the notes cannot be recovered.</p>
            </>
          )}
        </Modal>
      )}
      {tool === 'organize' && <OrganizeModal onClose={() => setTool(null)} />}
      {tool === 'duplicates' && <DuplicatesModal onClose={() => setTool(null)} />}
      {cloudOpen && (
        <Modal
          title="New cloud vault"
          onClose={() => setCloudOpen(false)}
          footer={
            <>
              <button className="btn" onClick={() => setCloudOpen(false)}>Close</button>
              {google?.email ? (
                <button className="btn primary" disabled={busy || !cloudName.trim()} onClick={createCloud}>
                  {busy && <Spinner />} Create on Google Drive
                </button>
              ) : (
                <button className="btn primary" onClick={() => (setCloudOpen(false), setView('settings'))}>Connect a drive in Settings</button>
              )}
            </>
          }
        >
          <p className="small muted" style={{ marginTop: 0 }}>
            Stored in an <span className="kbd">Astralis/</span> folder on your drive, one subfolder per vault, end-to-end encrypted. Your other
            devices and the people you invite sync through it and edit together live.
          </p>
          <div className="list">
            {CLOUD_PROVIDERS.map((p) => {
              const ready = p === 'Google Drive' && !!google?.email
              return (
                <div key={p} className="list-item" style={{ opacity: ready ? 1 : 0.55, cursor: 'default', flexDirection: 'row', alignItems: 'center', gap: 8 }}>
                  <Cloud size={14} /> <span className="grow">{p}</span>
                  <span className="small faint">{ready ? google?.email : p === 'Google Drive' ? 'Not connected' : 'Coming soon'}</span>
                  {ready && <Check size={14} />}
                </div>
              )
            })}
          </div>
          {google?.email && (
            <div className="field" style={{ marginTop: 12 }}>
              <label>Vault name</label>
              <input className="input" autoFocus value={cloudName} placeholder="e.g. Family" onChange={(e) => setCloudName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && cloudName.trim() && createCloud()} />
            </div>
          )}
        </Modal>
      )}
      {prompt.element}
      {confirm.element}
    </div>
  )
}

/** Shown in place of the notes when the active vault is encrypted and locked. */
export function UnlockVault({ vault }: { vault: Vault }) {
  const { refreshVaults, bump } = useApp()
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  const unlock = async () => {
    setBusy(true)
    setError('')
    try {
      await api.unlockVault(vault.id, password)
      setPassword('')
      await refreshVaults()
      bump()
    } catch (e) {
      setError(typeof e === 'string' ? e : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="center-card">
      <div className="row" style={{ marginBottom: 14 }}>
        <Lock size={20} />
        <h2 style={{ margin: 0, fontSize: 17 }}>{vault.name}</h2>
      </div>
      <p className="muted small">This vault is encrypted. Enter its password to unlock it.</p>
      <div className="field">
        <input
          className="input"
          type="password"
          autoFocus
          placeholder="Password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && password && unlock()}
        />
        {error && <span className="small" style={{ color: 'var(--danger)' }}>{error}</span>}
      </div>
      <button className="btn primary" style={{ width: '100%' }} disabled={busy || !password} onClick={unlock}>
        {busy ? <Spinner /> : <KeyRound size={14} />} Unlock
      </button>
    </div>
  )
}
