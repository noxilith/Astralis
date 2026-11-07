import { useCallback, useEffect, useMemo, useState } from 'react'
import { ChevronLeft, ChevronRight, Plus } from 'lucide-react'
import { api, type CalEvent, type CalendarItem, type EventInput } from '../api'
import { useApp } from '../context'
import { Modal, useConfirm } from '../components/ui'
import { DAY, HOUR, MINUTE, addDays, fromLocalInput, startOfDay, toDateInput, toLocalInput } from '../lib/time'

const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
const COLORS = ['#8b7bff', '#4fd1a5', '#6bb4ff', '#ffb547', '#ff6b81', '#c792ea']

/** Monday-first 6-week grid covering the month. */
function monthGrid(year: number, month: number) {
  const first = new Date(year, month, 1)
  const offset = (first.getDay() + 6) % 7
  const start = startOfDay(new Date(year, month, 1 - offset))
  return Array.from({ length: 42 }, (_, i) => addDays(start, i))
}

type Editing = { event?: CalEvent; input: EventInput }

export function CalendarView() {
  const { version, bump, fail, openTask } = useApp()
  const [cursor, setCursor] = useState(() => {
    const d = new Date()
    return { year: d.getFullYear(), month: d.getMonth() }
  })
  const [items, setItems] = useState<CalendarItem[]>([])
  const [editing, setEditing] = useState<Editing | null>(null)
  const confirm = useConfirm()

  const days = useMemo(() => monthGrid(cursor.year, cursor.month), [cursor])

  const load = useCallback(() => {
    api.calendar(days[0], addDays(days[41], 1)).then(setItems).catch(fail)
  }, [days, fail])

  useEffect(() => {
    load()
  }, [load, version])

  const byDay = useMemo(() => {
    const map = new Map<number, CalendarItem[]>()
    for (const it of items) {
      const startDay = startOfDay(it.start)
      const endDay = it.end ? startOfDay(Math.max(it.start, it.end - 1)) : startDay
      for (let d = startDay; d <= endDay; d = addDays(d, 1)) {
        if (!map.has(d)) map.set(d, [])
        map.get(d)!.push(it)
      }
    }
    return map
  }, [items])

  const move = (delta: number) =>
    setCursor((c) => {
      const d = new Date(c.year, c.month + delta, 1)
      return { year: d.getFullYear(), month: d.getMonth() }
    })

  const newEvent = (day: number) => {
    const start = day + 9 * HOUR
    setEditing({ input: { title: '', startAt: start, endAt: start + HOUR, allDay: false, color: COLORS[0] } })
  }

  const save = async () => {
    if (!editing) return
    try {
      if (editing.event) await api.updateEvent(editing.event.id, editing.input)
      else await api.createEvent(editing.input)
      setEditing(null)
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const today = startOfDay()
  const monthName = new Date(cursor.year, cursor.month, 1).toLocaleDateString(undefined, { month: 'long', year: 'numeric' })
  const set = (patch: Partial<EventInput>) => setEditing((e) => (e ? { ...e, input: { ...e.input, ...patch } } : e))

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div className="view-header">
        <h1>{monthName}</h1>
        <button className="icon-btn" onClick={() => move(-1)} aria-label="Previous month"><ChevronLeft size={18} /></button>
        <button className="icon-btn" onClick={() => move(1)} aria-label="Next month"><ChevronRight size={18} /></button>
        <button className="btn sm" onClick={() => setCursor({ year: new Date().getFullYear(), month: new Date().getMonth() })}>Today</button>
        <span className="spacer" />
        <button className="btn primary sm" onClick={() => newEvent(today)}><Plus size={14} /> Event</button>
      </div>
      <div className="cal-head">
        {WEEKDAYS.map((d) => <div key={d}>{d}</div>)}
      </div>
      <div className="view-body">
        <div className="cal">
          {days.map((d) => {
            const dayItems = byDay.get(d) ?? []
            const other = new Date(d).getMonth() !== cursor.month
            return (
              <div key={d} className={'cal-day' + (other ? ' other' : '') + (d === today ? ' today' : '')} onClick={() => newEvent(d)}>
                <span className="num">{new Date(d).getDate()}</span>
                {dayItems.slice(0, 4).map((it, i) => (
                  <div
                    key={it.id + it.start + i}
                    className={`cal-item ${it.kind}${it.done ? ' done' : ''}${it.projected ? ' projected' : ''}`}
                    style={it.kind === 'event' ? { background: (it.color ?? COLORS[0]) + '33', borderLeft: `3px solid ${it.color ?? COLORS[0]}` } : undefined}
                    title={it.title}
                    onClick={async (e) => {
                      e.stopPropagation()
                      if (it.kind === 'task') return openTask(it.id)
                      try {
                        const ev = await api.getEvent(it.id)
                        setEditing({ event: ev, input: { ...ev } })
                      } catch (err) {
                        fail(err)
                      }
                    }}
                  >
                    {!it.allDay && <span className="faint">{new Date(it.start).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })} </span>}
                    {it.kind === 'task' ? '☐ ' : ''}
                    {it.title}
                  </div>
                ))}
                {dayItems.length > 4 && <span className="small faint">+{dayItems.length - 4} more</span>}
              </div>
            )
          })}
        </div>
      </div>

      {editing && (
        <Modal
          title={editing.event ? 'Edit event' : 'New event'}
          onClose={() => setEditing(null)}
          footer={
            <>
              {editing.event && (
                <button
                  className="btn danger"
                  onClick={async () => {
                    if (await confirm.ask(`Delete “${editing.event!.title}”?`, 'Delete')) {
                      api.deleteEvent(editing.event!.id).then(() => (setEditing(null), bump())).catch(fail)
                    }
                  }}
                >
                  Delete
                </button>
              )}
              <span className="grow" />
              <button className="btn" onClick={() => setEditing(null)}>Cancel</button>
              <button className="btn primary" disabled={!editing.input.title.trim()} onClick={save}>Save</button>
            </>
          }
        >
          <div className="field">
            <label>Title</label>
            <input className="input" autoFocus value={editing.input.title} onChange={(e) => set({ title: e.target.value })} onKeyDown={(e) => e.key === 'Enter' && save()} />
          </div>
          <label className="checkbox" style={{ marginBottom: 12 }}>
            <input
              type="checkbox"
              checked={!!editing.input.allDay}
              onChange={(e) => {
                const allDay = e.target.checked
                const s = startOfDay(editing.input.startAt)
                set(allDay ? { allDay, startAt: s, endAt: s + DAY - MINUTE } : { allDay, startAt: s + 9 * HOUR, endAt: s + 10 * HOUR })
              }}
            />
            All day
          </label>
          <div className="row">
            <div className="field grow">
              <label>Starts</label>
              <input
                className="input"
                type={editing.input.allDay ? 'date' : 'datetime-local'}
                value={editing.input.allDay ? toDateInput(editing.input.startAt) : toLocalInput(editing.input.startAt)}
                onChange={(e) => {
                  const v = fromLocalInput(editing.input.allDay ? e.target.value + 'T00:00' : e.target.value)
                  if (v == null) return
                  const dur = editing.input.endAt - editing.input.startAt
                  set({ startAt: v, endAt: v + dur })
                }}
              />
            </div>
            <div className="field grow">
              <label>Ends</label>
              <input
                className="input"
                type={editing.input.allDay ? 'date' : 'datetime-local'}
                value={editing.input.allDay ? toDateInput(editing.input.endAt) : toLocalInput(editing.input.endAt)}
                onChange={(e) => {
                  const v = fromLocalInput(editing.input.allDay ? e.target.value + 'T23:59' : e.target.value)
                  if (v != null) set({ endAt: v })
                }}
              />
            </div>
          </div>
          <div className="field">
            <label>Reminder</label>
            <select
              className="select"
              value={editing.input.remindAt == null ? 'none' : String(editing.input.startAt - editing.input.remindAt)}
              onChange={(e) => set({ remindAt: e.target.value === 'none' ? null : editing.input.startAt - Number(e.target.value) })}
            >
              <option value="none">No reminder</option>
              <option value="0">At start</option>
              <option value={String(10 * MINUTE)}>10 minutes before</option>
              <option value={String(HOUR)}>1 hour before</option>
              <option value={String(DAY)}>1 day before</option>
            </select>
          </div>
          <div className="field">
            <label>Location</label>
            <input className="input" value={editing.input.location ?? ''} onChange={(e) => set({ location: e.target.value })} />
          </div>
          <div className="field">
            <label>Description</label>
            <textarea className="textarea" value={editing.input.description ?? ''} onChange={(e) => set({ description: e.target.value })} />
          </div>
          <div className="row">
            {COLORS.map((c) => (
              <button
                key={c}
                aria-label={`Color ${c}`}
                onClick={() => set({ color: c })}
                style={{ width: 22, height: 22, borderRadius: '50%', background: c, cursor: 'pointer', border: editing.input.color === c ? '2px solid var(--text)' : '2px solid transparent' }}
              />
            ))}
          </div>
        </Modal>
      )}
      {confirm.element}
    </div>
  )
}
