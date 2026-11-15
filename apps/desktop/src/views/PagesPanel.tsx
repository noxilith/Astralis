import { useEffect, useMemo, useRef, useState } from 'react'
import { ArrowLeft, ArrowRight, Archive, FilePlus2, FileText, Pencil, Search, Unlink } from 'lucide-react'
import { api, type Note } from '../api'
import { useApp } from '../context'
import { useConfirm } from '../components/ui'
import { relativeTime } from '../lib/time'

/**
 * The Pages tab of a note's side panel, like a wiki's page list: the note itself on top, its pages
 * under it. Pages are ordinary notes that belong to a note; they open in the editor like any other.
 */
export function PagesPanel({
  note,
  readOnly,
  onChange,
}: {
  note: Note
  readOnly: boolean
  onChange: (n: Note, remountEditor?: boolean) => void
}) {
  const { openNote, fail, version, bump } = useApp()
  const confirm = useConfirm()
  const rootId = note.parentId ?? note.id
  const [root, setRoot] = useState<Note | null>(null)
  const [pages, setPages] = useState<Note[]>([])
  const [adding, setAdding] = useState(false)
  const [draft, setDraft] = useState('')
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState('')
  const [filter, setFilter] = useState('')
  const busy = useRef(false)

  // Titles and the list refresh as notes are edited elsewhere (debounced).
  useEffect(() => {
    const t = setTimeout(() => {
      api.getNote(rootId).then(setRoot).catch(() => {})
      api.listPages(rootId).then(setPages).catch(() => {})
    }, 250)
    return () => clearTimeout(t)
  }, [rootId, note.title, note.parentId, version])

  const all = useMemo(() => (root ? [root, ...pages] : pages), [root, pages])
  const index = all.findIndex((p) => p.id === note.id)
  const prev = index > 0 ? all[index - 1] : null
  const next = index >= 0 && index < all.length - 1 ? all[index + 1] : null
  const q = filter.trim().toLowerCase()
  const shown = q ? pages.filter((p) => (p.title || 'Untitled').toLowerCase().includes(q)) : pages

  const create = async () => {
    const title = draft.trim()
    if (!title || !root || busy.current) return
    busy.current = true
    try {
      const p = await api.createNote({ title, vaultId: root.vaultId, folderId: root.folderId, parentId: root.id })
      setDraft('')
      setAdding(false)
      setPages((ps) => [...ps, p])
      bump()
      openNote(p.id)
    } catch (e) {
      fail(e)
    } finally {
      busy.current = false
    }
  }

  const rename = async (p: Note) => {
    const title = renameDraft.trim()
    setRenaming(null)
    if (!title || title === p.title) return
    try {
      const u = await api.updateNote(p.id, { title })
      if (u.id === note.id) onChange(u, false)
      setPages((ps) => ps.map((x) => (x.id === u.id ? u : x)))
      setRoot((r) => (r && r.id === u.id ? u : r))
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const archive = async (p: Note) => {
    if (!(await confirm.ask(`Archive the page “${p.title || 'Untitled'}”? It moves to the archive, and you can restore it from there.`, 'Archive page'))) return
    try {
      await api.archiveNote(p.id)
      setPages((ps) => ps.filter((x) => x.id !== p.id))
      bump()
      if (p.id === note.id) openNote(rootId)
    } catch (e) {
      fail(e)
    }
  }

  const detach = async (p: Note) => {
    try {
      const u = await api.setNoteParent(p.id, null)
      setPages((ps) => ps.filter((x) => x.id !== p.id))
      if (u.id === note.id) onChange(u, false)
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const row = (p: Note, isRoot: boolean) => {
    const current = p.id === note.id
    return (
      <div key={p.id} className={'page-row' + (current ? ' active' : '') + (isRoot ? ' root' : '')}>
        <button className="page-main" onClick={() => !current && openNote(p.id)} onDoubleClick={() => !readOnly && (setRenaming(p.id), setRenameDraft(p.title))}>
          <FileText size={14} />
          {renaming === p.id ? (
            <input
              className="input page-rename"
              autoFocus
              value={renameDraft}
              onClick={(e) => e.stopPropagation()}
              onChange={(e) => setRenameDraft(e.target.value)}
              onBlur={() => rename(p)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') rename(p)
                if (e.key === 'Escape') setRenaming(null)
              }}
            />
          ) : (
            <span className="page-title ellipsis">{p.title || 'Untitled'}</span>
          )}
          {renaming !== p.id && <span className="page-time">{relativeTime(p.updatedAt)}</span>}
        </button>
        {!readOnly && !isRoot && renaming !== p.id && (
          <span className="page-actions">
            <button className="icon-btn" title="Rename" aria-label="Rename page" onClick={() => (setRenaming(p.id), setRenameDraft(p.title))}>
              <Pencil size={13} />
            </button>
            <button className="icon-btn" title="Make it a normal note" aria-label="Make it a normal note" onClick={() => detach(p)}>
              <Unlink size={13} />
            </button>
            <button className="icon-btn" title="Archive page" aria-label="Archive page" onClick={() => archive(p)}>
              <Archive size={13} />
            </button>
          </span>
        )}
      </div>
    )
  }

  return (
    <div className="pages-panel">
      <div className="pages-head">
        <h3>
          <FileText size={12} /> Pages <span className="faint">{pages.length || ''}</span>
        </h3>
        {!readOnly && (
          <button className="btn primary sm" onClick={() => setAdding(true)}>
            <FilePlus2 size={13} /> New page
          </button>
        )}
      </div>

      {adding && (
        <div className="pages-new">
          <input
            className="input"
            autoFocus
            placeholder="Page title, then Enter"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => !draft.trim() && setAdding(false)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') create()
              if (e.key === 'Escape') (setAdding(false), setDraft(''))
            }}
          />
        </div>
      )}

      {pages.length > 6 && (
        <div className="pages-search">
          <Search size={13} />
          <input placeholder="Filter pages" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </div>
      )}

      <div className="pages-list">
        {root && row(root, true)}
        <div className="pages-children">
          {shown.map((p) => row(p, false))}
          {q && shown.length === 0 && <div className="small faint" style={{ padding: '6px 8px' }}>No page matches.</div>}
        </div>
      </div>

      {pages.length === 0 && !adding && (
        <p className="small muted pages-empty">
          Split a long note into pages, like a wiki. Each page is its own note, listed here under this one.
        </p>
      )}

      {(prev || next) && (
        <div className="pages-nav">
          <button className="btn sm" disabled={!prev} onClick={() => prev && openNote(prev.id)} title={prev?.title || 'Untitled'}>
            <ArrowLeft size={13} /> Previous
          </button>
          <button className="btn sm" disabled={!next} onClick={() => next && openNote(next.id)} title={next?.title || 'Untitled'}>
            Next <ArrowRight size={13} />
          </button>
        </div>
      )}
      {confirm.element}
    </div>
  )
}
