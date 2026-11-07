import { useState } from 'react'
import { Brain, ChevronDown, Link2, Plus, Sparkles, Tags, Wand2 } from 'lucide-react'
import { api, type Note, type RelatedNote, type Summary, type TagSuggestion } from '../api'
import { useApp } from '../context'
import { Empty, Modal, Spinner } from './ui'

type Panel =
  | { kind: 'summary'; data: Summary | null; sentences: number }
  | { kind: 'tags'; data: TagSuggestion[] | null }
  | { kind: 'related'; data: RelatedNote[] | null }

/** The ✨ AI dropdown in the editor toolbar: Tidy up, Summarize, Suggest tags, Related notes. */
export function AiMenu({
  note,
  readOnly,
  onTidy,
  onChange,
}: {
  note: Note
  readOnly: boolean
  onTidy: () => void
  /** Called with the updated note; `remount` reloads the editor (after body changes). */
  onChange: (n: Note, remount?: boolean) => void
}) {
  const { openNote, fail, bump } = useApp()
  const [open, setOpen] = useState(false)
  const [panel, setPanel] = useState<Panel | null>(null)
  const plainVault = !note.encrypted

  const summarize = async (sentences: number) => {
    setPanel({ kind: 'summary', data: null, sentences })
    try {
      const data = await api.summarizeNote(note.id, sentences)
      setPanel({ kind: 'summary', data, sentences })
    } catch (e) {
      setPanel(null)
      fail(e)
    }
  }
  const suggestTags = async () => {
    setPanel({ kind: 'tags', data: null })
    try {
      setPanel({ kind: 'tags', data: await api.suggestTags(note.id) })
    } catch (e) {
      setPanel(null)
      fail(e)
    }
  }
  const related = async () => {
    setPanel({ kind: 'related', data: null })
    try {
      setPanel({ kind: 'related', data: await api.relatedNotes(note.id) })
    } catch (e) {
      setPanel(null)
      fail(e)
    }
  }

  const addTags = async (tags: string[]) => {
    try {
      let n = note
      for (const t of tags) n = await api.addTag(note.id, t)
      onChange(n)
      bump()
      setPanel((p) => (p?.kind === 'tags' ? { ...p, data: p.data?.filter((s) => !tags.includes(s.tag)) ?? null } : p))
    } catch (e) {
      fail(e)
    }
  }

  const item = (label: string, hint: string, icon: React.ReactNode, run: () => void) => (
    <button
      onClick={() => {
        setOpen(false)
        run()
      }}
    >
      {icon}
      <span className="grow">
        {label}
        <span className="menu-hint">{hint}</span>
      </span>
    </button>
  )

  return (
    <div style={{ position: 'relative' }}>
      <button className="btn ghost sm ai-btn" onClick={() => setOpen((o) => !o)} title="AI tools (run on this device)">
        <Sparkles size={14} /> AI <ChevronDown size={13} />
      </button>
      {open && (
        <>
          <div className="menu-backdrop" onClick={() => setOpen(false)} />
          <div className="menu ai-menu">
            {!readOnly && item('Tidy up', 'Clean formatting, organize into sections', <Wand2 size={14} />, onTidy)}
            {item('Summarize', 'The key sentences of this note', <Sparkles size={14} />, () => summarize(3))}
            {plainVault && !readOnly && item('Suggest tags', 'Tags from its content and similar notes', <Tags size={14} />, suggestTags)}
            {plainVault && item('Related notes', 'Notes about the same things', <Brain size={14} />, related)}
          </div>
        </>
      )}

      {panel?.kind === 'summary' && (
        <Modal
          title="Summary"
          onClose={() => setPanel(null)}
          footer={
            <>
              <label className="small muted row grow">
                Length
                <select className="select" style={{ width: 120 }} value={panel.sentences} onChange={(e) => summarize(Number(e.target.value))}>
                  {[1, 2, 3, 4, 5].map((n) => (
                    <option key={n} value={n}>{n} sentence{n > 1 ? 's' : ''}</option>
                  ))}
                </select>
              </label>
              <button className="btn" onClick={() => panel.data && navigator.clipboard?.writeText(panel.data.text)} disabled={!panel.data?.text}>
                Copy
              </button>
              {!readOnly && (
                <button
                  className="btn primary"
                  disabled={!panel.data?.text}
                  onClick={async () => {
                    if (!panel.data) return
                    try {
                      onChange(await api.updateNote(note.id, { body: `> **Summary:** ${panel.data.text}\n\n${note.body}` }), true)
                      setPanel(null)
                    } catch (e) {
                      fail(e)
                    }
                  }}
                >
                  Insert at top
                </button>
              )}
            </>
          }
        >
          {!panel.data ? (
            <Empty title="Reading the note…"><Spinner /></Empty>
          ) : panel.data.text ? (
            <div className="summary" style={{ fontSize: 14, lineHeight: 1.7 }}>{panel.data.text}</div>
          ) : (
            <Empty title="Not enough text to summarize" />
          )}
        </Modal>
      )}

      {panel?.kind === 'tags' && (
        <Modal
          title="Suggested tags"
          onClose={() => setPanel(null)}
          footer={
            <>
              <button className="btn" onClick={() => setPanel(null)}>Done</button>
              <button className="btn primary" disabled={!panel.data?.length} onClick={() => panel.data && addTags(panel.data.map((s) => s.tag))}>
                Add all
              </button>
            </>
          }
        >
          {!panel.data ? (
            <Empty title="Looking for tags…"><Spinner /></Empty>
          ) : panel.data.length === 0 ? (
            <Empty title="No more suggestions" />
          ) : (
            <>
              <p className="small muted" style={{ marginTop: 0 }}>Click a tag to add it.</p>
              <div className="row wrap">
                {panel.data.map((s) => (
                  <span key={s.tag} className="chip suggest" title={s.source === 'similar' ? 'Used on similar notes' : 'Key phrase in this note'} onClick={() => addTags([s.tag])}>
                    <Plus size={11} /> {s.tag}
                  </span>
                ))}
              </div>
            </>
          )}
          {note.tags.length > 0 && (
            <p className="small faint" style={{ marginBottom: 0 }}>Already tagged: {note.tags.map((t) => `#${t}`).join(' ')}</p>
          )}
        </Modal>
      )}

      {panel?.kind === 'related' && (
        <Modal title="Related notes" onClose={() => setPanel(null)}>
          {!panel.data ? (
            <Empty title="Finding related notes…"><Spinner /></Empty>
          ) : panel.data.length === 0 ? (
            <Empty title="Nothing similar yet" />
          ) : (
            panel.data.map((r) => (
              <div key={r.note.id} className="row" style={{ padding: '7px 0', borderBottom: '1px solid var(--border)' }}>
                <span className="score" style={{ width: 40 }}>{Math.round(r.score * 100)}%</span>
                <a
                  className="grow ellipsis"
                  style={{ cursor: 'pointer' }}
                  onClick={() => {
                    setPanel(null)
                    openNote(r.note.id)
                  }}
                >
                  {r.note.title || 'Untitled'}
                </a>
                {r.linked ? (
                  <span className="small faint">Linked</span>
                ) : (
                  !readOnly && (
                    <button
                      className="btn sm"
                      onClick={async () => {
                        try {
                          onChange(await api.updateNote(note.id, { body: `${note.body.trimEnd()}\n\nRelated: [[${r.note.title}]]\n` }), true)
                          setPanel((p) => (p?.kind === 'related' ? { ...p, data: p.data?.map((x) => (x.note.id === r.note.id ? { ...x, linked: true } : x)) ?? null } : p))
                        } catch (e) {
                          fail(e)
                        }
                      }}
                    >
                      <Link2 size={12} /> Link
                    </button>
                  )
                )}
              </div>
            ))
          )}
        </Modal>
      )}
    </div>
  )
}
