// In-memory stand-in for the Rust backend so the UI can run in a plain browser
// (`./run.sh ui`). Only loaded in dev when Tauri isn't present. Not a reimplementation:
// just enough to click around the interface. Encrypted vaults accept any password.
import { mockIPC } from '@tauri-apps/api/mocks'
import type { CalEvent, Folder, Note, NoteKind, Planner, Task, Template, Vault } from '../api'

type Args = Record<string, any> // eslint-disable-line @typescript-eslint/no-explicit-any

const now = () => Date.now()
const id = () => crypto.randomUUID()
const vaults: Vault[] = [{ id: id(), name: 'Personal', createdAt: now(), encrypted: false, unlocked: true, noteCount: 0 }]
const notes: Note[] = []
const folders: Folder[] = []
const tasks: Task[] = []
const planners: Planner[] = []
const events: CalEvent[] = []
const settings = new Map<string, string>()
const images = new Map<string, { mime: string; data: string }>()
const templates: Template[] = [
  { id: id(), name: 'Meeting Notes', title: 'Meeting — {{date}}', body: '## Agenda\n- \n\n## Notes\n\n## Action Items\n- [ ] ', builtin: true, createdAt: now(), updatedAt: now() },
  { id: id(), name: 'Daily Journal', title: 'Journal — {{date}}', body: '## Today\n\n## Reflection\n', builtin: true, createdAt: now(), updatedAt: now() },
]

const vaultOf = (v?: string | null) => v ?? vaults[0].id
const note = (
  title: string, body: string, tags: string[] = [], pinned = false, kind: NoteKind = 'note', vaultId?: string | null, folderId: string | null = null,
): Note => {
  const v = vaults.find((x) => x.id === vaultOf(vaultId))!
  const n: Note = {
    id: id(), kind, title, body: kind === 'whiteboard' && !body ? '{}' : body, tags, pinned,
    createdAt: now(), updatedAt: now(), archivedAt: null, vaultId: v.id, folderId, encrypted: v.encrypted,
  }
  notes.unshift(n)
  return n
}
const folder = (name: string, parentId: string | null = null, vaultId?: string | null): Folder => {
  const f: Folder = { id: id(), vaultId: vaultOf(vaultId), parentId, name, createdAt: now() }
  folders.push(f)
  return f
}
const work = folder('Work')
const projects = folder('Projects', work.id)
folder('Personal')
note('Welcome to Astralis', 'This is a **browser preview** with a mock backend.\n\nTry linking: [[Project Apollo]] and [[Ideas]].', ['astralis'], true)
note('Project Apollo', 'Goals for the launch. See [[Ideas]].\n\n- [ ] Draft plan\n- [ ] Review', ['work'], false, 'note', null, projects.id)
note('Standup notes', 'Daily standup.', [], false, 'note', null, work.id)
note('Ideas', 'A place for loose ideas.', ['personal'])
note('Brainstorm', '', [], false, 'whiteboard', null, projects.id)

const find = <T extends { id: string }>(list: T[], key: string) => {
  const x = list.find((i) => i.id === key)
  if (!x) throw `not found: ${key}`
  return x
}
const links = (body: string) => [...body.matchAll(/\[\[([^\]|#\n]+)/g)].map((m) => m[1].trim())
const fill = (s: string, title = '') => s.replaceAll('{{date}}', new Date().toISOString().slice(0, 10)).replaceAll('{{title}}', title)
const inVault = (v?: string | null) => notes.filter((n) => n.vaultId === vaultOf(v))
const ref = (n: Note) => ({ id: n.id, title: n.title })

const handlers: Record<string, (a: Args) => unknown> = {
  create_note: ({ input }) =>
    note(input.title ?? '', input.body ?? '', input.tags ?? [], input.pinned ?? false, input.kind ?? 'note', input.vaultId, input.folderId ?? null),
  list_folders: ({ vaultId }) => folders.filter((f) => f.vaultId === vaultOf(vaultId)).sort((a, b) => a.name.localeCompare(b.name)),
  create_folder: ({ vaultId, parentId, name }) => folder(name, parentId, vaultId),
  rename_folder: ({ id: k, name }) => Object.assign(find(folders, k), { name }),
  move_folder: ({ id: k, parentId }) => {
    for (let c: string | null = parentId; c; c = find(folders, c).parentId) if (c === k) throw "can't move a folder into itself"
    return Object.assign(find(folders, k), { parentId })
  },
  delete_folder: ({ id: k }) => {
    const f = find(folders, k)
    notes.forEach((n) => n.folderId === k && (n.folderId = f.parentId))
    folders.forEach((c) => c.parentId === k && (c.parentId = f.parentId))
    folders.splice(folders.indexOf(f), 1)
  },
  move_note_to_folder: ({ noteId, folderId }) => Object.assign(find(notes, noteId), { folderId }),
  create_whiteboard: ({ vaultId, title }) => note(title || 'Untitled whiteboard', '{}', [], false, 'whiteboard', vaultId),
  save_whiteboard: ({ id: k, scene }) => void Object.assign(find(notes, k), { body: scene, updatedAt: now() }),
  quick_capture: ({ vaultId, text }) => note(text.split('\n')[0], text.split('\n').slice(1).join('\n'), [], false, 'note', vaultId),
  get_note: ({ id: k }) => find(notes, k),
  update_note: ({ id: k, patch }) => Object.assign(find(notes, k), patch, { updatedAt: now() }),
  list_notes: ({ filter }) =>
    inVault(filter.vaultId)
      .filter((n) => (filter.archive === 'archived' ? n.archivedAt : !n.archivedAt))
      .filter((n) => !filter.tag || n.tags.includes(filter.tag))
      .filter((n) => !filter.kind || n.kind === filter.kind)
      .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt),
  set_pinned: ({ id: k, pinned }) => Object.assign(find(notes, k), { pinned }),
  archive_note: ({ id: k }) => Object.assign(find(notes, k), { archivedAt: now() }),
  unarchive_note: ({ id: k }) => Object.assign(find(notes, k), { archivedAt: null }),
  delete_note: ({ id: k }) => void notes.splice(notes.indexOf(find(notes, k)), 1),
  add_tag: ({ id: k, tag }) => {
    const n = find(notes, k)
    n.tags = [...new Set([...n.tags, tag.toLowerCase()])].sort()
    return n
  },
  remove_tag: ({ id: k, tag }) => Object.assign(find(notes, k), { tags: find(notes, k).tags.filter((t) => t !== tag) }),
  list_tags: ({ vaultId }) => {
    const counts = new Map<string, number>()
    inVault(vaultId).filter((n) => !n.archivedAt).forEach((n) => n.tags.forEach((t) => counts.set(t, (counts.get(t) ?? 0) + 1)))
    return [...counts].map(([name, count]) => ({ name, count }))
  },
  search: ({ vaultId, query }) =>
    inVault(vaultId)
      .filter((n) => !n.encrypted && (n.title + (n.kind === 'note' ? n.body : '')).toLowerCase().includes(query.toLowerCase()))
      .map((n) => ({ id: n.id, kind: n.kind, title: n.title, snippet: n.kind === 'note' ? n.body.slice(0, 80) : '', archived: !!n.archivedAt, updatedAt: n.updatedAt, score: 1 })),
  backlinks: ({ id: k }) => {
    const me = find(notes, k)
    const t = me.title.toLowerCase()
    return inVault(me.vaultId).filter((n) => links(n.body).some((l) => l.toLowerCase() === t)).map((n) => ({ note: ref(n), context: n.body.split('\n')[0] }))
  },
  outgoing_links: ({ id: k }) => {
    const me = find(notes, k)
    return [...new Set(links(me.body))].map((title) => ({
      title,
      notes: inVault(me.vaultId).filter((n) => n.title.toLowerCase() === title.toLowerCase()).map(ref),
    }))
  },
  find_by_title: ({ vaultId, title }) => inVault(vaultId).filter((n) => n.title.toLowerCase() === title.toLowerCase()).map(ref),
  suggest_titles: ({ vaultId, query }) => inVault(vaultId).filter((n) => n.title.toLowerCase().includes(query.toLowerCase())).map(ref),
  graph: ({ vaultId, includeOrphans }) => {
    const active = inVault(vaultId).filter((n) => !n.archivedAt && !n.encrypted)
    const edges = active.flatMap((s) =>
      links(s.body).flatMap((l) => active.filter((t) => t.title.toLowerCase() === l.toLowerCase()).map((t) => ({ source: s.id, target: t.id }))),
    )
    const nodes = active.map((n) => ({ id: n.id, kind: n.kind, title: n.title, tags: n.tags, pinned: n.pinned, degree: edges.filter((e) => e.source === n.id || e.target === n.id).length }))
    return { nodes: includeOrphans ? nodes : nodes.filter((n) => n.degree > 0), edges }
  },
  note_history: () => [],
  list_templates: () => templates,
  note_from_template: ({ templateId, title, vaultId }) => {
    const t = find(templates, templateId)
    return note(fill(t.title, title ?? ''), fill(t.body, title ?? ''), [], false, 'note', vaultId)
  },
  // vaults
  list_vaults: () => vaults.map((v) => ({ ...v, noteCount: notes.filter((n) => n.vaultId === v.id && !n.archivedAt).length })),
  default_vault: () => vaults[0].id,
  create_vault: ({ name, password }) => {
    const v: Vault = { id: id(), name, createdAt: now(), encrypted: !!password, unlocked: true, noteCount: 0 }
    vaults.push(v)
    return v
  },
  rename_vault: ({ id: k, name }) => Object.assign(find(vaults, k), { name }),
  unlock_vault: ({ id: k }) => Object.assign(find(vaults, k), { unlocked: true }),
  lock_vault: ({ id: k }) => void Object.assign(find(vaults, k), { unlocked: false }),
  lock_all_vaults: () => void vaults.forEach((v) => v.encrypted && (v.unlocked = false)),
  change_vault_password: () => undefined,
  delete_vault: ({ id: k }) => {
    if (vaults.length <= 1) throw "can't delete the only vault"
    vaults.splice(vaults.indexOf(find(vaults, k)), 1)
    for (let i = notes.length - 1; i >= 0; i--) if (notes[i].vaultId === k) notes.splice(i, 1)
  },
  move_note_to_vault: ({ noteId, vaultId }) => Object.assign(find(notes, noteId), { vaultId, folderId: null, encrypted: find(vaults, vaultId).encrypted }),
  // tasks
  create_task: ({ input }) => {
    const t: Task = {
      id: id(), title: input.title, notes: input.notes ?? '', status: input.status ?? 'todo', priority: input.priority ?? 0,
      dueAt: input.dueAt ?? null, allDay: input.allDay ?? false, remindAt: input.remindAt ?? null, reminded: false,
      recurrence: input.recurrence ?? null, plannerId: input.plannerId ?? null, noteId: input.noteId ?? null,
      completedAt: null, completionCount: 0, position: tasks.length, createdAt: now(), updatedAt: now(),
    }
    tasks.push(t)
    return t
  },
  update_task: ({ id: k, patch }) => Object.assign(find(tasks, k), patch),
  complete_task: ({ id: k }) => Object.assign(find(tasks, k), { status: 'done', completedAt: now() }),
  reopen_task: ({ id: k }) => Object.assign(find(tasks, k), { status: 'todo', completedAt: null }),
  delete_task: ({ id: k }) => void tasks.splice(tasks.indexOf(find(tasks, k)), 1),
  list_tasks: ({ filter }) =>
    tasks.filter(
      (t) =>
        (filter.includeDone || t.status !== 'done') &&
        (!filter.plannerId || t.plannerId === filter.plannerId) &&
        (!filter.noteId || t.noteId === filter.noteId) &&
        (!filter.dueBefore || (t.dueAt != null && t.dueAt < filter.dueBefore)) &&
        (!filter.noDueDate || t.dueAt == null),
    ),
  list_planners: () =>
    planners.map((p) => ({ ...p, taskCount: tasks.filter((t) => t.plannerId === p.id).length, doneCount: tasks.filter((t) => t.plannerId === p.id && t.status === 'done').length })),
  create_planner: ({ input }) => {
    const p: Planner = { id: id(), name: input.name, description: input.description ?? '', color: input.color ?? '#8b7bff', createdAt: now(), updatedAt: now(), taskCount: 0, doneCount: 0 }
    planners.push(p)
    return p
  },
  create_event: ({ input }) => {
    const e: CalEvent = { id: id(), description: '', location: '', allDay: false, remindAt: null, color: null, ...input, reminded: false, createdAt: now(), updatedAt: now() }
    events.push(e)
    return e
  },
  update_event: ({ id: k, input }) => Object.assign(find(events, k), input),
  delete_event: ({ id: k }) => void events.splice(events.indexOf(find(events, k)), 1),
  get_event: ({ id: k }) => find(events, k),
  calendar: ({ start, end }) => [
    ...events.filter((e) => e.startAt < end && e.endAt >= start).map((e) => ({ kind: 'event', id: e.id, title: e.title, start: e.startAt, end: e.endAt, allDay: e.allDay, done: false, priority: null, color: e.color, projected: false })),
    ...tasks.filter((t) => t.dueAt != null && t.dueAt >= start && t.dueAt < end).map((t) => ({ kind: 'task', id: t.id, title: t.title, start: t.dueAt, end: null, allDay: t.allDay, done: t.status === 'done', priority: t.priority, color: null, projected: false })),
  ],
  get_setting: ({ key }) => settings.get(key) ?? null,
  set_setting: ({ key, value }) => void settings.set(key, value),
  check_grammar: () => [],
  brain_status: () => ({ loaded: false, model: 'mock', neural: false, indexed: notes.length, pending: 0 }),
  related_notes: () => [],
  suggest_tags: () => [{ tag: 'example', score: 0.5, source: 'keyword' }],
  summarize_note: ({ id: k }) => ({ sentences: [], text: find(notes, k).body.split(/[.\n]/)[0] }),
  add_attachment: ({ vaultId, name, mime, data }) => {
    const id = crypto.randomUUID()
    images.set(id, { mime, data })
    const ext = mime.split('/')[1]?.replace('jpeg', 'jpg').replace('svg+xml', 'svg') ?? 'bin'
    return { id, vaultId: vaultOf(vaultId), name, mime, size: data.length, path: `attachments/${id}.${ext}`, createdAt: now() }
  },
  get_attachment: ({ id: k }) => {
    const img = images.get(k)
    if (!img) throw 'attachment not found'
    return img
  },
  p2p_status: () => ({ ready: true, id: 'mock-device', name: 'This device', peers: [] }),
  p2p_set_name: () => undefined,
  p2p_start_pairing: () => ({ code: '482913', invite: 'astralis:mockdeviceid:482913' }),
  p2p_cancel_pairing: () => undefined,
  p2p_join: () => { throw 'no Astralis devices found nearby — make sure both are on the same network' },
  p2p_unpair: () => undefined,
  live_text: () => undefined,
  live_presence: () => undefined,
  live_scene: () => undefined,
  tidy_note: ({ text }) => {
    const body = text.replace(/^\* /gm, '- ').replace(/\n{3,}/g, '\n\n')
    return { body, changes: body === text ? [] : ['Made bullet points consistent', 'Cleaned up spacing'] }
  },
  organize_notes: () => [],
  find_duplicates: () => [],
}

mockIPC(
  (cmd, args) => {
    if (cmd.startsWith('plugin:notification')) return true
    if (cmd.startsWith('plugin:')) return 0
    const h = handlers[cmd]
    if (!h) {
      console.warn('[mock] unhandled command', cmd, args)
      return null
    }
    return structuredClone(h((args ?? {}) as Args))
  },
  { shouldMockEvents: true },
)
console.info('[Astralis] running with the in-memory mock backend (browser preview)')
