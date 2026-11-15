import { useEffect, useState } from 'react'
import { Check, Cloud } from 'lucide-react'
import { api, type CloudProvider, type MyCloudVault } from '../api'
import { useApp } from '../context'
import { Modal, Spinner } from './ui'
import { useGoogle } from './GoogleDrive'
import { useOneDrive } from './OneDrive'

/**
 * "Add existing cloud vault": the vaults in this Google account's Drive (made on any of its
 * devices) that aren't on this device yet. Needs only this device and Google Drive: the key is read
 * from the Drive, so the device that made the vault doesn't have to be open.
 */
export function AddCloudVaultModal({ onClose, onAdded }: { onClose: () => void; onAdded: (name: string) => void | Promise<void> }) {
  const google = useGoogle()
  const onedrive = useOneDrive()
  const { setView, fail } = useApp()
  const [provider, setProvider] = useState<CloudProvider>(!google?.email && onedrive?.email ? 'onedrive' : 'google-drive')
  const account = provider === 'onedrive' ? onedrive?.email : google?.email
  const label = provider === 'onedrive' ? 'OneDrive' : 'Google Drive'
  const [list, setList] = useState<MyCloudVault[] | null>(null)
  const [error, setError] = useState('')
  const [adding, setAdding] = useState<string | null>(null)

  useEffect(() => {
    if (!account) return
    setError('')
    setList(null)
    api.cloudMyVaults(provider).then(setList).catch((e) => setError(e instanceof Error ? e.message : String(e)))
  }, [account, provider])

  const add = async (v: MyCloudVault) => {
    setAdding(v.folderId)
    setError('')
    try {
      await onAdded(await api.cloudAddMine(v.folderId, provider))
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
          {!account && (
            <button className="btn primary" onClick={() => (onClose(), setView('settings'))}>Connect a drive in Settings</button>
          )}
        </>
      }
    >
      <p className="small muted" style={{ marginTop: 0 }}>
        The cloud vaults in your {label}, made on any of your devices. Adding one needs only this device and {label}: the
        device that made it doesn't have to be open.
      </p>
      {google?.email && onedrive?.email && (
        <div className="row" style={{ marginBottom: 10 }}>
          <button className={'btn sm' + (provider === 'google-drive' ? ' primary' : '')} onClick={() => setProvider('google-drive')}>Google Drive</button>
          <button className={'btn sm' + (provider === 'onedrive' ? ' primary' : '')} onClick={() => setProvider('onedrive')}>OneDrive</button>
        </div>
      )}
      {!account && (
        <p className="small">Connect {label} first (Settings → Connected drives), with the same account as your other devices.</p>
      )}
      {account && !list && !error && (
        <p className="small muted">
          <Spinner /> Looking in your {label}…
        </p>
      )}
      {list && list.length === 0 && (
        <p className="small">No cloud vaults found in {account}. Make one with “New cloud vault” on another device first.</p>
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
