import { useEffect, useState } from 'react'
import { Laptop, Link2, Unlink, Wifi } from 'lucide-react'
import { p2p, type P2pStatus } from '../api'
import { useApp } from '../context'
import { PAIRED, useCollab } from '../lib/collab'
import { Modal, Spinner, useConfirm } from './ui'

/** Settings → Devices: this device's name, pairing, and paired devices. */
export function Devices() {
  const { toast, fail } = useApp()
  const collab = useCollab()
  const [status, setStatus] = useState<P2pStatus | null>(null)
  const [name, setName] = useState('')
  const [code, setCode] = useState<{ code: string; invite: string } | null>(null)
  const [joining, setJoining] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const confirm = useConfirm()

  const refresh = () =>
    p2p
      .status()
      .then((s) => {
        setStatus(s)
        setName((n) => n || s.name)
      })
      .catch(() => {})
  useEffect(() => {
    refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [collab.peers])

  // Close the pairing code once the other device has joined.
  useEffect(() => {
    const onPaired = (e: Event) => {
      setCode(null)
      toast(`Paired with ${(e as CustomEvent<string>).detail}`)
    }
    window.addEventListener(PAIRED, onPaired)
    return () => window.removeEventListener(PAIRED, onPaired)
  }, [toast])

  if (!status?.ready) {
    return (
      <div className="row small muted">
        <Spinner /> Starting peer-to-peer…
      </div>
    )
  }

  const paired = status.peers.filter((p) => p.trusted)
  const nearby = status.peers.filter((p) => !p.trusted && p.nearby)

  return (
    <>
      <div className="field" style={{ maxWidth: 420 }}>
        <label>This device's name (shown to collaborators)</label>
        <input
          className="input"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== status.name && p2p.setName(name).then(refresh).catch(fail)}
          onKeyDown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
        />
      </div>
      <div className="row" style={{ marginBottom: 14 }}>
        <button
          className="btn primary"
          onClick={() =>
            p2p
              .startPairing()
              .then(setCode)
              .catch(fail)
          }
        >
          <Link2 size={14} /> Pair a device
        </button>
        <button className="btn" onClick={() => setJoining('')}>
          Join with a code
        </button>
      </div>

      {paired.length === 0 && <p className="small faint">No paired devices yet.</p>}
      {paired.map((p) => (
        <div key={p.id} className="device-row">
          <span className="device-dot" style={{ background: p.connected ? 'var(--success)' : p.nearby ? 'var(--warning)' : 'var(--text-faint)' }} />
          <Laptop size={15} style={{ color: p.color }} />
          <span className="grow ellipsis">{p.name}</span>
          <span className="small faint">{p.connected ? 'Connected · syncing' : p.nearby ? 'Nearby' : 'Offline'}</span>
          <button
            className="btn ghost sm"
            title="Unpair"
            onClick={async () => {
              if (await confirm.ask(`Unpair “${p.name}”? It will stop syncing with this device.`, 'Unpair')) {
                p2p.unpair(p.id).then(refresh).catch(fail)
              }
            }}
          >
            <Unlink size={13} />
          </button>
        </div>
      ))}
      {nearby.length > 0 && (
        <p className="small muted" style={{ marginBottom: 0 }}>
          <Wifi size={12} /> Nearby, not paired: {nearby.map((p) => p.name).join(', ')}
        </p>
      )}

      {code && (
        <Modal
          title="Pair a device"
          onClose={() => {
            p2p.cancelPairing().catch(() => {})
            setCode(null)
          }}
        >
          <p className="small muted" style={{ marginTop: 0 }}>
            On the other device, open <b>Settings → Devices → Join with a code</b> and enter:
          </p>
          <div className="pair-code">{code.code.slice(0, 3)} {code.code.slice(3)}</div>
          <p className="small muted">
            <b>Not on the same network?</b> Send them this invite instead (any chat works) — it has no IP address, just
            this device's identity and the code:
          </p>
          <div className="row">
            <input className="input invite-input" readOnly value={code.invite} onFocus={(e) => e.currentTarget.select()} />
            <button className="btn" onClick={() => navigator.clipboard?.writeText(code.invite).then(() => toast("Invite copied"))}>
              Copy
            </button>
          </div>
          <p className="small faint" style={{ marginBottom: 0 }}>
            Works once and expires in 5 minutes. Everything between devices is end-to-end encrypted.
          </p>
        </Modal>
      )}

      {joining !== null && (
        <Modal
          title="Join with a code"
          onClose={() => setJoining(null)}
          footer={
            <>
              <button className="btn" onClick={() => setJoining(null)}>Cancel</button>
              <button
                className="btn primary"
                disabled={busy || !(joining.replace(/\D/g, '').length === 6 || joining.trim().startsWith('astralis:'))}
                onClick={async () => {
                  setBusy(true)
                  try {
                    const peer = await p2p.join(joining)
                    toast(`Paired with ${peer.name}`)
                    setJoining(null)
                    refresh()
                  } catch (e) {
                    fail(e)
                  } finally {
                    setBusy(false)
                  }
                }}
              >
                {busy && <Spinner />} Pair
              </button>
            </>
          }
        >
          <div className="field">
            <label>The 6-digit code (same network) or an invite (anywhere)</label>
            <input
              className="input pair-input"
              autoFocus
              inputMode="numeric"
              placeholder="123 456  or  astralis:…"
              value={joining}
              onChange={(e) => setJoining(e.target.value)}
            />
          </div>
        </Modal>
      )}
      {confirm.element}
    </>
  )
}
