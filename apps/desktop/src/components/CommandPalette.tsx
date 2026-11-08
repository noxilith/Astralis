import { Fragment, useEffect, useMemo, useRef, useState } from 'react'
import { api, type SearchHit } from '../api'
import { useApp, type View } from '../context'

interface Action { label: string; hint?: string; run: () => void }

/** Renders an FTS snippet, whose matches are wrapped in \u0002…\u0003 (plain text, never HTML). */
function Snippet({ text }: { text: string }) {
  const parts = text.split(/(\u0002[^\u0003]*\u0003)/)
  return (
    <>
      {parts.map((p, i) =>
        p.startsWith('\u0002') ? <mark key={i}>{p.slice(1, -1)}</mark> : <Fragment key={i}>{p}</Fragment>,
      )}
    </>
  )
}

export function CommandPalette({ onNewNote, onNewWhiteboard }: { onNewNote: () => void; onNewWhiteboard: () => void }) {
  const { setPaletteOpen, openNote, setView, setNoteId, vaultId, setListMode } = useApp()
  const [q, setQ] = useState('')
  const [hits, setHits] = useState<SearchHit[]>([])
  const [sel, setSel] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)
  const close = () => setPaletteOpen(false)

  useEffect(() => {
    const t = setTimeout(() => {
      if (!q.trim()) return setHits([])
      api.search(vaultId, q, true).then(setHits).catch(() => setHits([]))
    }, 120)
    return () => clearTimeout(t)
  }, [q, vaultId])

  const actions = useMemo<Action[]>(() => {
    const go = (view: View, label: string): Action => ({ label: `Go to ${label}`, run: () => (setNoteId(null), setView(view)) })
    const all: Action[] = [
      { label: 'New note', hint: 'Ctrl N', run: onNewNote },
      { label: 'New whiteboard', run: onNewWhiteboard },
      {
        label: q.trim() ? `Quick capture “${q.trim().slice(0, 40)}”` : 'Quick capture',
        run: () => q.trim() && api.quickCapture(vaultId, q).then((n) => openNote(n.id)),
      },
      go('tasks', 'Tasks'),
      go('calendar', 'Calendar'),
      go('planners', 'Planners'),
      go('graph', 'Graph'),
      go('brain', 'Brain'),
      go('templates', 'Templates'),
      { label: 'Go to Archive', run: () => (setNoteId(null), setListMode({ kind: 'archive' }), setView('notes')) },
      go('settings', 'Settings'),
    ]
    const needle = q.trim().toLowerCase()
    return needle ? all.filter((a, i) => i < 3 || a.label.toLowerCase().includes(needle)) : all
  }, [q, onNewNote, onNewWhiteboard, openNote, setView, setNoteId, vaultId, setListMode])

  const items: ({ kind: 'hit'; hit: SearchHit } | { kind: 'action'; action: Action })[] = [
    ...hits.map((hit) => ({ kind: 'hit' as const, hit })),
    ...actions.map((action) => ({ kind: 'action' as const, action })),
  ]

  useEffect(() => setSel(0), [q])
  useEffect(() => {
    listRef.current?.querySelector('.selected')?.scrollIntoView({ block: 'nearest' })
  }, [sel])

  const choose = (i: number) => {
    const it = items[i]
    if (!it) return
    close()
    if (it.kind === 'hit') {
      setListMode(it.hit.archived ? { kind: 'archive' } : { kind: 'tree' })
      openNote(it.hit.id)
    } else it.action.run()
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="modal palette">
        <input
          autoFocus
          placeholder="Search notes or type a command…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') close()
            else if (e.key === 'ArrowDown') (e.preventDefault(), setSel((s) => Math.min(s + 1, items.length - 1)))
            else if (e.key === 'ArrowUp') (e.preventDefault(), setSel((s) => Math.max(s - 1, 0)))
            else if (e.key === 'Enter') choose(sel)
          }}
        />
        <div className="palette-list" ref={listRef}>
          {hits.length > 0 && <div className="palette-group">Notes</div>}
          {items.map((it, i) => (
            <Fragment key={i}>
              {it.kind === 'action' && i === hits.length && <div className="palette-group">Commands</div>}
              <div className={'palette-item' + (i === sel ? ' selected' : '')} onMouseEnter={() => setSel(i)} onClick={() => choose(i)}>
                {it.kind === 'hit' ? (
                  <>
                    <span>{it.hit.title || 'Untitled'} {it.hit.archived && <span className="small faint">(archived)</span>}</span>
                    {it.hit.snippet && <span className="snippet"><Snippet text={it.hit.snippet} /></span>}
                  </>
                ) : (
                  <span className="row">
                    <span className="grow">{it.action.label}</span>
                    {it.action.hint && <span className="kbd">{it.action.hint}</span>}
                  </span>
                )}
              </div>
            </Fragment>
          ))}
        </div>
      </div>
    </div>
  )
}
