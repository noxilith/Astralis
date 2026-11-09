import { useEffect, useState } from 'react'
import { Check, ChevronDown, ChevronRight, Cloud, ExternalLink, Mail, Trash2, FolderOpen } from 'lucide-react'
import { api, type CloudInvite, type GoogleStatus, type Vault } from '../api'
import { useApp } from '../context'
import { useCollab } from '../lib/collab'
import { Modal, Spinner } from './ui'

// Google sign-in state, shared by Settings and the vault menu.
let status: GoogleStatus | null = null
const listeners = new Set<(s: GoogleStatus | null) => void>()
export const refreshGoogle = () =>
  api
    .googleStatus()
    .then((s) => {
      status = s
      listeners.forEach((l) => l(s))
      return s
    })
    .catch(() => null)

/** Whether Google Drive is connected (and as whom). */
export function useGoogle() {
  const [s, setS] = useState(status)
  useEffect(() => {
    listeners.add(setS)
    if (!status) refreshGoogle()
    return () => void listeners.delete(setS)
  }, [])
  return s
}

const CONSOLE = 'https://console.cloud.google.com'

/** A link that opens in the system browser. */
function Ext({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <a href={href} onClick={(e) => (e.preventDefault(), void api.openUrl(href).catch(() => {}))}>
      {children} <ExternalLink size={10} />
    </a>
  )
}

/** Settings → Connected drives → Google Drive. */
export function GoogleDriveSettings() {
  const { toast, fail } = useApp()
  const g = useGoogle()
  const [setup, setSetup] = useState(false)
  const [clientId, setClientId] = useState('')
  const [secret, setSecret] = useState('')
  const [busy, setBusy] = useState(false)

  const save = async () => {
    setBusy(true)
    try {
      await api.googleSetClient(clientId, secret)
      await refreshGoogle()
      setSecret('')
      setSetup(false)
      toast('Google OAuth client saved — now connect')
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  const connect = async () => {
    setBusy(true)
    try {
      const email = await api.googleConnect()
      await refreshGoogle()
      toast(`Connected Google Drive as ${email}`)
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="drive-card">
      <div className="row">
        <Cloud size={15} className={g?.email ? '' : 'muted'} />
        <span className="grow">Google Drive</span>
        {g?.email ? (
          <>
            <span className="small faint ellipsis" style={{ maxWidth: 200 }}>
              <Check size={12} /> {g.email}
            </span>
            <button className="btn sm" onClick={() => api.googleDisconnect().then(refreshGoogle).catch(fail)}>
              Disconnect
            </button>
          </>
        ) : busy ? (
          <span className="small muted">
            <Spinner /> Finish in your browser…
          </span>
        ) : g?.configured ? (
          <>
            {!g.builtin && <button className="btn ghost sm" onClick={() => setSetup((s) => !s)}>Client…</button>}
            <button className="btn primary sm" onClick={connect}>Connect</button>
          </>
        ) : (
          <button className="btn sm" onClick={() => setSetup((s) => !s)}>
            {setup ? <ChevronDown size={13} /> : <ChevronRight size={13} />} Set up
          </button>
        )}
      </div>

      {setup && (
        <div className="drive-setup">
          <p className="small muted" style={{ marginTop: 0 }}>
            Astralis connects with <b>your own</b> Google Cloud OAuth client, so no one else's server is involved. One-time setup, about 5
            minutes:
          </p>
          <ol className="small drive-steps">
            <li>
              Open <Ext href={`${CONSOLE}/projectcreate`}>Google Cloud</Ext> and create a
              project (any name, e.g. “Astralis”).
            </li>
            <li>
              Enable the <Ext href={`${CONSOLE}/apis/library/drive.googleapis.com`}>Google Drive API</Ext> and
              the <Ext href={`${CONSOLE}/apis/library/picker.googleapis.com`}>Google Picker API</Ext>.
            </li>
            <li>
              In <Ext href={`${CONSOLE}/auth/branding`}>Google Auth Platform</Ext>: Get started → app
              name “Astralis”, your email → Audience <b>External</b>. Under <b>Data access</b> add the scope <span className="kbd">…/auth/drive.file</span>.
              Under <b>Audience</b>, click <b>Publish app</b> (fine for drive.file; otherwise sign-in expires every 7 days and only test users can
              join).
            </li>
            <li>
              <Ext href={`${CONSOLE}/auth/clients/create`}>Create a client</Ext> → Application type{' '}
              <b>Desktop app</b> → Create. Copy the client ID and secret here:
            </li>
          </ol>
          <div className="field">
            <label>Client ID</label>
            <input className="input" placeholder="…apps.googleusercontent.com" value={clientId} onChange={(e) => setClientId(e.target.value)} />
          </div>
          <div className="field">
            <label>Client secret</label>
            <input className="input" type="password" placeholder="GOCSPX-…" value={secret} onChange={(e) => setSecret(e.target.value)} />
          </div>
          <p className="small faint">
            Everyone you invite uses Astralis with the same client, so share your client ID and secret with them too (Settings → Connected
            drives on their side). Google calls a desktop client's secret non-confidential; it can't access anyone's Drive by itself.
          </p>
          <div className="row">
            <button className="btn" onClick={() => setSetup(false)}>Cancel</button>
            <button className="btn primary" disabled={busy || !clientId.trim() || !secret.trim()} onClick={save}>
              Save
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

/** A cloud vault's people: invite by Google email, see who joined, take people out. */
export function CloudInvitesModal({ vault, onClose }: { vault: Vault; onClose: () => void }) {
  const { toast, fail } = useApp()
  const { peers } = useCollab()
  const [invites, setInvites] = useState<CloudInvite[] | null>(null)
  const [email, setEmail] = useState('')
  const [busy, setBusy] = useState(false)
  // Google wants the owner to pick the vault folder once; this is what to finish afterwards.
  const [needsAccess, setNeedsAccess] = useState<{ retry: () => Promise<void> } | null>(null)
  const load = () => api.cloudInvites(vault.id).then(setInvites).catch(fail)
  /** Runs a sharing change; if Google first needs the folder picked, asks for that instead. */
  const sharing = async (change: () => Promise<void>) => {
    try {
      await change()
      setNeedsAccess(null)
    } catch (e) {
      if (String((e as { message?: string })?.message ?? e).includes('needs-folder-access')) setNeedsAccess({ retry: change })
      else fail(e)
    }
  }
  const grant = async () => {
    if (!needsAccess) return
    setBusy(true)
    try {
      await api.cloudGrantFolder(vault.id)
      await sharing(needsAccess.retry)
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }
  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [vault.id])

  const invite = async () => {
    setBusy(true)
    const who = email.trim()
    await sharing(async () => {
      await api.cloudInvite(vault.id, who)
      toast(`Invited ${who} — Google emails them the folder`)
      setEmail('')
      await load()
    })
    setBusy(false)
  }

  return (
    <Modal title={`People in “${vault.name}”`} onClose={onClose} footer={<button className="btn" onClick={onClose}>Done</button>}>
      <p className="small muted" style={{ marginTop: 0 }}>
        Invite someone by their Google email. Google emails them the vault's Drive folder; in Astralis they choose{' '}
        <b>Join shared vault from Google Drive…</b> and pick it. Then you edit together live — no pairing code. Google only ever stores
        encrypted data.
      </p>
      <div className="row" style={{ marginBottom: 12 }}>
        <input
          className="input grow"
          type="email"
          placeholder="friend@gmail.com"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && email.includes('@') && invite()}
        />
        <button className="btn primary" disabled={busy || !email.includes('@')} onClick={invite}>
          {busy ? <Spinner /> : <Mail size={13} />} Invite
        </button>
      </div>
      {needsAccess && (
        <div className="drive-access">
          <b>Google needs your OK once</b>
          <p className="small muted">
            The people in this vault have added files to its folder, and Google only lets Astralis change who the folder is shared with
            once you've given it access to everything in it. In the Google window that opens, go into the <b>Astralis</b> folder and pick{' '}
            <b>{vault.name}</b>. You only need to do this once for this vault.
          </p>
          <div className="row" style={{ gap: 8 }}>
            <button className="btn primary" disabled={busy} onClick={grant}>
              {busy ? <Spinner /> : <FolderOpen size={13} />} Pick the folder in Google Drive
            </button>
            <button className="btn ghost" disabled={busy} onClick={() => setNeedsAccess(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {invites === null ? (
        <Spinner />
      ) : invites.length === 0 ? (
        <p className="small faint">No one invited yet.</p>
      ) : (
        invites.map((i) => {
          const device = i.acceptedBy ? peers.find((p) => p.id === i.acceptedBy) : undefined
          return (
            <div key={i.email} className="share-row">
              <Mail size={13} className="muted" />
              <span className="grow ellipsis">{i.email}</span>
              <span className="small faint">
                {i.acceptedBy ? (device?.connected ? `Joined · ${device.name} online` : 'Joined') : 'Invited · not joined yet'}
              </span>
              <button
                className="btn ghost sm"
                title="Remove (their copy is removed too)"
                onClick={() =>
                  sharing(async () => {
                    await api.cloudUninvite(vault.id, i.email)
                    toast(`Removed ${i.email}`)
                    await load()
                  })
                }
              >
                <Trash2 size={13} />
              </button>
            </div>
          )
        })
      )}
    </Modal>
  )
}
