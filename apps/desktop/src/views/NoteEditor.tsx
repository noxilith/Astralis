import { useEffect, useRef, useState } from 'react'
import {
  Archive,
  ArchiveRestore,
  Eye,
  FilePlus2,
  FolderInput,
  History,
  Lock,
  MoreHorizontal,
  PanelRight,
  Pencil,
  Pin,
  PinOff,
  SpellCheck,
  StickyNote,
  Trash2,
  Sparkles,
} from 'lucide-react'
import type { EditorView } from '@codemirror/view'
import { api, type Folder, type Id, type Note, type TidyResult } from '../api'
import { MarkdownBar } from '../components/MarkdownBar'
import { AiMenu } from '../components/AiMenu'
import { setImageWidth } from '../lib/attachments'
import { useApp } from '../context'
import { Editor } from '../components/Editor'
import { Markdown } from '../components/Markdown'
import { WhiteboardEditor } from '../components/WhiteboardEditor'
import { UnlockVault, vaultIcon } from '../components/VaultSwitcher'
import { folderPath } from '../components/NotesNav'
import { Empty, Modal, Spinner, useConfirm, useDebounced, usePrompt } from '../components/ui'
import { relativeTime } from '../lib/time'
import { NotePanel } from './NotePanel'
import { HistoryModal } from './HistoryModal'

/** The main area for notes: the open note or whiteboard, its toolbar and side panel. */
export function NoteEditor({ theme }: { theme: 'dark' | 'light' }) {
  const app = useApp()
  const { noteId, setNoteId, vaultId, setVaultId, vaults, refreshVaults, bump, fail, toast, publishNote } = app
  const vault = vaults.find((v) => v.id === vaultId) ?? null
  const locked = !!vault && vault.encrypted && !vault.unlocked
  const [current, setCurrent] = useState<Note | null>(null)
  const [editorKey, setEditorKey] = useState(0)
  const [preview, setPreview] = useState(false)
  const [showPanel, setShowPanel] = useState(true)
  const [grammar, setGrammar] = useState(true)
  const [showHistory, setShowHistory] = useState(false)
  const [menuOpen, setMenuOpen] = useState(false)
  const [moveOpen, setMoveOpen] = useState(false)
  const [folderPicker, setFolderPicker] = useState<Folder[] | null>(null)
  const confirm = useConfirm()
  const prompt = usePrompt()
  const draft = useRef<{ id: Id; title: string; body: string } | null>(null)
  const titleRef = useRef<HTMLInputElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  const [tidy, setTidy] = useState<{ result: TidyResult; original: string } | null>(null)
  const [tidyBusy, setTidyBusy] = useState(false)
  const [fixSpelling, setFixSpelling] = useState(true)

  const openTidy = async (spelling: boolean) => {
    if (!draft.current) return
    const original = draft.current.body
    setTidyBusy(true)
    try {
      setTidy({ result: await api.tidyNote(original, spelling), original })
    } catch (e) {
      fail(e)
    } finally {
      setTidyBusy(false)
    }
  }

  useEffect(() => {
    api.getSetting('grammar').then((v) => setGrammar(v !== 'off')).catch(() => {})
  }, [])

  // Fresh, empty notes start with the cursor in the title.
  useEffect(() => {
    if (current && !current.title && !current.body && !current.archivedAt) titleRef.current?.focus()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current?.id])

  // Load the selected note (switching to its vault if needed) and remount the editor.
  useEffect(() => {
    if (!noteId || locked) {
      setCurrent(null)
      return
    }
    if (current?.id === noteId) return
    api
      .getNote(noteId)
      .then((n) => {
        if (n.vaultId !== vaultId) setVaultId(n.vaultId)
        setCurrent(n)
        setPreview(false)
        draft.current = { id: n.id, title: n.title, body: n.body }
        setEditorKey((k) => k + 1)
      })
      .catch(() => setNoteId(null))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [noteId, locked])

  const save = useDebounced(async (id: Id, title: string, body: string) => {
    try {
      const n = await api.updateNote(id, { title, body })
      publishNote(n)
      setCurrent((c) => (c && c.id === n.id ? { ...n, title: c.title, body: c.body } : c))
    } catch (e) {
      fail(e)
    }
  }, 500)

  const saveTitle = useDebounced(async (id: Id, title: string) => {
    try {
      publishNote(await api.updateNote(id, { title }))
    } catch (e) {
      fail(e)
    }
  }, 500)

  // Flush pending edits when switching notes or leaving the view.
  useEffect(
    () => () => {
      save.flush()
      saveTitle.flush()
    },
    [noteId], // eslint-disable-line react-hooks/exhaustive-deps
  )

  const edit = (patch: { title?: string; body?: string }) => {
    if (!current || !draft.current) return
    draft.current = { ...draft.current, ...patch }
    setCurrent({ ...current, ...patch })
    if (current.kind === 'whiteboard') {
      // The canvas saves its own scene; only the title goes through here (never the body).
      if (patch.title !== undefined) saveTitle.call(current.id, draft.current.title)
      return
    }
    save.call(current.id, draft.current.title, draft.current.body)
  }

  const replaceCurrent = (n: Note, remountEditor = false) => {
    setCurrent(n)
    draft.current = { id: n.id, title: n.title, body: n.body }
    publishNote(n)
    if (remountEditor) setEditorKey((k) => k + 1)
  }

  const openLink = async (title: string) => {
    if (!current || current.encrypted) return
    try {
      save.flush()
      const found = await api.findByTitle(current.vaultId, title)
      if (found.length) {
        app.openNote(found[0].id)
      } else {
        const n = await api.createNote({ title, vaultId: current.vaultId, folderId: current.folderId })
        bump()
        app.openNote(n.id)
        toast(`Created “${title}”`)
      }
    } catch (e) {
      fail(e)
    }
  }

  const act = async (fn: () => Promise<unknown>, after?: () => void) => {
    try {
      await fn()
      after?.()
      bump()
      refreshVaults()
    } catch (e) {
      fail(e)
    }
  }

  if (locked && vault) return <UnlockVault vault={vault} />
  if (!current)
    return (
      <Empty icon={<StickyNote size={34} />} title="Select or create a note">
        <span className="small">
          <span className="kbd">Ctrl K</span> search · <span className="kbd">Ctrl N</span> new note
        </span>
      </Empty>
    )

  const isBoard = current.kind === 'whiteboard'
  const readOnly = !!current.archivedAt
  const panel = showPanel && !isBoard && !current.encrypted

  return (
    <div className={'split' + (panel ? ' split-editor' : '')} style={panel ? undefined : { gridTemplateColumns: 'minmax(0,1fr)' }}>
      <div className="pane">
        <div className="editor-wrap">
          <div className="editor-toolbar">
            <span className="small faint grow">
              {current.encrypted && <Lock size={11} style={{ marginRight: 4 }} />}
              {readOnly ? 'Archived — read only' : `Edited ${relativeTime(current.updatedAt)}`}
            </span>
            {!readOnly && !isBoard && (
              <button className={'icon-btn' + (preview ? ' active' : '')} title={preview ? 'Edit' : 'Preview'} onClick={() => setPreview((p) => !p)}>
                {preview ? <Pencil size={16} /> : <Eye size={16} />}
              </button>
            )}
            {!readOnly && !isBoard && (
              <button
                className={'icon-btn' + (grammar ? ' active' : '')}
                title="Grammar & spelling check"
                onClick={() => {
                  setGrammar(!grammar)
                  api.setSetting('grammar', grammar ? 'off' : 'on').catch(() => {})
                }}
              >
                <SpellCheck size={16} />
              </button>
            )}
            {!isBoard && !current.encrypted && (
              <button className="icon-btn" title="History" onClick={() => setShowHistory(true)}>
                <History size={16} />
              </button>
            )}
            <button
              className={'icon-btn' + (current.pinned ? ' active' : '')}
              title={current.pinned ? 'Unpin' : 'Pin'}
              onClick={() => act(() => api.setPinned(current.id, !current.pinned).then((n) => replaceCurrent(n)))}
            >
              {current.pinned ? <PinOff size={16} /> : <Pin size={16} />}
            </button>
            {!isBoard && (
              <AiMenu note={current} readOnly={readOnly} onTidy={() => openTidy(fixSpelling)} onChange={(n, remount) => replaceCurrent(n, remount)} />
            )}
            {!isBoard && !current.encrypted && (
              <button className={'icon-btn' + (showPanel ? ' active' : '')} title="Side panel" onClick={() => setShowPanel((s) => !s)}>
                <PanelRight size={16} />
              </button>
            )}
            <div style={{ position: 'relative' }}>
              <button className="icon-btn" title="More" onClick={() => setMenuOpen((m) => !m)}>
                <MoreHorizontal size={16} />
              </button>
              {menuOpen && (
                <>
                  <div className="menu-backdrop" onClick={() => setMenuOpen(false)} />
                  <div className="menu">
                    {readOnly ? (
                      <button onClick={() => (setMenuOpen(false), act(() => api.unarchiveNote(current.id).then((n) => replaceCurrent(n))))}>
                        <ArchiveRestore size={14} /> Restore from archive
                      </button>
                    ) : (
                      <button
                        onClick={() => {
                          setMenuOpen(false)
                          act(
                            () => (save.flush(), api.archiveNote(current.id)),
                            () => {
                              setNoteId(null)
                              toast('Archived', {
                                actions: [{ label: 'Undo', run: () => act(() => api.unarchiveNote(current.id), () => setNoteId(current.id)) }],
                              })
                            },
                          )
                        }}
                      >
                        <Archive size={14} /> Archive
                      </button>
                    )}
                    <div className="menu-sep" />
                    {!readOnly && (
                      <button
                        onClick={() => {
                          setMenuOpen(false)
                          api.listFolders(current.vaultId).then(setFolderPicker).catch(fail)
                        }}
                      >
                        <FolderInput size={14} /> Move to folder…
                      </button>
                    )}
                    {vaults.length > 1 && (
                      <button onClick={() => (setMenuOpen(false), setMoveOpen(true))}>
                        <FolderInput size={14} /> Move to vault…
                      </button>
                    )}
                    {!isBoard && (
                      <button
                        onClick={async () => {
                          setMenuOpen(false)
                          const name = await prompt.ask('Save as template', 'Template name', current.title)
                          if (name) act(() => api.templateFromNote(current.id, name), () => toast('Template saved'))
                        }}
                      >
                        <FilePlus2 size={14} /> Save as template
                      </button>
                    )}
                    <button
                      className="danger"
                      onClick={async () => {
                        setMenuOpen(false)
                        if (await confirm.ask(`Permanently delete “${current.title || 'Untitled'}”${isBoard ? '' : ' and its history'}? This can't be undone.`, 'Delete forever')) {
                          act(() => api.deleteNote(current.id), () => setNoteId(null))
                        }
                      }}
                    >
                      <Trash2 size={14} /> Delete permanently
                    </button>
                  </div>
                </>
              )}
            </div>
          </div>
          {!isBoard && !readOnly && !preview && <MarkdownBar getView={() => viewRef.current} vaultId={current.vaultId} />}
          <input
            ref={titleRef}
            className="title-input"
            style={isBoard ? { fontSize: 18, padding: '10px 18px 8px' } : undefined}
            value={current.title}
            placeholder="Untitled"
            readOnly={readOnly}
            onChange={(e) => edit({ title: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !isBoard) (document.querySelector('.cm-content') as HTMLElement | null)?.focus()
            }}
          />
          {isBoard ? (
            <WhiteboardEditor key={`${current.id}:${editorKey}`} note={current} theme={theme} readOnly={readOnly} />
          ) : preview || readOnly ? (
            <div className="editor-host">
              <Markdown
                source={current.body}
                onOpenLink={openLink}
                onResizeImage={readOnly ? undefined : (src, occ, width) => edit({ body: setImageWidth(draft.current?.body ?? current.body, src, occ, width) })}
              />
            </div>
          ) : (
            <Editor
              key={`${current.id}:${editorKey}`}
              value={current.body}
              vaultId={current.vaultId}
              onChange={(body) => edit({ body })}
              onOpenLink={openLink}
              grammar={grammar}
              autoFocus={!current.body && !!current.title}
              onReady={(v) => (viewRef.current = v)}
            />
          )}
        </div>
      </div>

      {panel && (
        <div className="pane">
          <NotePanel note={current} readOnly={readOnly} onChange={(n, remount) => replaceCurrent(n, remount)} onOpenLink={openLink} />
        </div>
      )}

      {showHistory && (
        <HistoryModal
          note={current}
          onClose={() => setShowHistory(false)}
          onRestored={(n) => {
            replaceCurrent(n, true)
            setShowHistory(false)
            bump()
          }}
        />
      )}

      {(tidy || tidyBusy) && (
        <Modal
          title="Tidy up"
          wide
          onClose={() => setTidy(null)}
          footer={
            <>
              <label className="checkbox grow">
                <input
                  type="checkbox"
                  checked={fixSpelling}
                  onChange={(e) => {
                    setFixSpelling(e.target.checked)
                    openTidy(e.target.checked)
                  }}
                />
                Fix spelling &amp; grammar
              </label>
              <button className="btn" onClick={() => setTidy(null)}>Cancel</button>
              <button
                className="btn primary"
                disabled={!tidy || tidy.result.body === tidy.original}
                onClick={async () => {
                  if (!tidy) return
                  try {
                    save.flush()
                    const n = await api.updateNote(current.id, { body: tidy.result.body })
                    replaceCurrent(n, true)
                    setTidy(null)
                    toast('Note tidied — the previous version is in History')
                  } catch (e) {
                    fail(e)
                  }
                }}
              >
                Apply
              </button>
            </>
          }
        >
          {!tidy ? (
            <Empty title="Tidying…"><Spinner /></Empty>
          ) : tidy.result.body === tidy.original ? (
            <Empty icon={<Sparkles size={28} />} title="This note is already tidy" />
          ) : (
            <>
              <ul className="tidy-changes">
                {tidy.result.changes.map((c) => (
                  <li key={c}><Sparkles size={12} /> {c}</li>
                ))}
              </ul>
              <div className="tidy-compare">
                <div>
                  <div className="tidy-label">Before</div>
                  <div className="tidy-pane"><Markdown source={tidy.original} /></div>
                </div>
                <div>
                  <div className="tidy-label">After</div>
                  <div className="tidy-pane"><Markdown source={tidy.result.body} /></div>
                </div>
              </div>
            </>
          )}
        </Modal>
      )}

      {folderPicker && (
        <Modal title="Move to folder" onClose={() => setFolderPicker(null)}>
          <div className="tree" style={{ minHeight: 0, padding: 0 }}>
            {[{ id: null as Id | null, label: 'Top level of the vault' }, ...folderPicker.map((f) => ({ id: f.id as Id | null, label: folderPath(folderPicker, f.id) }))]
              .sort((a, b) => (a.id === null ? -1 : b.id === null ? 1 : a.label.localeCompare(b.label)))
              .map((o) => (
                <div
                  key={o.id ?? 'root'}
                  className={'tree-row' + (o.id === current.folderId ? ' active' : '')}
                  onClick={() =>
                    act(async () => {
                      const n = await api.moveNoteToFolder(current.id, o.id)
                      setCurrent({ ...current, folderId: n.folderId })
                      publishNote(n)
                      setFolderPicker(null)
                    })
                  }
                >
                  <span className="tree-icon folder"><FolderInput size={14} /></span>
                  <span className="ellipsis">{o.label}</span>
                </div>
              ))}
          </div>
        </Modal>
      )}

      {moveOpen && (
        <Modal title="Move to vault" onClose={() => setMoveOpen(false)}>
          <p className="muted small">
            Moving into an encrypted vault erases this note's history, tags, links and AI index, so no plaintext copy remains.
          </p>
          <div className="list">
            {vaults
              .filter((v) => v.id !== current.vaultId)
              .map((v) => (
                <div
                  key={v.id}
                  className="list-item"
                  style={v.unlocked ? undefined : { opacity: 0.5, cursor: 'default' }}
                  title={v.unlocked ? undefined : 'Unlock this vault first'}
                  onClick={() =>
                    v.unlocked &&
                    act(
                      () => (save.flush(), api.moveNoteToVault(current.id, v.id)),
                      () => {
                        setMoveOpen(false)
                        setNoteId(null)
                        toast(`Moved to ${v.name}`)
                      },
                    )
                  }
                >
                  <div className="title">
                    {vaultIcon(v, 13)} {v.name} {!v.unlocked && <span className="small faint">(locked)</span>}
                  </div>
                </div>
              ))}
          </div>
        </Modal>
      )}
      {confirm.element}
      {prompt.element}
    </div>
  )
}
