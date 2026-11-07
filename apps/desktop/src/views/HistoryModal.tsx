import { useEffect, useState } from 'react'
import { api, type Note, type NoteVersion } from '../api'
import { useApp } from '../context'
import { Empty, Modal } from '../components/ui'
import { formatDateTime } from '../lib/time'

export function HistoryModal({ note, onClose, onRestored }: { note: Note; onClose: () => void; onRestored: (n: Note) => void }) {
  const { fail, toast } = useApp()
  const [versions, setVersions] = useState<NoteVersion[]>([])
  const [selected, setSelected] = useState<NoteVersion | null>(null)

  const load = () =>
    api
      .noteHistory(note.id)
      .then((v) => {
        setVersions(v)
        setSelected((s) => s ?? v[0] ?? null)
      })
      .catch(fail)

  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [note.id])

  return (
    <Modal
      title={`History — ${note.title || 'Untitled'}`}
      wide
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={() => api.snapshotNote(note.id).then(() => (toast('Version saved'), load())).catch(fail)}>
            Save current version
          </button>
          <span className="grow" />
          <button className="btn primary" disabled={!selected} onClick={() => selected && api.restoreVersion(selected.id).then(onRestored).catch(fail)}>
            Restore this version
          </button>
        </>
      }
    >
      {versions.length === 0 ? (
        <Empty title="No earlier versions yet">
          <span className="small">Versions are saved automatically as you edit (at most every 5 minutes).</span>
        </Empty>
      ) : (
        <div className="history-list">
          <div className="versions list">
            {versions.map((v) => (
              <div key={v.id} className={'list-item' + (v.id === selected?.id ? ' active' : '')} onClick={() => setSelected(v)}>
                <div className="title small">{formatDateTime(v.createdAt)}</div>
                <div className="meta ellipsis">{v.title || 'Untitled'}</div>
              </div>
            ))}
          </div>
          {selected && (
            <div>
              <h3 style={{ margin: '0 16px 10px' }}>{selected.title || 'Untitled'}</h3>
              <pre>{selected.body}</pre>
            </div>
          )}
        </div>
      )}
    </Modal>
  )
}
