import { useEffect, useState } from 'react'
import { Merge, Sparkles, Tag } from 'lucide-react'
import { api, type Cluster, type DuplicatePair } from '../api'
import { useApp } from '../context'
import { Empty, Modal, Spinner } from './ui'

/** Groups the vault's notes by topic and offers a tag for each group. */
export function OrganizeModal({ onClose }: { onClose: () => void }) {
  const { vaultId, openNote, bump, fail, toast } = useApp()
  const [clusters, setClusters] = useState<Cluster[] | null>(null)

  useEffect(() => {
    api.organizeNotes(vaultId).then(setClusters).catch((e) => (fail(e), onClose()))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [vaultId])

  const applyTag = async (c: Cluster) => {
    try {
      for (const n of c.notes) await api.addTag(n.id, c.suggestedTag)
      toast(`Tagged ${c.notes.length} notes #${c.suggestedTag}`)
      bump()
    } catch (e) {
      fail(e)
    }
  }

  return (
    <Modal title="Organize notes by topic" wide onClose={onClose}>
      <p className="small muted" style={{ marginTop: 0 }}>
        Your notes grouped by what they're about, with a suggested tag for each group. Runs on this device.
      </p>
      {!clusters ? (
        <Empty title="Reading your notes…"><Spinner /></Empty>
      ) : clusters.length === 0 ? (
        <Empty icon={<Sparkles size={26} />} title="Add at least 4 notes to organize" />
      ) : (
        <div className="cards" style={{ padding: 0 }}>
          {clusters.map((c, i) => (
            <div key={i} className="tile">
              <h4>{c.label}</h4>
              <div className="list" style={{ maxHeight: 180, overflow: 'auto' }}>
                {c.notes.map((n) => (
                  <div
                    key={n.id}
                    className="link-row"
                    onClick={() => {
                      onClose()
                      openNote(n.id)
                    }}
                  >
                    <span className="ellipsis small">{n.title || 'Untitled'}</span>
                  </div>
                ))}
              </div>
              {c.suggestedTag && (
                <button className="btn sm" onClick={() => applyTag(c)}>
                  <Tag size={12} /> Tag all #{c.suggestedTag}
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </Modal>
  )
}

/** Finds near-identical notes in the vault and merges them on request. */
export function DuplicatesModal({ onClose }: { onClose: () => void }) {
  const { vaultId, openNote, bump, fail, toast } = useApp()
  const [threshold, setThreshold] = useState(0.85)
  const [dups, setDups] = useState<DuplicatePair[] | null>(null)

  const search = (t: number) => {
    setDups(null)
    api.findDuplicates(vaultId, t).then(setDups).catch((e) => (fail(e), onClose()))
  }
  useEffect(() => {
    search(threshold)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [vaultId])

  const merge = async (d: DuplicatePair) => {
    try {
      const n = await api.mergeNotes([d.a.id, d.b.id])
      setDups((ds) => ds?.filter((x) => x !== d) ?? null)
      bump()
      toast('Merged — the originals are in the archive', { actions: [{ label: 'Open', run: () => openNote(n.id) }] })
    } catch (e) {
      fail(e)
    }
  }

  return (
    <Modal
      title="Find duplicate notes"
      wide
      onClose={onClose}
      footer={
        <label className="small muted row grow">
          Sensitivity
          <input type="range" min={0.6} max={0.98} step={0.01} value={threshold} onChange={(e) => setThreshold(Number(e.target.value))} onPointerUp={() => search(threshold)} onKeyUp={() => search(threshold)} />
          <span className="score">{Math.round(threshold * 100)}%</span>
        </label>
      }
    >
      {!dups ? (
        <Empty title="Comparing notes…"><Spinner /></Empty>
      ) : dups.length === 0 ? (
        <Empty icon={<Sparkles size={26} />} title="No duplicates found" />
      ) : (
        dups.map((d) => (
          <div key={d.a.id + d.b.id} className="row" style={{ padding: '8px 0', borderBottom: '1px solid var(--border)' }}>
            <span className="score" style={{ width: 44 }}>{Math.round(d.score * 100)}%</span>
            <a className="grow ellipsis" style={{ cursor: 'pointer' }} onClick={() => (onClose(), openNote(d.a.id))}>{d.a.title || 'Untitled'}</a>
            <span className="faint">≈</span>
            <a className="grow ellipsis" style={{ cursor: 'pointer' }} onClick={() => (onClose(), openNote(d.b.id))}>{d.b.title || 'Untitled'}</a>
            <button className="btn sm" onClick={() => merge(d)}><Merge size={12} /> Merge</button>
          </div>
        ))
      )}
    </Modal>
  )
}
