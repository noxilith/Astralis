import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { api, errorMessage, type Id, type Note, type NoteKind, type Vault } from './api'

export type View = 'notes' | 'tasks' | 'calendar' | 'planners' | 'graph' | 'templates' | 'settings'

/** What the notes list in the sidebar shows. */
export type ListMode = { kind: 'tree' } | { kind: 'archive' } | { kind: 'tag'; tag: string }

export interface ToastAction { label: string; run: () => void }
export interface Toast { id: number; message: string; kind: 'info' | 'error'; actions?: ToastAction[] }

interface AppCtx {
  view: View
  setView: (v: View) => void
  /** The active vault (workspace) for notes. Null until loaded. */
  vaultId: Id | null
  setVaultId: (id: Id) => void
  vaults: Vault[]
  refreshVaults: () => Promise<Vault[]>
  /** Note shown in the editor. */
  noteId: Id | null
  openNote: (id: Id) => void
  setNoteId: (id: Id | null) => void
  /** Folder new notes are created in (null = vault root). */
  activeFolderId: Id | null
  setActiveFolderId: (id: Id | null) => void
  listMode: ListMode
  setListMode: (m: ListMode) => void
  /** Creates a note/whiteboard in the active vault + folder and opens it. */
  createNote: (kind: NoteKind, opts?: { templateId?: Id; folderId?: Id | null }) => Promise<Note | null>
  /** Latest saved version of a note, so lists can update without refetching. */
  lastSaved: Note | null
  publishNote: (n: Note) => void
  taskId: Id | null
  openTask: (id: Id) => void
  setTaskId: (id: Id | null) => void
  plannerId: Id | null
  openPlanner: (id: Id | null) => void
  /** Increments whenever data changes so views can refetch. */
  version: number
  bump: () => void
  toasts: Toast[]
  toast: (message: string, opts?: { kind?: 'info' | 'error'; actions?: ToastAction[]; timeout?: number }) => void
  fail: (e: unknown) => void
  dismiss: (id: number) => void
  paletteOpen: boolean
  setPaletteOpen: (open: boolean) => void
}

const Ctx = createContext<AppCtx | null>(null)

export function AppProvider({ children }: { children: ReactNode }) {
  const [view, setView] = useState<View>('notes')
  const [noteId, setNoteId] = useState<Id | null>(null)
  const [activeFolderId, setActiveFolderId] = useState<Id | null>(null)
  const [listMode, setListMode] = useState<ListMode>({ kind: 'tree' })
  const [lastSaved, setLastSaved] = useState<Note | null>(null)
  const [taskId, setTaskId] = useState<Id | null>(null)
  const [plannerId, setPlannerId] = useState<Id | null>(null)
  const [version, setVersion] = useState(0)
  const [toasts, setToasts] = useState<Toast[]>([])
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [vaultId, setVaultIdState] = useState<Id | null>(null)
  const [vaults, setVaults] = useState<Vault[]>([])
  const nextToast = useRef(1)

  const refreshVaults = useCallback(async () => {
    const list = await api.listVaults()
    setVaults(list)
    return list
  }, [])

  const setVaultId = useCallback((id: Id) => {
    setVaultIdState(id)
    api.setSetting('activeVault', id).catch(() => {})
  }, [])
  // If the active vault disappears (deleted, or merged away after pairing), fall back.
  useEffect(() => {
    if (vaultId && vaults.length && !vaults.some((v) => v.id === vaultId)) {
      api.defaultVault().then(setVaultId).catch(() => {})
    }
  }, [vaults, vaultId, setVaultId])
  // Folders belong to a vault: switching vaults resets the active folder.
  useEffect(() => setActiveFolderId(null), [vaultId])

  // Restore the last active vault (falling back to the default one).
  useEffect(() => {
    Promise.all([refreshVaults(), api.getSetting('activeVault')])
      .then(async ([list, saved]) => {
        const id = list.some((v) => v.id === saved) ? saved! : await api.defaultVault()
        setVaultIdState(id)
      })
      .catch(() => {})
  }, [refreshVaults])

  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), [])
  const toast = useCallback<AppCtx['toast']>(
    (message, opts = {}) => {
      const id = nextToast.current++
      setToasts((t) => [...t, { id, message, kind: opts.kind ?? 'info', actions: opts.actions }])
      const timeout = opts.timeout ?? (opts.actions ? 30_000 : 4000)
      if (timeout > 0) setTimeout(() => dismiss(id), timeout)
    },
    [dismiss],
  )
  const fail = useCallback((e: unknown) => toast(errorMessage(e), { kind: 'error', timeout: 6000 }), [toast])
  const bump = useCallback(() => setVersion((v) => v + 1), [])

  const createNote = useCallback<AppCtx['createNote']>(
    async (kind, opts = {}) => {
      const folderId = opts.folderId === undefined ? activeFolderId : opts.folderId
      try {
        let n = opts.templateId
          ? await api.noteFromTemplate(opts.templateId, undefined, vaultId)
          : await api.createNote({ kind, vaultId, folderId, title: kind === 'whiteboard' ? 'Untitled whiteboard' : '' })
        if (opts.templateId && folderId) n = await api.moveNoteToFolder(n.id, folderId)
        setListMode((m) => (m.kind === 'archive' ? { kind: 'tree' } : m))
        setNoteId(n.id)
        setView('notes')
        bump()
        refreshVaults()
        return n
      } catch (e) {
        fail(e)
        return null
      }
    },
    [activeFolderId, vaultId, bump, refreshVaults, fail],
  )

  const value = useMemo<AppCtx>(
    () => ({
      view,
      setView,
      vaultId,
      setVaultId,
      vaults,
      refreshVaults,
      noteId,
      setNoteId,
      openNote: (id) => {
        setNoteId(id)
        setView('notes')
      },
      activeFolderId,
      setActiveFolderId,
      listMode,
      setListMode,
      createNote,
      lastSaved,
      publishNote: setLastSaved,
      taskId,
      setTaskId,
      openTask: (id) => {
        setTaskId(id)
        setView('tasks')
      },
      plannerId,
      openPlanner: (id) => {
        setPlannerId(id)
        setView('planners')
      },
      version,
      bump,
      toasts,
      toast,
      fail,
      dismiss,
      paletteOpen,
      setPaletteOpen,
    }),
    [
      view, vaultId, setVaultId, vaults, refreshVaults, noteId, activeFolderId, listMode, createNote, lastSaved,
      taskId, plannerId, version, bump, toasts, toast, fail, dismiss, paletteOpen,
    ],
  )
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>
}

export function useApp() {
  const c = useContext(Ctx)
  if (!c) throw new Error('useApp outside AppProvider')
  return c
}
