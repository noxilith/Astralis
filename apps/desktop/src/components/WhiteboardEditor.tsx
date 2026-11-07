import { lazy, Suspense, useEffect, useMemo, useRef } from 'react'
import type { ExcalidrawImperativeAPI, Collaborator, SocketId } from '@excalidraw/excalidraw/types'
import { api, p2p, type Note } from '../api'
import { useApp } from '../context'
import { REMOTE_SCENE, connectedCount, throttle, useCollab } from '../lib/collab'
import { Empty, Spinner, useDebounced } from './ui'
import '@excalidraw/excalidraw/index.css'

// Excalidraw is large; only load it when a whiteboard is opened.
const Excalidraw = lazy(async () => {
  ;(window as unknown as { EXCALIDRAW_ASSET_PATH: string }).EXCALIDRAW_ASSET_PATH = '/excalidraw/'
  const m = await import('@excalidraw/excalidraw')
  return { default: m.Excalidraw }
})

type El = { id: string; version: number; versionNonce: number; isDeleted?: boolean }

/** Element-wise merge: the newer version of each element wins (ties broken deterministically). */
function mergeElements(local: readonly El[], remote: El[]): El[] {
  const byId = new Map(local.map((e) => [e.id, e]))
  const out = [...local]
  for (const r of remote) {
    const l = byId.get(r.id)
    if (!l) {
      out.push(r)
    } else if (r.version > l.version || (r.version === l.version && r.versionNonce < l.versionNonce)) {
      out[out.indexOf(l)] = r
    }
  }
  return out
}

const signature = (els: readonly El[]) => els.map((e) => `${e.id}:${e.version}`).join('|')

/**
 * Canvas editor for whiteboard notes. The scene is saved (debounced) as the note body. While
 * paired devices are connected, drawing streams live and their pointers show with names.
 */
export function WhiteboardEditor({ note, theme, readOnly }: { note: Note; theme: 'dark' | 'light'; readOnly?: boolean }) {
  const { fail } = useApp()
  const collab = useCollab()
  const apiRef = useRef<ExcalidrawImperativeAPI | null>(null)
  const last = useRef(note.body)
  const lastRemote = useRef('')
  const save = useDebounced((scene: string) => api.saveWhiteboard(note.id, scene).catch(fail), 800)
  useEffect(() => () => save.flush(), []) // eslint-disable-line react-hooks/exhaustive-deps

  const sendScene = useMemo(() => throttle((els: string) => p2p.liveScene(note.id, els).catch(() => {}), 120), [note.id])
  const sendPointer = useMemo(
    () => throttle((x: number, y: number) => p2p.livePresence(note.id, null, null, [x, y]).catch(() => {}), 50),
    [note.id],
  )
  useEffect(() => () => void p2p.livePresence(null, null, null, null).catch(() => {}), [note.id])

  // Drawing from collaborators.
  useEffect(() => {
    const onScene = (e: Event) => {
      const { noteId, elements } = (e as CustomEvent<{ noteId: string; elements: string }>).detail
      const a = apiRef.current
      if (noteId !== note.id || !a) return
      try {
        const merged = mergeElements(a.getSceneElementsIncludingDeleted() as unknown as El[], JSON.parse(elements))
        lastRemote.current = signature(merged)
        a.updateScene({ elements: merged as never })
      } catch {
        /* ignore malformed scene */
      }
    }
    window.addEventListener(REMOTE_SCENE, onScene)
    return () => window.removeEventListener(REMOTE_SCENE, onScene)
  }, [note.id])

  // Collaborators' pointers, with names.
  const pointers = collab.presence.filter((p) => p.noteId === note.id && p.pointer)
  useEffect(() => {
    const a = apiRef.current
    if (!a) return
    const map = new Map<SocketId, Collaborator>()
    for (const p of pointers) {
      map.set(p.peer as SocketId, {
        username: p.name,
        color: { background: p.color, stroke: p.color },
        pointer: { x: p.pointer![0], y: p.pointer![1], tool: 'pointer' },
        id: p.peer,
      })
    }
    a.updateScene({ collaborators: map })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [JSON.stringify(pointers)])

  const initialData = useMemo(() => {
    try {
      const s = JSON.parse(note.body || '{}')
      return { elements: s.elements ?? [], appState: { ...(s.appState ?? {}), collaborators: new Map() }, files: s.files ?? {} }
    } catch {
      return { elements: [], appState: {}, files: {} }
    }
    // Only the initial scene matters; the editor owns it afterwards.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [note.id])

  return (
    <div className="wb-host">
      <Suspense fallback={<Empty title="Loading whiteboard…"><Spinner /></Empty>}>
        <Excalidraw
          theme={theme}
          initialData={initialData}
          viewModeEnabled={readOnly}
          excalidrawAPI={(a) => (apiRef.current = a)}
          isCollaborating={collab.connected > 0}
          UIOptions={{ canvasActions: { loadScene: false, saveToActiveFile: false, export: { saveFileToDisk: true } } }}
          onPointerUpdate={({ pointer }) => {
            if (connectedCount() > 0) sendPointer(pointer.x, pointer.y)
          }}
          onChange={(elements, appState, files) => {
            if (readOnly) return
            const scene = JSON.stringify({
              elements: elements.filter((el) => !el.isDeleted),
              appState: { viewBackgroundColor: appState.viewBackgroundColor, gridSize: appState.gridSize },
              files,
            })
            if (scene !== last.current) {
              last.current = scene
              save.call(scene)
            }
            // Stream our own drawing (deleted elements too, so erasing propagates) — but not
            // changes that just came in from someone else.
            const all = elements as unknown as El[]
            if (connectedCount() > 0 && signature(all) !== lastRemote.current) {
              lastRemote.current = signature(all)
              sendScene(JSON.stringify(all))
            }
          }}
        />
      </Suspense>
    </div>
  )
}
