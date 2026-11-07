import { useCallback, useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { isPermissionGranted, requestPermission } from '@tauri-apps/plugin-notification'
import { api, type Reminder } from './api'
import { AppProvider, useApp } from './context'
import { Sidebar } from './components/Sidebar'
import { CommandPalette } from './components/CommandPalette'
import { Toasts } from './components/ui'
import { QuickNote } from './components/QuickNote'
import { startCollab } from './lib/collab'
import { NoteEditor } from './views/NoteEditor'
import { Productivity } from './views/Productivity'
import { GraphView } from './views/GraphView'
import { TemplatesView } from './views/TemplatesView'
import { SettingsView } from './views/SettingsView'
import { MINUTE } from './lib/time'

type Theme = 'dark' | 'light'

function Shell() {
  const app = useApp()
  const { view, setView, paletteOpen, setPaletteOpen, toast, fail, bump, openTask, createNote, refreshVaults } = app

  // Peer-to-peer: device list, live presence, and refresh views when synced changes land.
  useEffect(() => startCollab(() => (bump(), void refreshVaults())), [bump, refreshVaults])

  // Errors raised outside React (e.g. image uploads from the editor) surface as toasts.
  useEffect(() => {
    const onError = (e: Event) => fail((e as CustomEvent).detail)
    window.addEventListener("astralis-error", onError)
    return () => window.removeEventListener("astralis-error", onError)
  }, [fail])
  const [theme, setThemeState] = useState<Theme>('dark')

  useEffect(() => {
    api.getSetting('theme').then((t) => t && setThemeState(t as Theme)).catch(() => {})
    isPermissionGranted()
      .then(async (ok) => {
        if (!ok) await requestPermission()
      })
      .catch(() => {})
  }, [])

  useEffect(() => {
    document.documentElement.dataset.theme = theme
  }, [theme])

  const setTheme = (t: Theme) => {
    setThemeState(t)
    api.setSetting('theme', t).catch(() => {})
  }

  const newNote = useCallback(() => void createNote('note'), [createNote])
  const newWhiteboard = useCallback(() => void createNote('whiteboard'), [createNote])

  // Reminders arrive from the Rust side (also shown as a system notification).
  useEffect(() => {
    const un = listen<Reminder>('reminder', ({ payload: r }) => {
      bump()
      toast(`⏰ ${r.title}`, {
        timeout: 0,
        actions: [
          { label: 'Snooze 10 min', run: () => api.snoozeReminder(r.kind, r.id, Date.now() + 10 * MINUTE).catch(fail) },
          ...(r.kind === 'task'
            ? [
                { label: 'Open', run: () => openTask(r.id) },
                { label: 'Done', run: () => api.completeTask(r.id).then(bump).catch(fail) },
              ]
            : []),
        ],
      })
    })
    return () => {
      un.then((f) => f())
    }
  }, [toast, bump, fail, openTask])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey
      if (mod && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setPaletteOpen(!paletteOpen)
      } else if (mod && !e.shiftKey && e.key.toLowerCase() === 'n') {
        e.preventDefault()
        newNote()
      } else if (mod && e.shiftKey && e.key.toLowerCase() === 't') {
        e.preventDefault()
        setView('tasks')
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [paletteOpen, setPaletteOpen, newNote, setView])

  return (
    <div className="app">
      <Sidebar />
      <main className="main">
        <div className="main-content">
        {view === 'notes' && <NoteEditor theme={theme} />}
        {(view === 'tasks' || view === 'calendar' || view === 'planners') && <Productivity view={view} />}
        {view === 'graph' && <GraphView />}
        {view === 'templates' && <TemplatesView />}
        {view === 'settings' && <SettingsView theme={theme} setTheme={setTheme} />}
        </div>
        <QuickNote />
      </main>
      {paletteOpen && <CommandPalette onNewNote={newNote} onNewWhiteboard={newWhiteboard} />}
      <Toasts />
    </div>
  )
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  )
}
