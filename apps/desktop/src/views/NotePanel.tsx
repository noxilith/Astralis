import { useEffect, useState } from 'react'
import { ArrowUpRight, CheckCircle2, Circle, Link2, Tag, X } from 'lucide-react'
import { api, type Backlink, type Note, type OutgoingLink, type Task } from '../api'
import { useApp } from '../context'

/** Side panel for a note: tags, links, backlinks and linked tasks. (AI tools live in the ✨ AI menu.) */
export function NotePanel({
  note,
  readOnly,
  onChange,
  onOpenLink,
}: {
  note: Note
  readOnly: boolean
  onChange: (n: Note, remountEditor?: boolean) => void
  onOpenLink: (title: string) => void
}) {
  const { openNote, openTask, fail, version, bump } = useApp()
  const [backlinks, setBacklinks] = useState<Backlink[]>([])
  const [outgoing, setOutgoing] = useState<OutgoingLink[]>([])
  const [tasks, setTasks] = useState<Task[]>([])
  const [tagInput, setTagInput] = useState('')
  const [taskInput, setTaskInput] = useState('')

  // Links and linked tasks refresh as the note is edited (debounced).
  useEffect(() => {
    const t = setTimeout(() => {
      api.backlinks(note.id).then(setBacklinks).catch(() => {})
      api.outgoingLinks(note.id).then(setOutgoing).catch(() => {})
      api.listTasks({ noteId: note.id, includeDone: true }).then(setTasks).catch(() => {})
    }, 400)
    return () => clearTimeout(t)
  }, [note.id, note.body, note.title, version])

  const addTag = async (tag: string) => {
    if (!tag.trim()) return
    try {
      onChange(await api.addTag(note.id, tag))
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const addTask = async () => {
    if (!taskInput.trim()) return
    try {
      const t = await api.createTask({ title: taskInput, noteId: note.id })
      setTasks((ts) => [...ts, t])
      setTaskInput('')
    } catch (e) {
      fail(e)
    }
  }

  const toggleTask = async (t: Task) => {
    try {
      const u = t.status === 'done' ? await api.reopenTask(t.id) : await api.completeTask(t.id)
      setTasks((ts) => ts.map((x) => (x.id === u.id ? u : x)))
    } catch (e) {
      fail(e)
    }
  }

  return (
    <div>
      <div className="panel-section">
        <h3>
          <Tag size={12} /> Tags
        </h3>
        <div className="row wrap">
          {note.tags.map((t) => (
            <span key={t} className="chip">
              #{t}
              {!readOnly && (
                <button
                  aria-label={`Remove ${t}`}
                  onClick={() => api.removeTag(note.id, t).then((n) => (onChange(n), bump())).catch(fail)}
                >
                  <X size={11} />
                </button>
              )}
            </span>
          ))}
          {!readOnly && (
            <input
              className="input"
              style={{ padding: '2px 8px', width: 110, fontSize: 12 }}
              placeholder="add tag…"
              value={tagInput}
              onChange={(e) => setTagInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  addTag(tagInput)
                  setTagInput('')
                }
              }}
            />
          )}
        </div>
      </div>

      <div className="panel-section">
        <h3>
          <ArrowUpRight size={12} /> Backlinks <span className="faint">{backlinks.length || ''}</span>
        </h3>
        {backlinks.length === 0 && <div className="small faint">No notes link here yet.</div>}
        {backlinks.map((b) => (
          <div key={b.note.id} className="link-row" onClick={() => openNote(b.note.id)}>
            <span className="ellipsis">{b.note.title || 'Untitled'}</span>
            {b.context && <span className="ctx">{b.context}</span>}
          </div>
        ))}
      </div>

      <div className="panel-section">
        <h3>
          <Link2 size={12} /> Links
        </h3>
        {outgoing.length === 0 && <div className="small faint">Type [[ in the editor to link a note.</div>}
        {outgoing.map((l) => (
          <div key={l.title} className="link-row" onClick={() => onOpenLink(l.title)}>
            <span className={'ellipsis' + (l.notes.length ? '' : ' faint')}>
              {l.title} {!l.notes.length && <span className="small">(create)</span>}
            </span>
          </div>
        ))}
      </div>

      <div className="panel-section">
        <h3>
          <CheckCircle2 size={12} /> Tasks
        </h3>
        {tasks.map((t) => (
          <div key={t.id} className="row" style={{ padding: '3px 0' }}>
            <button className="icon-btn" style={{ width: 22, height: 22 }} onClick={() => toggleTask(t)} aria-label="Toggle task">
              {t.status === 'done' ? <CheckCircle2 size={15} color="var(--success)" /> : <Circle size={15} />}
            </button>
            <span
              className={'grow ellipsis small' + (t.status === 'done' ? ' faint' : '')}
              style={{ cursor: 'pointer', textDecoration: t.status === 'done' ? 'line-through' : undefined }}
              onClick={() => openTask(t.id)}
            >
              {t.title}
            </span>
          </div>
        ))}
        {!readOnly && (
          <input
            className="input"
            style={{ marginTop: 6, fontSize: 12.5 }}
            placeholder="Add a task for this note…"
            value={taskInput}
            onChange={(e) => setTaskInput(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && addTask()}
          />
        )}
      </div>
    </div>
  )
}
