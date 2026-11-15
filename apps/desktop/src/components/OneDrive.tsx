import { useEffect, useState } from 'react'
import { Check, ChevronDown, ChevronRight, Cloud, ExternalLink, Users } from 'lucide-react'
import { api, type OneDriveStatus, type SharedVault } from '../api'
import { useApp } from '../context'
import { Modal, Spinner } from './ui'

// OneDrive sign-in state, shared by Settings and the vault menu.
let status: OneDriveStatus | null = null
const listeners = new Set<(s: OneDriveStatus | null) => void>()
export const refreshOneDrive = () =>
  api
    .onedriveStatus()
    .then((s) => {
      status = s
      listeners.forEach((l) => l(s))
      return s
    })
    .catch(() => null)

/** Whether OneDrive is connected (and as whom). */
export function useOneDrive() {
  const [s, setS] = useState(status)
  useEffect(() => {
    listeners.add(setS)
    if (!status) refreshOneDrive()
    const back = () => document.visibilityState === 'visible' && void refreshOneDrive()
    document.addEventListener('visibilitychange', back)
    return () => {
      document.removeEventListener('visibilitychange', back)
      listeners.delete(setS)
    }
  }, [])
  return s
}

const PORTAL = 'https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade'

function Ext({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <a href={href} onClick={(e) => (e.preventDefault(), void api.openUrl(href).catch(() => {}))}>
      {children} <ExternalLink size={10} />
    </a>
  )
}

/** Settings → Connected drives → OneDrive. */
export function OneDriveSettings() {
  const { toast, fail } = useApp()
  const s = useOneDrive()
  const [setup, setSetup] = useState(false)
  const [clientId, setClientId] = useState('')
  const [busy, setBusy] = useState(false)
  const [signInError, setSignInError] = useState('')

  const save = async () => {
    setBusy(true)
    try {
      await api.onedriveSetClient(clientId)
      await refreshOneDrive()
      setSetup(false)
      toast('Microsoft app saved — now connect')
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  const connect = async () => {
    setBusy(true)
    setSignInError('')
    try {
      const email = await api.onedriveConnect()
      await refreshOneDrive()
      toast(`Connected OneDrive as ${email}`)
    } catch (e) {
      setSignInError(e instanceof Error ? e.message : String(e))
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="drive-card">
      <div className="row">
        <Cloud size={15} className={s?.email ? '' : 'muted'} />
        <span className="grow">Microsoft OneDrive</span>
        {s?.email ? (
          <>
            <span className="small faint ellipsis" style={{ maxWidth: 200 }}>
              <Check size={12} /> {s.email}
            </span>
            <button className="btn sm" onClick={() => api.onedriveDisconnect().then(refreshOneDrive).catch(fail)}>
              Disconnect
            </button>
          </>
        ) : busy ? (
          <span className="small muted">
            <Spinner /> Finish in your browser…
          </span>
        ) : s?.configured ? (
          <>
            {!s.builtin && <button className="btn ghost sm" onClick={() => setSetup((v) => !v)}>App…</button>}
            <button className="btn primary sm" onClick={connect}>Connect</button>
          </>
        ) : (
          <button className="btn sm" onClick={() => setSetup((v) => !v)}>
            {setup ? <ChevronDown size={13} /> : <ChevronRight size={13} />} Set up
          </button>
        )}
      </div>
      {signInError && <p className="small" style={{ color: 'var(--danger)', margin: '6px 0 0', wordBreak: 'break-word' }}>Couldn't connect: {signInError}</p>}

      {setup && (
        <div className="drive-setup">
          <p className="small muted" style={{ marginTop: 0 }}>
            Astralis connects with <b>your own</b> Microsoft app registration, so no one else's server is involved. It's free and there's no
            secret to copy, only an ID:
          </p>
          <ol className="small drive-steps">
            <li>
              Open <Ext href={PORTAL}>App registrations</Ext> in the Microsoft Entra admin center and choose <b>New registration</b>.
            </li>
            <li>
              Name it “Astralis”. For <b>Supported account types</b> pick “…any organizational directory and personal Microsoft accounts”.
              For <b>Redirect URI</b> pick <b>Public client/native (mobile &amp; desktop)</b> and enter <span className="kbd">http://localhost</span>.
            </li>
            <li>
              Under <b>API permissions</b> add Microsoft Graph <i>delegated</i> permissions <span className="kbd">Files.ReadWrite</span>,{' '}
              <span className="kbd">offline_access</span> and <span className="kbd">User.Read</span>. Under <b>Authentication</b>, set{' '}
              <b>Allow public client flows</b> to <b>Yes</b>.
            </li>
            <li>Copy the <b>Application (client) ID</b> from the Overview page and paste it here:</li>
          </ol>
          <div className="field">
            <label>Application (client) ID</label>
            <input className="input" placeholder="11111111-2222-3333-4444-555555555555" value={clientId} onChange={(e) => setClientId(e.target.value)} />
          </div>
          <p className="small faint">
            Everyone you invite uses Astralis with the same app, so share this ID with them too. A client ID isn't secret and can't access
            anyone's OneDrive by itself.
          </p>
          <div className="row">
            <button className="btn" onClick={() => setSetup(false)}>Cancel</button>
            <button className="btn primary" disabled={busy || !clientId.trim()} onClick={save}>
              Save
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

/** "Join shared vault from OneDrive": the vaults other people shared with this account. */
export function JoinOneDriveModal({ onClose, onJoined }: { onClose: () => void; onJoined: (name: string) => void | Promise<void> }) {
  const one = useOneDrive()
  const { setView, fail } = useApp()
  const [list, setList] = useState<SharedVault[] | null>(null)
  const [error, setError] = useState('')
  const [joining, setJoining] = useState<string | null>(null)

  useEffect(() => {
    if (!one?.email) return
    api.cloudSharedVaults().then(setList).catch((e) => setError(e instanceof Error ? e.message : String(e)))
  }, [one?.email])

  const join = async (v: SharedVault) => {
    setJoining(v.folderId)
    setError('')
    try {
      await onJoined(await api.cloudJoinOnedrive(v.folderId))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      fail(e)
    } finally {
      setJoining(null)
    }
  }

  return (
    <Modal
      title="Join shared vault from OneDrive"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Close</button>
          {!one?.email && <button className="btn primary" onClick={() => (onClose(), setView('settings'))}>Connect OneDrive in Settings</button>}
        </>
      }
    >
      <p className="small muted" style={{ marginTop: 0 }}>
        Vaults other people shared with your Microsoft account. Joining connects to the owner's Astralis, so they need to be online once.
      </p>
      {!one?.email && <p className="small">Connect OneDrive first (Settings → Connected drives).</p>}
      {one?.email && !list && !error && (
        <p className="small muted">
          <Spinner /> Looking in your OneDrive…
        </p>
      )}
      {list && list.length === 0 && <p className="small">No shared vaults found. Ask the owner to invite {one?.email}, then accept their email invitation.</p>}
      <div className="list">
        {(list ?? []).map((v) => (
          <div key={v.folderId} className="list-item" style={{ cursor: 'default', flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <Users size={14} />
            <span className="grow">
              <span className="ellipsis">{v.name}</span>
              <span className="small faint" style={{ display: 'block' }}>From {v.ownerName || 'another device'}</span>
            </span>
            <button className="btn primary sm" disabled={joining !== null} onClick={() => join(v)}>
              {joining === v.folderId && <Spinner />} Join
            </button>
          </div>
        ))}
      </div>
      {error && <p className="small" style={{ color: 'var(--danger)', wordBreak: 'break-word' }}>{error}</p>}
    </Modal>
  )
}
