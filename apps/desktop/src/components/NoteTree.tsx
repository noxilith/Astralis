import { useMemo, useState, type DragEvent, type MouseEvent, type ReactNode } from 'react'
import { Archive, ChevronDown, ChevronRight, Copy, CornerLeftUp, ExternalLink, Folder as FolderIcon, FolderOpen, FolderPlus, MoreHorizontal, Pencil, PenTool, Pin, PinOff, StickyNote, Trash2 } from 'lucide-react'
import type { Folder, Id, Note } from '../api'
import { useCollab, type Presence } from '../lib/collab'

export const noteIcon = (n: Note, size = 14) =>
  n.kind === 'whiteboard' ? <PenTool size={size} /> : <StickyNote size={size} />

type Drag = { type: 'note' | 'folder'; id: Id }

/** What the right-click menu on a note can do. */
export type NoteAction = 'open' | 'rename' | 'pin' | 'duplicate' | 'toTop' | 'archive' | 'delete'

/** Keeps a context menu on screen near the bottom/right edges. */
const menuPos = (x: number, y: number, height: number) => ({
  left: Math.min(x, window.innerWidth - 230),
  top: Math.min(y, window.innerHeight - height - 8),
})

// Key in the expanded-folders set that marks the Pinned section as collapsed (open by default).
export const PINNED_CLOSED = "__pinned_closed"

export interface NoteTreeProps {
  notes: Note[]
  folders: Folder[]
  selectedId: Id | null
  activeFolderId: Id | null
  expanded: Set<Id>
  onToggle: (id: Id) => void
  onSelectNote: (n: Note) => void
  onSelectFolder: (f: Folder) => void
  onMoveNote: (noteId: Id, folderId: Id | null) => void
  onMoveFolder: (folderId: Id, parentId: Id | null) => void
  onNewNote: (folderId: Id, kind: 'note' | 'whiteboard') => void
  onNewFolder: (parentId: Id | null) => void
  onRenameFolder: (f: Folder) => void
  onDeleteFolder: (f: Folder) => void
  /** Dropping a note on the Pinned section pins it. */
  onPinNote?: (noteId: Id) => void
  /** Right-click menu on a note. */
  onNoteAction?: (note: Note, action: NoteAction) => void
}

/** Colored dots for the people editing a note (or something inside a folder). */
function EditingDots({ people, inside }: { people?: Presence[]; inside?: boolean }) {
  if (!people?.length) return null
  const names = people.map((x) => x.name).join(', ')
  return (
    <span className={'editing-dots' + (inside ? ' inside' : '')} title={`${names} ${inside ? 'editing in here' : people.length > 1 ? 'are editing' : 'is editing'}`}>
      {people.slice(0, 3).map((x) => (
        <span key={x.peer} className="editing-dot" style={{ background: x.color }}>
          {x.name.slice(0, 1).toUpperCase()}
        </span>
      ))}
      {people.length > 3 && <span className="editing-more">+{people.length - 3}</span>}
    </span>
  )
}

const byNote = (a: Note, b: Note) =>
  (a.title || 'Untitled').localeCompare(b.title || 'Untitled', undefined, { sensitivity: 'base' })

/** Compact, file-explorer style tree of folders and notes. Drag rows onto folders to move them. */
export function NoteTree(p: NoteTreeProps) {
  const [drag, setDrag] = useState<Drag | null>(null)
  const [dropTarget, setDropTarget] = useState<Id | "root" | "pinned" | null>(null)
  const [menu, setMenu] = useState<{ folder: Folder; x: number; y: number } | null>(null)
  const [noteMenu, setNoteMenu] = useState<{ note: Note; x: number; y: number } | null>(null)
  const noteAction = (a: NoteAction) => {
    if (noteMenu) p.onNoteAction?.(noteMenu.note, a)
    setNoteMenu(null)
  }

  const { subfolders, notesIn } = useMemo(() => {
    const known = new Set(p.folders.map((f) => f.id))
    const subfolders = new Map<Id | null, Folder[]>()
    for (const f of p.folders) {
      const key = f.parentId && known.has(f.parentId) ? f.parentId : null
      if (!subfolders.has(key)) subfolders.set(key, [])
      subfolders.get(key)!.push(f)
    }
    const notesIn = new Map<Id | null, Note[]>()
    for (const n of p.notes) {
      const key = n.folderId && known.has(n.folderId) ? n.folderId : null
      if (!notesIn.has(key)) notesIn.set(key, [])
      notesIn.get(key)!.push(n)
    }
    notesIn.forEach((list) => list.sort(byNote))
    return { subfolders, notesIn }
  }, [p.folders, p.notes])

  // Who's in which note right now — and, for folders, anywhere inside them.
  const { presence } = useCollab()
  const editors = useMemo(() => {
    const byId = new Map<Id, Presence[]>()
    const add = (id: Id, pr: Presence) => {
      const list = byId.get(id) ?? []
      if (!list.some((x) => x.peer === pr.peer)) byId.set(id, [...list, pr])
    }
    const parentOf = new Map(p.folders.map((f) => [f.id, f.parentId]))
    for (const pr of presence) {
      const note = pr.noteId ? p.notes.find((n) => n.id === pr.noteId) : undefined
      if (!note) continue
      add(note.id, pr)
      const seen = new Set<Id>()
      for (let f = note.folderId; f && !seen.has(f); f = parentOf.get(f) ?? null) {
        seen.add(f)
        add(f, pr)
      }
    }
    return byId
  }, [presence, p.notes, p.folders])

  const startDrag = (e: DragEvent, d: Drag) => {
    e.dataTransfer.setData('text/plain', d.id)
    e.dataTransfer.effectAllowed = 'move'
    setDrag(d)
  }
  const endDrag = () => {
    setDrag(null)
    setDropTarget(null)
  }
  const drop = (e: DragEvent, folderId: Id | null) => {
    e.preventDefault()
    e.stopPropagation()
    if (drag?.type === 'note') p.onMoveNote(drag.id, folderId)
    else if (drag?.type === 'folder' && drag.id !== folderId) p.onMoveFolder(drag.id, folderId)
    endDrag()
  }
  const dropProps = (target: Id | 'root') => ({
    onDragOver: (e: DragEvent) => {
      if (!drag) return
      e.preventDefault()
      e.stopPropagation()
      setDropTarget(target)
    },
    onDrop: (e: DragEvent) => drop(e, target === 'root' ? null : target),
  })

  const openMenu = (e: MouseEvent, folder: Folder) => {
    e.preventDefault()
    e.stopPropagation()
    setNoteMenu(null)
    setMenu({ folder, x: e.clientX, y: e.clientY })
  }

  const renderFolder = (f: Folder, depth: number): ReactNode => {
    const open = p.expanded.has(f.id)
    return (
      <div key={f.id}>
        <div
          className={'tree-row' + (dropTarget === f.id ? ' drop' : '') + (p.activeFolderId === f.id && !p.selectedId ? ' active' : '')}
          style={{ paddingLeft: 6 + depth * 14 }}
          draggable
          onDragStart={(e) => startDrag(e, { type: 'folder', id: f.id })}
          onDragEnd={endDrag}
          {...dropProps(f.id)}
          onClick={() => {
            p.onToggle(f.id)
            p.onSelectFolder(f)
          }}
          onContextMenu={(e) => openMenu(e, f)}
        >
          <span className="tree-caret">{open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</span>
          <span className="tree-icon folder">{open ? <FolderOpen size={14} /> : <FolderIcon size={14} />}</span>
          <span className="grow ellipsis">{f.name}</span>
          <EditingDots people={editors.get(f.id)} inside />
          <button className="tree-action" aria-label="Folder actions" onClick={(e) => openMenu(e, f)}>
            <MoreHorizontal size={13} />
          </button>
        </div>
        {open && renderChildren(f.id, depth + 1)}
      </div>
    )
  }

  const renderNote = (n: Note, depth: number, keyPrefix = "") => (
    <div
      key={keyPrefix + n.id}
      className={'tree-row' + (n.id === p.selectedId ? ' active' : '')}
      style={{ paddingLeft: 6 + depth * 14 + 17 }}
      draggable
      onDragStart={(e) => startDrag(e, { type: 'note', id: n.id })}
      onDragEnd={endDrag}
      onClick={() => p.onSelectNote(n)}
      onContextMenu={(e) => {
        if (!p.onNoteAction) return
        e.preventDefault()
        e.stopPropagation()
        setMenu(null)
        setNoteMenu({ note: n, x: e.clientX, y: e.clientY })
      }}
      title={n.title || 'Untitled'}
    >
      <span className="tree-icon">{noteIcon(n)}</span>
      <span className="grow ellipsis">{n.title || 'Untitled'}</span>
      <EditingDots people={editors.get(n.id)} />
      {n.pinned && !keyPrefix && <Pin size={11} className="faint" />}
    </div>
  )

  /** "Pinned" sits at the top of the vault: every pinned note, wherever it's filed. */
  const pinned = useMemo(() => p.notes.filter((n) => n.pinned).sort(byNote), [p.notes])
  const renderPinned = () => {
    if (!pinned.length && dropTarget !== 'pinned' && !(drag?.type === 'note')) return null
    const open = !p.expanded.has(PINNED_CLOSED)
    return (
      <div className="tree-pinned">
        <div
          className={'tree-row tree-section' + (dropTarget === 'pinned' ? ' drop' : '')}
          style={{ paddingLeft: 6 }}
          onClick={() => p.onToggle(PINNED_CLOSED)}
          onDragOver={(e) => {
            if (drag?.type !== 'note') return
            e.preventDefault()
            e.stopPropagation()
            setDropTarget('pinned')
          }}
          onDrop={(e) => {
            e.preventDefault()
            e.stopPropagation()
            if (drag?.type === 'note') p.onPinNote?.(drag.id)
            endDrag()
          }}
          title={pinned.length ? undefined : 'Drop a note here to pin it'}
        >
          <span className="tree-caret">{open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</span>
          <span className="tree-icon pin"><Pin size={14} /></span>
          <span className="grow">Pinned</span>
          <span className="small faint">{pinned.length || ''}</span>
        </div>
        {open && pinned.map((n) => renderNote(n, 1, 'pin:'))}
      </div>
    )
  }

  const renderChildren = (parent: Id | null, depth: number): ReactNode => (
    <>
      {(subfolders.get(parent) ?? []).map((f) => renderFolder(f, depth))}
      {(notesIn.get(parent) ?? []).map((n) => renderNote(n, depth))}
    </>
  )

  return (
    <div className={'tree' + (dropTarget === 'root' ? ' drop-root' : '')} {...dropProps('root')}>
      {renderPinned()}
      {renderChildren(null, 0)}
      {drag && <div className="tree-root-hint">Drop here to move to the top level</div>}
      {noteMenu && (
        <>
          <div className="menu-backdrop" onClick={() => setNoteMenu(null)} onContextMenu={(e) => (e.preventDefault(), setNoteMenu(null))} />
          <div className="menu" style={{ position: 'fixed', ...menuPos(noteMenu.x, noteMenu.y, 300), right: 'auto' }}>
            <button onClick={() => noteAction('open')}>
              <ExternalLink size={14} /> Open
            </button>
            <button onClick={() => noteAction('rename')}>
              <Pencil size={14} /> Rename
            </button>
            <button onClick={() => noteAction('pin')}>
              {noteMenu.note.pinned ? <PinOff size={14} /> : <Pin size={14} />} {noteMenu.note.pinned ? 'Unpin' : 'Pin to top'}
            </button>
            <button onClick={() => noteAction('duplicate')}>
              <Copy size={14} /> Duplicate
            </button>
            {noteMenu.note.folderId && (
              <button onClick={() => noteAction('toTop')}>
                <CornerLeftUp size={14} /> Move to top level
              </button>
            )}
            <div className="menu-sep" />
            <button onClick={() => noteAction('archive')}>
              <Archive size={14} /> Archive
            </button>
            <button className="danger" onClick={() => noteAction('delete')}>
              <Trash2 size={14} /> Delete permanently
            </button>
          </div>
        </>
      )}
      {menu && (
        <>
          <div className="menu-backdrop" onClick={() => setMenu(null)} onContextMenu={(e) => (e.preventDefault(), setMenu(null))} />
          <div className="menu" style={{ position: 'fixed', ...menuPos(menu.x, menu.y, 210), right: 'auto' }}>
            <button onClick={() => (setMenu(null), p.onNewNote(menu.folder.id, 'note'))}>
              <StickyNote size={14} /> New note here
            </button>
            <button onClick={() => (setMenu(null), p.onNewNote(menu.folder.id, 'whiteboard'))}>
              <PenTool size={14} /> New whiteboard here
            </button>
            <button onClick={() => (setMenu(null), p.onNewFolder(menu.folder.id))}>
              <FolderPlus size={14} /> New subfolder
            </button>
            <div className="menu-sep" />
            <button onClick={() => (setMenu(null), p.onRenameFolder(menu.folder))}>
              <Pencil size={14} /> Rename
            </button>
            <button className="danger" onClick={() => (setMenu(null), p.onDeleteFolder(menu.folder))}>
              <Trash2 size={14} /> Delete folder
            </button>
          </div>
        </>
      )}
    </div>
  )
}

/** Flat compact rows (search results, archive), with the folder path as a hint. */
export function NoteRows({ notes, folders, selectedId, onSelect }: { notes: Note[]; folders: Folder[]; selectedId: Id | null; onSelect: (n: Note) => void }) {
  const path = useMemo(() => {
    const byId = new Map(folders.map((f) => [f.id, f]))
    return (id: Id | null) => {
      const parts: string[] = []
      let cur = id ? byId.get(id) : undefined
      let guard = 0
      while (cur && guard++ < 64) {
        parts.unshift(cur.name)
        cur = cur.parentId ? byId.get(cur.parentId) : undefined
      }
      return parts.join(' / ')
    }
  }, [folders])
  const { presence } = useCollab()
  return (
    <div className="tree">
      {notes.map((n) => (
        <div key={n.id} className={'tree-row' + (n.id === selectedId ? ' active' : '')} style={{ paddingLeft: 10 }} onClick={() => onSelect(n)}>
          <span className="tree-icon">{noteIcon(n)}</span>
          <span className="ellipsis">{n.title || 'Untitled'}</span>
          <span className="grow ellipsis small faint" style={{ textAlign: 'right' }}>{path(n.folderId)}</span>
          <EditingDots people={presence.filter((x) => x.noteId === n.id)} />
        </div>
      ))}
    </div>
  )
}
