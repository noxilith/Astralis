import { useCallback, useEffect, useMemo, useState } from 'react'
import { ChevronDown, ChevronRight, FilePlus2, FolderPlus, Hash, Lock, PenTool, Search, StickyNote, X } from 'lucide-react'
import { api, type Folder, type Id, type Note, type TagCount, type Template } from '../api'
import { useApp } from '../context'
import { NoteRows, NoteTree, type NoteAction } from './NoteTree'
import { SYNCED } from '../lib/collab'
import { VaultSwitcher } from './VaultSwitcher'
import { Empty, Modal, useConfirm, usePrompt } from './ui'

const expandedKey = (vault: Id) => `astralis.expanded.${vault}`
const loadExpanded = (vault: Id | null): Set<Id> => {
  if (!vault) return new Set()
  try {
    return new Set(JSON.parse(localStorage.getItem(expandedKey(vault)) ?? '[]'))
  } catch {
    return new Set()
  }
}

/** "Work / Projects / Apollo" for a folder id. */
export const folderPath = (folders: Folder[], id: Id | null) => {
  const byId = new Map(folders.map((f) => [f.id, f]))
  const parts: string[] = []
  let cur = id ? byId.get(id) : undefined
  let guard = 0
  while (cur && guard++ < 64) {
    parts.unshift(cur.name)
    cur = cur.parentId ? byId.get(cur.parentId) : undefined
  }
  return parts.join(' / ')
}

const plain = (s: string) => s.replace(/```[\s\S]*?```/g, ' ').replace(/[#>*_`~[\]|-]/g, ' ').replace(/\s+/g, ' ').trim()

/** The notes side of the sidebar: vault switcher, New ▾, filter, folder tree and tags. */
export function NotesNav() {
  const app = useApp()
  const { vaultId, vaults, noteId, openNote, activeFolderId, setActiveFolderId, listMode, setListMode, createNote, lastSaved, version, fail } = app
  const vault = vaults.find((v) => v.id === vaultId) ?? null
  const locked = !!vault && vault.encrypted && !vault.unlocked
  const [notes, setNotes] = useState<Note[]>([])
  const [folders, setFolders] = useState<Folder[]>([])
  const [tags, setTags] = useState<TagCount[]>([])
  const [tagsOpen, setTagsOpen] = useState(false)
  const [expanded, setExpanded] = useState<Set<Id>>(() => loadExpanded(vaultId))
  const [filter, setFilter] = useState('')
  const [newMenuOpen, setNewMenuOpen] = useState(false)
  const [templates, setTemplates] = useState<Template[] | null>(null)
  const prompt = usePrompt()
  const confirm = useConfirm()

  const load = useCallback(async () => {
    if (!vaultId || locked) {
      setNotes([])
      setFolders([])
      setTags([])
      return
    }
    try {
      const [list, fs, ts] = await Promise.all([
        api.listNotes({
          vaultId,
          archive: listMode.kind === 'archive' ? 'archived' : 'active',
          tag: listMode.kind === 'tag' ? listMode.tag : null,
        }),
        api.listFolders(vaultId),
        api.listTags(vaultId),
      ])
      setNotes(list)
      setFolders(fs)
      setTags(ts)
    } catch (e) {
      fail(e)
    }
  }, [vaultId, locked, listMode, fail])

  useEffect(() => {
    load()
  }, [load, version])

  // Keep titles/pins in the list current as the editor saves.
  useEffect(() => {
    if (lastSaved) setNotes((list) => list.map((n) => (n.id === lastSaved.id ? { ...n, ...lastSaved } : n)))
  }, [lastSaved])

  useEffect(() => setExpanded(loadExpanded(vaultId)), [vaultId])

  const saveExpanded = (next: Set<Id>) => {
    setExpanded(next)
    if (vaultId)
      try {
        localStorage.setItem(expandedKey(vaultId), JSON.stringify([...next]))
      } catch {
        /* storage unavailable: expansion just isn't remembered */
      }
  }

  // Selecting a note makes its folder active and expands the folders above it.
  useEffect(() => {
    const n = notes.find((x) => x.id === noteId)
    if (!n) return
    setActiveFolderId(n.folderId)
    const byId = new Map(folders.map((f) => [f.id, f]))
    const next = new Set(expanded)
    let changed = false
    for (let cur = n.folderId ? byId.get(n.folderId) : undefined; cur; cur = cur.parentId ? byId.get(cur.parentId) : undefined) {
      if (!next.has(cur.id)) {
        next.add(cur.id)
        changed = true
      }
    }
    if (changed) saveExpanded(next)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [noteId, notes.length, folders.length])

  const folderAction = async (fn: () => Promise<unknown>) => {
    try {
      await fn()
      await load()
    } catch (e) {
      fail(e)
    }
  }

  /** Tell an open editor its note changed here (it reloads the title, pin, archive state). */
  const touched = (id: Id) => window.dispatchEvent(new CustomEvent(SYNCED, { detail: [id] }))

  const noteAction = async (n: Note, action: NoteAction) => {
    const name = n.title || 'Untitled'
    switch (action) {
      case 'open':
        return openNote(n.id)
      case 'rename': {
        const title = await prompt.ask(n.kind === 'whiteboard' ? 'Rename whiteboard' : 'Rename note', 'Title', n.title)
        if (title?.trim() && title !== n.title) folderAction(() => api.updateNote(n.id, { title }).then((x) => (app.publishNote(x), touched(n.id))))
        return
      }
      case 'pin':
        return folderAction(() => api.setPinned(n.id, !n.pinned).then((x) => (app.publishNote(x), touched(n.id))))
      case 'duplicate':
        return folderAction(async () => {
          const full = await api.getNote(n.id)
          const copy = await api.createNote({
            kind: full.kind,
            title: `${full.title || 'Untitled'} (copy)`,
            body: full.body,
            tags: full.tags,
            vaultId: full.vaultId,
            folderId: full.folderId,
          })
          openNote(copy.id)
        })
      case 'toTop':
        return folderAction(() => api.moveNoteToFolder(n.id, null).then(app.publishNote))
      case 'archive':
        return folderAction(async () => {
          await api.archiveNote(n.id)
          if (noteId === n.id) touched(n.id)
          app.toast(`Archived “${name}”`, { actions: [{ label: 'Undo', run: () => folderAction(() => api.unarchiveNote(n.id).then(() => touched(n.id))) }] })
        })
      case 'delete':
        if (await confirm.ask(`Permanently delete “${name}”${n.kind === 'whiteboard' ? '' : ' and its history'}? This can't be undone.`, 'Delete forever')) {
          folderAction(async () => {
            await api.deleteNote(n.id)
            if (noteId === n.id) app.setNoteId(null)
          })
        }
        return
    }
  }

  const newFolder = async (parentId: Id | null) => {
    setNewMenuOpen(false)
    const name = await prompt.ask('New folder', parentId ? `Inside “${folderPath(folders, parentId)}”` : 'Folder name')
    if (!name?.trim()) return
    folderAction(async () => {
      const f = await api.createFolder(vaultId, parentId, name)
      const next = new Set(expanded)
      if (parentId) next.add(parentId)
      next.add(f.id)
      saveExpanded(next)
      setActiveFolderId(f.id)
    })
  }

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase()
    if (!q) return notes
    return notes.filter((n) => n.title.toLowerCase().includes(q) || (n.kind === 'note' && n.body.toLowerCase().includes(q)))
  }, [notes, filter])

  const flat = !!filter.trim() || listMode.kind !== 'tree'

  return (
    <div className="notes-nav">
      <div className="notes-nav-head">
        <div className="row">
          <VaultSwitcher />
          {!locked && (
            <div className="split-btn">
              <button className="btn primary sm" onClick={() => createNote('note')} title="New note (Ctrl+N)">
                New
              </button>
              <button className="btn primary sm caret" onClick={() => setNewMenuOpen((o) => !o)} aria-label="More new options">
                <ChevronDown size={14} />
              </button>
              {newMenuOpen && (
                <>
                  <div className="menu-backdrop" onClick={() => setNewMenuOpen(false)} />
                  <div className="menu">
                    <button onClick={() => (setNewMenuOpen(false), createNote('note'))}>
                      <StickyNote size={14} /> Note
                    </button>
                    <button onClick={() => (setNewMenuOpen(false), createNote('whiteboard'))}>
                      <PenTool size={14} /> Whiteboard
                    </button>
                    <button onClick={() => newFolder(activeFolderId)}>
                      <FolderPlus size={14} /> Folder
                    </button>
                    <div className="menu-sep" />
                    <button onClick={() => (setNewMenuOpen(false), api.listTemplates().then(setTemplates).catch(fail))}>
                      <FilePlus2 size={14} /> From template…
                    </button>
                  </div>
                </>
              )}
            </div>
          )}
        </div>
        {!locked && (
          <div className="row">
            <div className="search-box grow">
              <Search size={13} className="faint" />
              <input placeholder="Filter notes…" value={filter} onChange={(e) => setFilter(e.target.value)} />
            </div>
            <button className="icon-btn" title={activeFolderId ? 'New subfolder' : 'New folder'} onClick={() => newFolder(activeFolderId)}>
              <FolderPlus size={16} />
            </button>
          </div>
        )}
        {listMode.kind !== 'tree' && (
          <div className="mode-chip">
            {listMode.kind === 'archive' ? 'Archive' : `#${listMode.tag}`}
            <button aria-label="Back to all notes" onClick={() => setListMode({ kind: 'tree' })}>
              <X size={12} />
            </button>
          </div>
        )}
      </div>

      <div className="notes-nav-body">
        {locked ? (
          <Empty icon={<Lock size={26} />} title="Vault locked" />
        ) : flat ? (
          <NoteRows notes={visible} folders={folders} selectedId={noteId} onSelect={(n) => openNote(n.id)} />
        ) : (
          <NoteTree
            notes={notes}
            folders={folders}
            selectedId={noteId}
            activeFolderId={activeFolderId}
            expanded={expanded}
            onToggle={(id) => {
              const next = new Set(expanded)
              if (next.has(id)) next.delete(id)
              else next.add(id)
              saveExpanded(next)
            }}
            onSelectNote={(n) => openNote(n.id)}
            onSelectFolder={(f) => setActiveFolderId(f.id)}
            onMoveNote={(id, folderId) => folderAction(() => api.moveNoteToFolder(id, folderId).then(app.publishNote))}
            onMoveFolder={(id, parentId) => folderAction(() => api.moveFolder(id, parentId))}
            onNewNote={(folderId, kind) => createNote(kind, { folderId })}
            onNewFolder={newFolder}
            onRenameFolder={async (f) => {
              const name = await prompt.ask('Rename folder', 'Name', f.name)
              if (name?.trim()) folderAction(() => api.renameFolder(f.id, name))
            }}
            onPinNote={(id) => folderAction(() => api.setPinned(id, true).then(app.publishNote))}
            onNoteAction={noteAction}
            onDeleteFolder={async (f) => {
              const parent = f.parentId ? `“${folderPath(folders, f.parentId)}”` : 'the top level'
              if (await confirm.ask(`Delete folder “${f.name}”? Its notes and subfolders move to ${parent} — nothing is deleted.`, 'Delete folder')) {
                folderAction(() => api.deleteFolder(f.id))
                if (activeFolderId === f.id) setActiveFolderId(f.parentId)
              }
            }}
          />
        )}
        {!locked && !visible.length && !folders.length && (
          <div className="small faint" style={{ padding: '10px 16px' }}>
            {filter ? 'No matching notes' : listMode.kind === 'archive' ? 'Nothing archived' : 'No notes yet — press Ctrl N'}
          </div>
        )}

        {!locked && tags.length > 0 && listMode.kind === 'tree' && !filter && (
          <div className="tags-section">
            <div className="tree-row" style={{ paddingLeft: 6 }} onClick={() => setTagsOpen((o) => !o)}>
              <span className="tree-caret">{tagsOpen ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</span>
              <span className="grow small muted" style={{ textTransform: 'uppercase', letterSpacing: '.08em', fontSize: 11 }}>Tags</span>
            </div>
            {tagsOpen &&
              tags.map((t) => (
                <div key={t.name} className="tree-row" style={{ paddingLeft: 23 }} onClick={() => setListMode({ kind: 'tag', tag: t.name })}>
                  <span className="tree-icon"><Hash size={13} /></span>
                  <span className="grow ellipsis">{t.name}</span>
                  <span className="small faint">{t.count}</span>
                </div>
              ))}
          </div>
        )}
      </div>

      {templates && (
        <Modal title="New note from template" onClose={() => setTemplates(null)}>
          <div className="list">
            {templates.map((t) => (
              <div
                key={t.id}
                className="list-item"
                onClick={() => {
                  setTemplates(null)
                  createNote('note', { templateId: t.id })
                }}
              >
                <div className="title">{t.name}</div>
                <div className="preview">{plain(t.body).slice(0, 120)}</div>
              </div>
            ))}
          </div>
        </Modal>
      )}
      {prompt.element}
      {confirm.element}
    </div>
  )
}
