import { useEffect, useState } from 'react'
import { Check, Cloud } from 'lucide-react'
import { api, type MyCloudVault } from '../api'
import { useApp } from '../context'
import { Modal, Spinner } from './ui'
import { useGoogle } from './GoogleDrive'

/**
 * "Add existing cloud vault": the vaults in this Google account's Drive (made on any of its
 * devices) that aren't on this device yet. Needs only this device and Google Drive: the key is read
 * from the Drive, so the device that made the vault doesn't have to be open.
 */
export function AddCloudVaultModal({ onClose, onAdded }: { onClose: () => void; onAdded: (name: string) => void | Promise<void> }) {
  const google = useGoogle()
  const { setView, fail } = useApp()
  const [list, setList] = useState<MyCloudVault[] | null>(null)
  const [error, setError] = useState('')
  const [adding, setAdding] = useState<string | null>(null)

  useEffect(() => {
    if (!google?.email) return
    setError('')
    api.cloudMyVaults().then(setList).catch((e) => setError(e instanceof Error ? e.message : String(e)))
  }, [google?.email])

  const add = async (v: MyCloudVault) => {
    setAdding(v.folderId)
    setError('')
    try {
      await onAdded(await api.cloudAddMine(v.folderId))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      fail(e)
    } finally {
      setAdding(null)
    }
  }

  const owner = (v: MyCloudVault) => v.ownerName || 'the device that made it'

  return (
    <Modal
      title="Add existing cloud vault"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Close</button>
          {!google?.email && (
            <button className="btn primary" onClick={() => (onClose(), setView('settings'))}>Connect a drive in Settings</button>
          )}
        </>
      }
    >
      <p className="small muted" style={{ marginTop: 0 }}>
        The cloud vaults in your Google Drive, made on any of your devices. Adding one needs only this device and Google Drive: the
        device that made it doesn't have to be open.
      </p>
      {!google?.email && (
        <p className="small">Connect Google Drive first (Settings → Connected drives), with the same account as your other devices.</p>
      )}
      {google?.email && !list && !error && (
        <p className="small muted">
          <Spinner /> Looking in your Drive…
        </p>
      )}
      {list && list.length === 0 && (
        <p className="small">No cloud vaults found in {google?.email}. Make one with “New cloud vault” on another device first.</p>
      )}
      <div className="list">
        {(list ?? []).map((v) => (
          <div key={v.folderId} className="list-item" style={{ cursor: 'default', flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <Cloud size={14} />
            <span className="grow">
              <span className="ellipsis">{v.name}</span>
              <span className="small faint" style={{ display: 'block' }}>
                {v.onThisDevice ? 'Already on this device' : v.ready ? `From ${v.ownerName || 'another device'}` : `Open the latest Astralis once on ${owner(v)}, then try again`}
              </span>
            </span>
            {v.onThisDevice ? (
              <Check size={14} />
            ) : (
              <button className="btn primary sm" disabled={!v.ready || adding !== null} onClick={() => add(v)}>
                {adding === v.folderId && <Spinner />} Add
              </button>
            )}
          </div>
        ))}
      </div>
      {error && <p className="small" style={{ color: 'var(--danger)', wordBreak: 'break-word' }}>{error}</p>}
    </Modal>
  )
}
