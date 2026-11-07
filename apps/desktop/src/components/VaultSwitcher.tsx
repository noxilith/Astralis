import { useState } from 'react'
import { Check, ChevronsUpDown, Cloud, Copy, FolderOpen, KeyRound, Lock, LockOpen, Pencil, Plus, Shapes, ShieldPlus, Trash2 } from 'lucide-react'
import { api, type Vault } from '../api'
import { useApp } from '../context'
import { Modal, Spinner, useConfirm, usePrompt } from './ui'
import { DuplicatesModal, OrganizeModal } from './VaultTools'

const CLOUD_PROVIDERS = ['Google Drive', 'Microsoft OneDrive', 'Box', 'Dropbox', 'pCloud']

export const vaultIcon = (v: Vault, size = 15) =>
  !v.encrypted ? <FolderOpen size={size} /> : v.unlocked ? <LockOpen size={size} /> : <Lock size={size} />

/** Dropdown above the note list: switch, create, rename, lock and delete vaults. */
export function VaultSwitcher() {
  const { vaultId, setVaultId, setNoteId, vaults, refreshVaults, fail, toast, bump } = useApp()
  const [open, setOpen] = useState(false)
  const [form, setForm] = useState<{ encrypted: boolean; name: string; password: string; confirm: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [cloudOpen, setCloudOpen] = useState(false)
  const [tool, setTool] = useState<'organize' | 'duplicates' | null>(null)
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
                <button className="danger" onClick={remove}>
                  <Trash2 size={14} /> Delete vault
                </button>
              </>
            )}
          </div>
        </>
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
              <button className="btn primary" onClick={() => (setCloudOpen(false), setView('settings'))}>Connect a drive in Settings</button>
            </>
          }
        >
          <p className="small muted" style={{ marginTop: 0 }}>
            Choose which connected drive stores this vault. It's kept in an <span className="kbd">Astralis/</span> folder on the drive, one
            subfolder per vault.
          </p>
          <div className="list">
            {CLOUD_PROVIDERS.map((p) => (
              <div key={p} className="list-item" style={{ opacity: 0.55, cursor: 'default', flexDirection: 'row', alignItems: 'center', gap: 8 }}>
                <Cloud size={14} /> <span className="grow">{p}</span> <span className="small faint">Not connected</span>
              </div>
            ))}
          </div>
          <p className="small faint">Cloud vaults are coming soon — no drives can be connected yet.</p>
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
