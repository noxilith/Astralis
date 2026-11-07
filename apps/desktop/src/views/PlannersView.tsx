import { useCallback, useEffect, useState } from 'react'
import { Kanban, Pencil, Plus, Trash2 } from 'lucide-react'
import { api, type Planner, type Task, type TaskStatus } from '../api'
import { useApp } from '../context'
import { Empty, Modal, PRIORITY_COLORS, useConfirm } from '../components/ui'
import { formatDue } from '../lib/time'

const COLUMNS: { status: TaskStatus; label: string }[] = [
  { status: 'todo', label: 'To do' },
  { status: 'doing', label: 'In progress' },
  { status: 'done', label: 'Done' },
]
const COLORS = ['#8b7bff', '#4fd1a5', '#6bb4ff', '#ffb547', '#ff6b81', '#c792ea']

export function PlannersView() {
  const { plannerId, openPlanner, openTask, version, bump, fail } = useApp()
  const [planners, setPlanners] = useState<Planner[]>([])
  const [tasks, setTasks] = useState<Task[]>([])
  const [form, setForm] = useState<{ planner?: Planner; name: string; description: string; color: string } | null>(null)
  const [dragId, setDragId] = useState<string | null>(null)
  const [dropCol, setDropCol] = useState<TaskStatus | null>(null)
  const [inputs, setInputs] = useState<Record<string, string>>({})
  const confirm = useConfirm()

  const load = useCallback(async () => {
    try {
      const ps = await api.listPlanners()
      setPlanners(ps)
      const active = ps.find((p) => p.id === plannerId) ?? ps[0]
      if (active && active.id !== plannerId) openPlanner(active.id)
      setTasks(active ? await api.listTasks({ plannerId: active.id, includeDone: true }) : [])
    } catch (e) {
      fail(e)
    }
  }, [plannerId, openPlanner, fail])

  useEffect(() => {
    load()
  }, [load, version])

  const active = planners.find((p) => p.id === plannerId) ?? null

  const moveTo = async (taskId: string, status: TaskStatus) => {
    const t = tasks.find((x) => x.id === taskId)
    if (!t || t.status === status) return
    try {
      await api.updateTask(taskId, { status })
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const addCard = async (status: TaskStatus) => {
    const title = (inputs[status] ?? '').trim()
    if (!title || !active) return
    try {
      await api.createTask({ title, status, plannerId: active.id })
      setInputs((i) => ({ ...i, [status]: '' }))
      bump()
    } catch (e) {
      fail(e)
    }
  }

  const saveForm = async () => {
    if (!form || !form.name.trim()) return
    try {
      const input = { name: form.name, description: form.description, color: form.color }
      const p = form.planner ? await api.updatePlanner(form.planner.id, input) : await api.createPlanner(input)
      setForm(null)
      openPlanner(p.id)
      bump()
    } catch (e) {
      fail(e)
    }
  }

  return (
    <div className="split split-2">
      <div className="pane" style={{ padding: 10 }}>
        <div className="row" style={{ padding: '4px 6px 10px' }}>
          <strong className="grow">Planners</strong>
          <button className="icon-btn" title="New planner" onClick={() => setForm({ name: '', description: '', color: COLORS[0] })}>
            <Plus size={16} />
          </button>
        </div>
        {planners.map((p) => (
          <button key={p.id} className={'nav-item' + (p.id === plannerId ? ' active' : '')} onClick={() => openPlanner(p.id)} style={{ flexDirection: 'column', alignItems: 'stretch', gap: 6 }}>
            <span className="row">
              <span className="prio-dot" style={{ background: p.color }} />
              <span className="grow ellipsis">{p.name}</span>
              <span className="count">{p.doneCount}/{p.taskCount}</span>
            </span>
            <span className="progress"><div style={{ width: `${p.taskCount ? (p.doneCount / p.taskCount) * 100 : 0}%`, background: p.color }} /></span>
          </button>
        ))}
      </div>

      <div className="pane" style={{ display: 'flex', flexDirection: 'column' }}>
        {active ? (
          <>
            <div className="view-header">
              <span className="prio-dot" style={{ background: active.color, width: 12, height: 12 }} />
              <div className="grow">
                <h1>{active.name}</h1>
                {active.description && <div className="small muted">{active.description}</div>}
              </div>
              <button className="icon-btn" title="Edit planner" onClick={() => setForm({ planner: active, name: active.name, description: active.description, color: active.color })}>
                <Pencil size={16} />
              </button>
              <button
                className="icon-btn"
                title="Delete planner"
                onClick={async () => {
                  if (await confirm.ask(`Delete planner “${active.name}”? Its tasks are kept.`, 'Delete')) {
                    api.deletePlanner(active.id).then(() => (openPlanner(null), bump())).catch(fail)
                  }
                }}
              >
                <Trash2 size={16} />
              </button>
            </div>
            <div className="board" style={{ flex: 1 }}>
              {COLUMNS.map((col) => (
                <div
                  key={col.status}
                  className={'column' + (dropCol === col.status ? ' drop' : '')}
                  onDragOver={(e) => {
                    e.preventDefault()
                    setDropCol(col.status)
                  }}
                  onDragLeave={() => setDropCol(null)}
                  onDrop={(e) => {
                    e.preventDefault()
                    setDropCol(null)
                    const id = e.dataTransfer.getData('text/plain')
                    if (id) moveTo(id, col.status)
                  }}
                >
                  <div className="column-header">
                    {col.label} <span className="faint small">{tasks.filter((t) => t.status === col.status).length}</span>
                  </div>
                  <div className="column-body">
                    {tasks
                      .filter((t) => t.status === col.status)
                      .map((t) => (
                        <div
                          key={t.id}
                          className={'card' + (dragId === t.id ? ' dragging' : '')}
                          draggable
                          onDragStart={(e) => {
                            e.dataTransfer.setData('text/plain', t.id)
                            setDragId(t.id)
                          }}
                          onDragEnd={() => setDragId(null)}
                          onDoubleClick={() => openTask(t.id)}
                          title="Drag to move · double-click to edit"
                        >
                          <div className="row">
                            {t.priority > 0 && <span className="prio-dot" style={{ background: PRIORITY_COLORS[t.priority] }} />}
                            <span className="grow" style={{ textDecoration: t.status === 'done' ? 'line-through' : undefined }}>{t.title}</span>
                          </div>
                          {t.dueAt != null && <span className="small faint">{formatDue(t.dueAt, t.allDay)}</span>}
                        </div>
                      ))}
                    <input
                      className="input"
                      placeholder="+ Add card"
                      value={inputs[col.status] ?? ''}
                      onChange={(e) => setInputs((i) => ({ ...i, [col.status]: e.target.value }))}
                      onKeyDown={(e) => e.key === 'Enter' && addCard(col.status)}
                    />
                  </div>
                </div>
              ))}
            </div>
          </>
        ) : (
          <Empty icon={<Kanban size={32} />} title="No planners yet">
            <button className="btn primary" onClick={() => setForm({ name: '', description: '', color: COLORS[0] })}>Create a planner</button>
          </Empty>
        )}
      </div>

      {form && (
        <Modal
          title={form.planner ? 'Edit planner' : 'New planner'}
          onClose={() => setForm(null)}
          footer={
            <>
              <button className="btn" onClick={() => setForm(null)}>Cancel</button>
              <button className="btn primary" disabled={!form.name.trim()} onClick={saveForm}>Save</button>
            </>
          }
        >
          <div className="field">
            <label>Name</label>
            <input className="input" autoFocus value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} onKeyDown={(e) => e.key === 'Enter' && saveForm()} />
          </div>
          <div className="field">
            <label>Description</label>
            <textarea className="textarea" value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />
          </div>
          <div className="row">
            {COLORS.map((c) => (
              <button
                key={c}
                aria-label={`Color ${c}`}
                onClick={() => setForm({ ...form, color: c })}
                style={{ width: 22, height: 22, borderRadius: '50%', background: c, cursor: 'pointer', border: form.color === c ? '2px solid var(--text)' : '2px solid transparent' }}
              />
            ))}
          </div>
        </Modal>
      )}
      {confirm.element}
    </div>
  )
}
