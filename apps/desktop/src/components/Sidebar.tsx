import { useEffect, useState, type ReactNode } from 'react'
import { Archive, CalendarCheck, FileText, Network, Search, Settings, StickyNote } from 'lucide-react'
import { PRODUCTIVITY_TABS, lastProductivityTab, type ProductivityTab } from '../views/Productivity'
import { api } from '../api'
import { useApp, type View } from '../context'
import { NotesNav } from './NotesNav'
import { AstralisMark } from './Logo'

/** Left column: logo, the vault's notes tree, and an icon bar for every other section. */
export function Sidebar() {
  const { view, setView, version, setPaletteOpen, listMode, setListMode } = useApp()
  const [dueToday, setDueToday] = useState(0)

  useEffect(() => {
    const end = new Date()
    end.setHours(24, 0, 0, 0)
    api.listTasks({ dueBefore: end.getTime() }).then((t) => setDueToday(t.length)).catch(() => {})
  }, [version])

  const archive = listMode.kind === 'archive'
  const go = (v: View) => {
    if (v === 'notes' && archive) setListMode({ kind: 'tree' })
    setView(v)
  }

  const icon = (label: string, node: ReactNode, active: boolean, onClick: () => void, badge?: number) => (
    <button key={label} className={'rail-btn' + (active ? ' active' : '')} title={label} aria-label={label} onClick={onClick}>
      {node}
      {!!badge && <span className="rail-badge">{badge > 9 ? '9+' : badge}</span>}
    </button>
  )

  return (
    <nav className="sidebar">
      <div className="brand">
        <AstralisMark size={30} />
        <span className="wordmark">Astralis</span>
      </div>
      <NotesNav />
      <div className="rail">
        {icon('Notes', <StickyNote size={17} />, view === 'notes' && !archive, () => go('notes'))}
        {icon('Search (Ctrl K)', <Search size={17} />, false, () => setPaletteOpen(true))}
        {icon(
          'Productivity — tasks, calendar, planners',
          <CalendarCheck size={17} />,
          PRODUCTIVITY_TABS.includes(view as ProductivityTab),
          () => go(lastProductivityTab()),
          dueToday,
        )}
        {icon('Graph', <Network size={17} />, view === 'graph', () => go('graph'))}
        {icon('Templates', <FileText size={17} />, view === 'templates', () => go('templates'))}
        {icon('Archive', <Archive size={17} />, view === 'notes' && archive, () => {
          setListMode(archive ? { kind: 'tree' } : { kind: 'archive' })
          setView('notes')
        })}
        <span className="grow" />
        {icon('Settings', <Settings size={17} />, view === 'settings', () => go('settings'))}
      </div>
    </nav>
  )
}
