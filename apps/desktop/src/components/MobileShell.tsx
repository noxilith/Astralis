import { useEffect, useState, type ReactNode } from 'react'
import { Brain as BrainIcon, CalendarDays, CheckSquare, ChevronLeft, FileText, Kanban, Network, Archive, Search, Settings, StickyNote, MoreHorizontal } from 'lucide-react'
import { api } from '../api'
import { useApp, type View } from '../context'
import { PHONE } from '../lib/platform'
import { NotesNav } from './NotesNav'
import { AstralisMark } from './Logo'

interface Tab {
  id: string
  label: string
  icon: ReactNode
  views: View[]
  open: () => void
  badge?: number
}

/**
 * The phone layout: one screen at a time with tabs along the bottom. Notes is a list that opens a
 * note full screen (with a Back button); Tasks, Calendar and Graph are the same screens as on the
 * desktop; "More" holds the rest.
 */
export function MobileShell({ children }: { children: ReactNode }) {
  const { view, setView, noteId, setNoteId, version, setPaletteOpen, listMode, setListMode } = useApp()
  const [dueToday, setDueToday] = useState(0)
  const [more, setMore] = useState(false)

  useEffect(() => {
    const end = new Date()
    end.setHours(24, 0, 0, 0)
    api.listTasks({ dueBefore: end.getTime() }).then((t) => setDueToday(t.length)).catch(() => {})
  }, [version])

  const archive = listMode.kind === 'archive'
  const go = (v: View) => {
    setMore(false)
    if (v === 'notes' && archive) setListMode({ kind: 'tree' })
    setView(v)
  }

  const tabs: Tab[] = [
    { id: 'notes', label: 'Notes', icon: <StickyNote size={20} />, views: ['notes'], open: () => (view === 'notes' && !archive ? setNoteId(null) : go('notes')) },
    { id: 'tasks', label: 'Tasks', icon: <CheckSquare size={20} />, views: ['tasks', 'planners'], open: () => go('tasks'), badge: dueToday },
    { id: 'calendar', label: 'Calendar', icon: <CalendarDays size={20} />, views: ['calendar'], open: () => go('calendar') },
    { id: 'graph', label: 'Graph', icon: <Network size={20} />, views: ['graph'], open: () => go('graph') },
  ]
  const inMore = !tabs.some((t) => t.views.includes(view) && t.id !== 'tasks') && (view === 'templates' || view === 'settings' || view === 'brain' || archive)

  const item = (label: string, icon: ReactNode, run: () => void) => (
    <button key={label} className="m-more-item" onClick={() => (setMore(false), run())}>
      {icon} {label}
    </button>
  )

  // A note is open: it fills the screen, with a way back to the list.
  const openNote = view === 'notes' && !!noteId && !archive
  const showList = view === 'notes' && !openNote

  return (
    <div className="app compact">
      <main className="m-main">
        {openNote && (
          <div className="m-bar">
            <button className="m-back" onClick={() => setNoteId(null)} aria-label="Back to notes">
              <ChevronLeft size={22} /> Notes
            </button>
          </div>
        )}
        {showList && (
          <div className="m-list-screen">
            <div className="m-list-head">
              <AstralisMark size={24} />
              <span className="wordmark">Astralis</span>
              <span className="grow" />
              <button className="icon-btn" aria-label="Search" onClick={() => setPaletteOpen(true)}>
                <Search size={19} />
              </button>
            </div>
            <NotesNav />
          </div>
        )}
        {!showList && <div className={'m-content' + (openNote ? ' with-bar' : '')}>{children}</div>}
      </main>
      <nav className="m-tabs" aria-label="Sections">
        {tabs.map((t) => (
          <button key={t.id} className={'m-tab' + (t.views.includes(view) && !archive ? ' active' : '')} onClick={t.open}>
            {t.icon}
            <span>{t.label}</span>
            {!!t.badge && <i className="m-badge">{t.badge > 9 ? '9+' : t.badge}</i>}
          </button>
        ))}
        <button className={'m-tab' + (inMore || more ? ' active' : '')} onClick={() => setMore((m) => !m)}>
          <MoreHorizontal size={20} />
          <span>More</span>
        </button>
      </nav>
      {more && (
        <>
          <div className="m-more-backdrop" onClick={() => setMore(false)} />
          <div className="m-more" role="menu">
            {item('Planners', <Kanban size={18} />, () => go('planners'))}
            {item('Templates', <FileText size={18} />, () => go('templates'))}
            {item('Archive', <Archive size={18} />, () => (setListMode({ kind: 'archive' }), setView('notes'), setNoteId(null)))}
            {!PHONE && item('Brain', <BrainIcon size={18} />, () => go('brain'))}
            {item('Settings', <Settings size={18} />, () => go('settings'))}
          </div>
        </>
      )}
    </div>
  )
}
