// What the Projects view shares between the sidebar (the file tree) and the main area (open files,
// the run output): kept outside React so both see the same thing and a run keeps going when you
// switch views.
import { useSyncExternalStore } from 'react'
import { api, type FileData, type ProjectRow, type RunInfo } from '../api'

export interface RunState {
  lines: string[]
  next: number
  running: boolean
  exit: number | null
}

interface Tabs {
  open: string[]
  active: string | null
}

interface State {
  /** Projects whose folder is on this device. */
  projects: ProjectRow[]
  current: string | null
  files: string[]
  filesError: string | null
  tabs: Record<string, Tabs>
  /** What was last read from disk, by `hash\0path`. */
  loaded: Record<string, FileData>
  /** Unsaved text, by `hash\0path`. */
  drafts: Record<string, string>
  run: Record<string, RunState>
  runInfo: RunInfo | null
  outputOpen: boolean
}

let state: State = { projects: [], current: null, files: [], filesError: null, tabs: {}, loaded: {}, drafts: {}, run: {}, runInfo: null, outputOpen: true }
const listeners = new Set<() => void>()

function set(patch: Partial<State> | ((s: State) => Partial<State>)) {
  state = { ...state, ...(typeof patch === 'function' ? patch(state) : patch) }
  listeners.forEach((l) => l())
}

export function useProjectsState(): State {
  return useSyncExternalStore(
    (cb) => (listeners.add(cb), () => void listeners.delete(cb)),
    () => state,
  )
}

export const key = (hash: string, path: string) => `${hash}\0${path}`
const LAST = 'astralis.projects.current'

// ---- projects and files ----

export async function refreshProjects(): Promise<void> {
  const info = await api.projectsList()
  const here = info.rows.filter((r) => r.here)
  set((s) => {
    let current = s.current && here.some((r) => r.hash === s.current) ? s.current : null
    if (!current) {
      let last: string | null = null
      try {
        last = localStorage.getItem(LAST)
      } catch {
        /* no storage */
      }
      current = here.find((r) => r.hash === last)?.hash ?? here[0]?.hash ?? null
    }
    return { projects: here, current }
  })
  if (state.current && state.files.length === 0) await refreshFiles()
  if (state.current) await refreshRunInfo()
}

export function selectProject(hash: string) {
  try {
    localStorage.setItem(LAST, hash)
  } catch {
    /* no storage */
  }
  set({ current: hash, files: [], filesError: null, runInfo: null })
  void refreshFiles()
  void refreshRunInfo()
}

export async function refreshFiles(): Promise<void> {
  const hash = state.current
  if (!hash) return
  try {
    const files = await api.projectFiles(hash)
    if (state.current === hash) set({ files, filesError: null })
  } catch (e) {
    if (state.current === hash) set({ files: [], filesError: e instanceof Error ? e.message : String(e) })
  }
}

// ---- open files ----

const tabsOf = (s: State, hash: string): Tabs => s.tabs[hash] ?? { open: [], active: null }

export function openFile(path: string) {
  const hash = state.current
  if (!hash) return
  set((s) => {
    const t = tabsOf(s, hash)
    return { tabs: { ...s.tabs, [hash]: { open: t.open.includes(path) ? t.open : [...t.open, path], active: path } } }
  })
  void loadFile(path)
}

export function activateFile(path: string) {
  const hash = state.current
  if (!hash) return
  set((s) => ({ tabs: { ...s.tabs, [hash]: { ...tabsOf(s, hash), active: path } } }))
  // Claude edits files too: a tab with no unsaved changes shows what is on disk now.
  if (!state.drafts[key(hash, path)]) void loadFile(path)
}

export function closeFile(path: string) {
  const hash = state.current
  if (!hash) return
  set((s) => {
    const t = tabsOf(s, hash)
    const open = t.open.filter((p) => p !== path)
    const at = t.open.indexOf(path)
    const active = t.active === path ? (open[Math.min(at, open.length - 1)] ?? null) : t.active
    const drafts = { ...s.drafts }
    delete drafts[key(hash, path)]
    return { tabs: { ...s.tabs, [hash]: { open, active } }, drafts }
  })
}

export async function loadFile(path: string): Promise<void> {
  const hash = state.current
  if (!hash) return
  const data = await api.projectRead(hash, path)
  set((s) => ({ loaded: { ...s.loaded, [key(hash, path)]: data } }))
}

export function editFile(path: string, text: string) {
  const hash = state.current
  if (!hash) return
  const k = key(hash, path)
  set((s) => {
    const drafts = { ...s.drafts }
    if (s.loaded[k]?.text === text) delete drafts[k]
    else drafts[k] = text
    return { drafts }
  })
}

export async function saveFile(path: string): Promise<void> {
  const hash = state.current
  if (!hash) return
  const k = key(hash, path)
  const text = state.drafts[k]
  if (text === undefined) return
  await api.projectWrite(hash, path, text)
  set((s) => {
    const drafts = { ...s.drafts }
    delete drafts[k]
    return { drafts, loaded: { ...s.loaded, [k]: { text, binary: false, tooBig: false, size: new Blob([text]).size } } }
  })
  void refreshFiles()
}

// ---- running ----

export async function refreshRunInfo(): Promise<void> {
  const hash = state.current
  if (!hash) return
  try {
    const info = await api.projectRunInfo(hash)
    if (state.current === hash) set({ runInfo: info })
  } catch {
    if (state.current === hash) set({ runInfo: null })
  }
}

let timer: number | null = null

async function poll() {
  const running = Object.entries(state.run).filter(([, r]) => r.running)
  for (const [hash, r] of running) {
    try {
      const out = await api.projectRunOutput(hash, r.next)
      set((s) => ({ run: { ...s.run, [hash]: { lines: [...(s.run[hash]?.lines ?? []), ...out.lines].slice(-5000), next: out.next, running: out.running, exit: out.exit } } }))
    } catch {
      set((s) => ({ run: { ...s.run, [hash]: { ...(s.run[hash] ?? r), running: false } } }))
    }
  }
  if (!Object.values(state.run).some((r) => r.running) && timer !== null) {
    window.clearInterval(timer)
    timer = null
    void refreshRunInfo()
  }
}

function ensurePolling() {
  if (timer === null) timer = window.setInterval(() => void poll(), 400)
}

/** Starts the run (the caller has asked the user to confirm a command they haven't run before). */
export async function startRun(confirmed: boolean): Promise<void> {
  const hash = state.current
  if (!hash) return
  await api.projectRun(hash, confirmed)
  set((s) => ({ run: { ...s.run, [hash]: { lines: [], next: 0, running: true, exit: null } }, outputOpen: true }))
  ensurePolling()
  void refreshRunInfo()
}

export async function stopRun(): Promise<void> {
  const hash = state.current
  if (hash) await api.projectStop(hash)
}

export function clearOutput() {
  const hash = state.current
  if (!hash) return
  set((s) => ({ run: { ...s.run, [hash]: { ...(s.run[hash] ?? { next: 0, running: false, exit: null }), lines: [] } } }))
}

export function setOutputOpen(open: boolean) {
  set({ outputOpen: open })
}

/** A run that was already going when the view opened (started before a reload). */
export function resumeRunIfAny() {
  const hash = state.current
  if (hash && state.runInfo?.running && !state.run[hash]?.running) {
    set((s) => ({ run: { ...s.run, [hash]: { lines: [], next: 0, running: true, exit: null } } }))
    ensurePolling()
  }
}
