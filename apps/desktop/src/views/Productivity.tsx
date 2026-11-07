import { useEffect, type ReactNode } from 'react'
import { CalendarDays, CheckSquare, Kanban } from 'lucide-react'
import { useApp, type View } from '../context'
import { TasksView } from './TasksView'
import { CalendarView } from './CalendarView'
import { PlannersView } from './PlannersView'

export type ProductivityTab = 'tasks' | 'calendar' | 'planners'
export const PRODUCTIVITY_TABS: ProductivityTab[] = ['tasks', 'calendar', 'planners']
const KEY = 'astralis.productivityTab'

/** The tab Productivity opens on: whichever was used last. */
export function lastProductivityTab(): ProductivityTab {
  try {
    const t = localStorage.getItem(KEY) as ProductivityTab | null
    return t && PRODUCTIVITY_TABS.includes(t) ? t : 'tasks'
  } catch {
    return 'tasks'
  }
}

const TABS: { view: ProductivityTab; label: string; icon: ReactNode }[] = [
  { view: 'tasks', label: 'Tasks', icon: <CheckSquare size={14} /> },
  { view: 'calendar', label: 'Calendar', icon: <CalendarDays size={14} /> },
  { view: 'planners', label: 'Planners', icon: <Kanban size={14} /> },
]

/** Tasks, Calendar and Planners under one roof, switched with a segmented control. */
export function Productivity({ view }: { view: View }) {
  const { setView } = useApp()

  useEffect(() => {
    try {
      localStorage.setItem(KEY, view)
    } catch {
      /* storage unavailable: just won't remember the tab */
    }
  }, [view])

  return (
    <div className="prod">
      <div className="prod-bar">
        <div className="segmented" role="tablist">
          {TABS.map((t) => (
            <button key={t.view} role="tab" aria-selected={view === t.view} className={view === t.view ? 'on' : ''} onClick={() => setView(t.view)}>
              {t.icon} {t.label}
            </button>
          ))}
        </div>
      </div>
      <div className="prod-body">
        {view === 'tasks' && <TasksView />}
        {view === 'calendar' && <CalendarView />}
        {view === 'planners' && <PlannersView />}
      </div>
    </div>
  )
}
