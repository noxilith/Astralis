import { useCallback, useEffect, useMemo, useState } from 'react'
import { Bell, CalendarDays, Check, CheckCircle2, Inbox, ListTodo, Repeat, StickyNote, Sun, Trash2 } from 'lucide-react'
import { api, type Frequency, type Id, type Planner, type Task, type TaskFilter, type TaskPatch } from '../api'
import { useApp } from '../context'
import { Empty, PRIORITY_COLORS, PRIORITY_LABELS, useConfirm } from '../components/ui'
import { DAY, HOUR, MINUTE, addDays, endOfDay, formatDue, fromLocalInput, startOfDay, toDateInput, toLocalInput } from '../lib/time'

type ListKey = 'inbox' | 'today' | 'upcoming' | 'nodate' | 'done' | `planner:${string}`

const LISTS: { key: ListKey; label: string; icon: React.ReactNode }[] = [
  { key: 'inbox', label: 'All open', icon: <Inbox size={15} /> },
  { key: 'today', label: 'Today', icon: <Sun size={15} /> },
  { key: 'upcoming', label: 'Next 7 days', icon: <CalendarDays size={15} /> },
  { key: 'nodate', label: 'No date', icon: <ListTodo size={15} /> },
  { key: 'done', label: 'Completed', icon: <CheckCircle2 size={15} /> },
]

function filterFor(key: ListKey): TaskFilter {
  switch (key) {
    case 'today':
      return { dueBefore: endOfDay() }
    case 'upcoming':
      return { dueBefore: endOfDay(addDays(Date.now(), 6)) }
    case 'nodate':
      return { noDueDate: true }
    case 'done':
      return { includeDone: true }
    case 'inbox':
      return {}
    default:
      return { plannerId: key.slice('planner:'.length) }
  }
}

export function TasksView() {
  const { taskId, setTaskId, version, bump, fail, toast, openNote } = useApp()
  const [list, setList] = useState<ListKey>('inbox')
  const [tasks, setTasks] = useState<Task[]>([])
  const [planners, setPlanners] = useState<Planner[]>([])
  const [input, setInput] = useState('')

  const load = useCallback(async () => {
    try {
      let ts = await api.listTasks(filterFor(list))
      if (list === 'done') ts = ts.filter((t) => t.status === 'done').sort((a, b) => (b.completedAt ?? 0) - (a.completedAt ?? 0))
      setTasks(ts)
      setPlanners(await api.listPlanners())
    } catch (e) {
      fail(e)
    }
  }, [list, fail])

  useEffect(() => {
    load()
  }, [load, version])

  const selected = useMemo(() => tasks.find((t) => t.id === taskId) ?? null, [tasks, taskId])

  // A task opened from elsewhere (calendar, note panel) may not be in the current list.
  useEffect(() => {
    if (!taskId) return
    api.listTasks({ includeDone: true }).then((all) => {
      const t = all.find((x) => x.id === taskId)
      if (t) setTasks((ts) => (ts.some((x) => x.id === t.id) ? ts : [t, ...ts]))
    })
  }, [taskId, version])

  const add = async () => {
    const title = input.trim()
    if (!title) return
    try {
      const plannerId = list.startsWith('planner:') ? list.slice(8) : null
      const dueAt = list === 'today' ? endOfDay() - MINUTE : null
      const t = await api.createTask({ title, plannerId, dueAt, allDay: list === 'today' })
      setInput('')
      setTasks((ts) => [...ts, t])
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const toggle = async (t: Task) => {
    try {
      if (t.status === 'done') {
        await api.reopenTask(t.id)
      } else {
        const u = await api.completeTask(t.id)
        if (u.status !== 'done' && u.dueAt) toast(`Repeats — next due ${formatDue(u.dueAt, u.allDay)}`)
      }
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const update = async (id: Id, patch: TaskPatch) => {
    try {
      const u = await api.updateTask(id, patch)
      setTasks((ts) => ts.map((x) => (x.id === u.id ? u : x)))
    } catch (e) {
      fail(e)
    }
  }

  const listLabel = LISTS.find((l) => l.key === list)?.label ?? planners.find((p) => `planner:${p.id}` === list)?.name ?? ''
  const now = Date.now()

  return (
    <div className="split" style={{ gridTemplateColumns: selected ? '220px minmax(0,1fr) 340px' : '220px minmax(0,1fr)' }}>
      <div className="pane" style={{ padding: 10 }}>
        {LISTS.map((l) => (
          <button key={l.key} className={'nav-item' + (list === l.key ? ' active' : '')} onClick={() => setList(l.key)}>
            {l.icon} {l.label}
          </button>
        ))}
        {planners.length > 0 && <div className="nav-section">Planners</div>}
        {planners.map((p) => (
          <button key={p.id} className={'nav-item' + (list === `planner:${p.id}` ? ' active' : '')} onClick={() => setList(`planner:${p.id}`)}>
            <span className="prio-dot" style={{ background: p.color }} /> <span className="ellipsis">{p.name}</span>
            <span className="count">{p.taskCount - p.doneCount}</span>
          </button>
        ))}
      </div>

      <div className="pane">
        <div className="pane-header">
          <strong>{listLabel}</strong>
          <span className="faint small">{tasks.length}</span>
        </div>
        {list !== 'done' && (
          <div className="add-row">
            <input
              className="input"
              placeholder="Add a task and press Enter…"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && add()}
            />
          </div>
        )}
        {tasks.map((t) => {
          const overdue = t.status !== 'done' && t.dueAt != null && t.dueAt < (t.allDay ? startOfDay() : now)
          return (
            <div key={t.id} className={'task-row' + (t.id === taskId ? ' active' : '') + (t.status === 'done' ? ' done' : '')} onClick={() => setTaskId(t.id)}>
              <button
                className={`check${t.status === 'done' ? ' done' : ''} p${t.priority}`}
                onClick={(e) => {
                  e.stopPropagation()
                  toggle(t)
                }}
                aria-label={t.status === 'done' ? 'Mark not done' : 'Complete'}
              >
                {t.status === 'done' && <Check size={12} />}
              </button>
              <div className="grow">
                <div className="task-title ellipsis">{t.title}</div>
                <div className="task-meta">
                  {t.dueAt != null && <span className={overdue ? 'overdue' : ''}>{formatDue(t.dueAt, t.allDay)}</span>}
                  {t.recurrence && <Repeat size={11} />}
                  {t.remindAt != null && !t.reminded && <Bell size={11} />}
                  {t.status === 'doing' && <span>In progress</span>}
                  {t.noteId && <StickyNote size={11} />}
                  {t.plannerId && <span>{planners.find((p) => p.id === t.plannerId)?.name}</span>}
                </div>
              </div>
            </div>
          )
        })}
        {!tasks.length && <Empty icon={<CheckCircle2 size={30} />} title={list === 'done' ? 'Nothing completed yet' : 'All clear'} />}
      </div>

      {selected && (
        <div className="pane">
          <TaskDetails
            key={selected.id}
            task={selected}
            planners={planners}
            onUpdate={(p) => update(selected.id, p)}
            onOpenNote={openNote}
            onDelete={async () => {
              try {
                await api.deleteTask(selected.id)
                setTaskId(null)
                bump()
              } catch (e) {
                fail(e)
              }
            }}
          />
        </div>
      )}
    </div>
  )
}

const REMINDER_OFFSETS: { label: string; ms: number }[] = [
  { label: 'At due time', ms: 0 },
  { label: '10 minutes before', ms: 10 * MINUTE },
  { label: '1 hour before', ms: HOUR },
  { label: '1 day before', ms: DAY },
]

function TaskDetails({
  task,
  planners,
  onUpdate,
  onDelete,
  onOpenNote,
}: {
  task: Task
  planners: Planner[]
  onUpdate: (p: TaskPatch) => void
  onDelete: () => void
  onOpenNote: (id: Id) => void
}) {
  const [title, setTitle] = useState(task.title)
  const [notes, setNotes] = useState(task.notes)
  const confirm = useConfirm()

  const setDue = (dueAt: number | null, allDay = task.allDay) => {
    const patch: TaskPatch = { dueAt, allDay }
    if (dueAt == null) {
      patch.recurrence = null
      patch.remindAt = null
    } else if (task.remindAt != null && task.dueAt != null) {
      patch.remindAt = dueAt - (task.dueAt - task.remindAt)
    }
    onUpdate(patch)
  }

  return (
    <div style={{ padding: 18 }}>
      <div className="field">
        <input
          className="input"
          style={{ fontSize: 16, fontWeight: 600 }}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => title.trim() && title !== task.title && onUpdate({ title })}
          onKeyDown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
        />
      </div>
      <div className="row" style={{ marginBottom: 12 }}>
        <div className="field grow" style={{ marginBottom: 0 }}>
          <label>Status</label>
          <select className="select" value={task.status} onChange={(e) => onUpdate({ status: e.target.value as Task['status'] })}>
            <option value="todo">To do</option>
            <option value="doing">In progress</option>
            <option value="done">Done</option>
          </select>
        </div>
        <div className="field grow" style={{ marginBottom: 0 }}>
          <label>Priority</label>
          <select className="select" value={task.priority} onChange={(e) => onUpdate({ priority: Number(e.target.value) })}>
            {PRIORITY_LABELS.map((l, i) => (
              <option key={l} value={i} style={{ color: PRIORITY_COLORS[i] }}>
                {l}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="field">
        <label>Due</label>
        <div className="row">
          <input
            className="input"
            type={task.allDay ? 'date' : 'datetime-local'}
            value={task.allDay ? toDateInput(task.dueAt) : toLocalInput(task.dueAt)}
            onChange={(e) => {
              const v = fromLocalInput(task.allDay && e.target.value ? e.target.value + 'T23:59' : e.target.value)
              setDue(v)
            }}
          />
          {task.dueAt != null && (
            <button className="btn ghost sm" onClick={() => setDue(null)}>Clear</button>
          )}
        </div>
        <div className="row" style={{ marginTop: 4 }}>
          <label className="checkbox">
            <input
              type="checkbox"
              checked={task.allDay}
              onChange={(e) => {
                const allDay = e.target.checked
                const base = task.dueAt ?? Date.now()
                setDue(allDay ? endOfDay(base) - MINUTE : startOfDay(base) + 9 * HOUR, allDay)
              }}
            />
            All day
          </label>
          <span className="grow" />
          <button className="btn ghost sm" onClick={() => setDue(task.allDay ? endOfDay() - MINUTE : Date.now() + HOUR)}>Today</button>
          <button className="btn ghost sm" onClick={() => setDue(task.allDay ? endOfDay(addDays(Date.now(), 1)) - MINUTE : startOfDay(addDays(Date.now(), 1)) + 9 * HOUR)}>
            Tomorrow
          </button>
        </div>
      </div>

      <div className="field">
        <label>Repeat</label>
        <div className="row">
          <select
            className="select"
            disabled={task.dueAt == null}
            value={task.recurrence?.freq ?? ''}
            onChange={(e) =>
              onUpdate({ recurrence: e.target.value ? { freq: e.target.value as Frequency, interval: task.recurrence?.interval ?? 1 } : null })
            }
          >
            <option value="">Doesn't repeat</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
            <option value="monthly">Monthly</option>
            <option value="yearly">Yearly</option>
          </select>
          {task.recurrence && (
            <>
              <span className="small muted">every</span>
              <input
                className="input"
                type="number"
                min={1}
                style={{ width: 70 }}
                value={task.recurrence.interval}
                onChange={(e) => onUpdate({ recurrence: { ...task.recurrence!, interval: Math.max(1, Number(e.target.value) || 1) } })}
              />
            </>
          )}
        </div>
        {task.dueAt == null && <span className="small faint">Set a due date to repeat.</span>}
      </div>

      <div className="field">
        <label>Reminder</label>
        <select
          className="select"
          value={task.remindAt == null ? 'none' : task.dueAt != null ? String(REMINDER_OFFSETS.find((o) => task.dueAt! - o.ms === task.remindAt)?.ms ?? 'custom') : 'custom'}
          onChange={(e) => {
            const v = e.target.value
            if (v === 'none') onUpdate({ remindAt: null })
            else if (v === 'custom') onUpdate({ remindAt: task.remindAt ?? Date.now() + HOUR })
            else onUpdate({ remindAt: (task.dueAt ?? Date.now()) - Number(v) })
          }}
        >
          <option value="none">No reminder</option>
          {task.dueAt != null && REMINDER_OFFSETS.map((o) => <option key={o.ms} value={o.ms}>{o.label}</option>)}
          <option value="custom">Custom time…</option>
        </select>
        {task.remindAt != null && (
          <input className="input" type="datetime-local" value={toLocalInput(task.remindAt)} onChange={(e) => onUpdate({ remindAt: fromLocalInput(e.target.value) })} />
        )}
      </div>

      <div className="field">
        <label>Planner</label>
        <select className="select" value={task.plannerId ?? ''} onChange={(e) => onUpdate({ plannerId: e.target.value || null })}>
          <option value="">None</option>
          {planners.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
        </select>
      </div>

      <div className="field">
        <label>Notes</label>
        <textarea className="textarea" value={notes} onChange={(e) => setNotes(e.target.value)} onBlur={() => notes !== task.notes && onUpdate({ notes })} />
      </div>

      <div className="row">
        {task.noteId && (
          <button className="btn sm" onClick={() => onOpenNote(task.noteId!)}>
            <StickyNote size={13} /> Open linked note
          </button>
        )}
        {task.completionCount > 0 && <span className="small faint">Completed {task.completionCount}×</span>}
        <span className="grow" />
        <button className="btn danger sm" onClick={async () => (await confirm.ask(`Delete “${task.title}”?`, 'Delete')) && onDelete()}>
          <Trash2 size={13} /> Delete
        </button>
      </div>
      {confirm.element}
    </div>
  )
}
