import { lazy, Suspense, useEffect, useMemo, useRef } from 'react'
import { api, type Note } from '../api'
import { useApp } from '../context'
import { Empty, Spinner, useDebounced } from './ui'
import '@excalidraw/excalidraw/index.css'

// Excalidraw is large; only load it when a whiteboard is opened.
const Excalidraw = lazy(async () => {
  ;(window as unknown as { EXCALIDRAW_ASSET_PATH: string }).EXCALIDRAW_ASSET_PATH = '/excalidraw/'
  const m = await import('@excalidraw/excalidraw')
  return { default: m.Excalidraw }
})

/** Canvas editor for whiteboard notes. The scene is saved (debounced) as the note body. */
export function WhiteboardEditor({ note, theme, readOnly }: { note: Note; theme: 'dark' | 'light'; readOnly?: boolean }) {
  const { fail } = useApp()
  const last = useRef(note.body)
  const save = useDebounced((scene: string) => api.saveWhiteboard(note.id, scene).catch(fail), 800)
  useEffect(() => () => save.flush(), []) // eslint-disable-line react-hooks/exhaustive-deps

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
          UIOptions={{ canvasActions: { loadScene: false, saveToActiveFile: false, export: { saveFileToDisk: true } } }}
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
          }}
        />
      </Suspense>
    </div>
  )
}
