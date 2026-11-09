// In-memory stand-in for the Rust backend so the UI can run in a plain browser
// (`./run.sh ui`). Only loaded in dev when Tauri isn't present. Not a reimplementation:
// just enough to click around the interface. Encrypted vaults accept any password.
import { mockIPC } from '@tauri-apps/api/mocks'
import * as Y from 'yjs'
import type { CalEvent, Folder, Note, NoteKind, Planner, Task, Template, Vault } from '../api'

type Args = Record<string, any> // eslint-disable-line @typescript-eslint/no-explicit-any

const now = () => Date.now()
const id = () => crypto.randomUUID()
const vaults: Vault[] = [{ id: id(), name: 'Personal', createdAt: now(), encrypted: false, unlocked: true, noteCount: 0, owner: null, sharedWith: [], cloud: null }]
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

let assistantReady = false
let agentsAccess = 'edit'
const mockMe = { name: "noxilith", color: "#8b7bff" }
const google: { clientId: string | null; email: string | null } = { clientId: null, email: null }
const invites = new Map<string, { email: string; acceptedBy: string | null }[]>()
const granted = new Set<string>()
const mem = (folder: string, project: string, kind: string, hook: string, source: string, isNew: boolean, recallCount = 0, daysAgo = 1) => ({
  id: id(), short: Math.random().toString(16).slice(2, 8), folder, project, kind, hook, content: '', source, new: isNew, pinned: false,
  updatedAt: now() - daysAgo * 86400000, recalledAt: recallCount ? now() - 3600000 : null, recallCount,
})
const skills = [
  { target: 'claude', path: '/home/you/.claude/skills/astralis-brain/SKILL.md', state: 'missing' },
  { target: 'codex', path: '/home/you/.codex/skills/astralis-brain/SKILL.md', state: 'outdated' },
]
const brainMems = [
  mem('me', 'Me', 'preference', 'Explain changes in plain words, no jargon', 'claude-code', false, 31),
  mem('me', 'Me', 'preference', 'Uses fish shell; give bash blocks for commands', 'claude-code', false, 12),
  mem('astralis', 'astralis', 'gotcha', 'Python is broken on this machine — script edits with node', 'claude-code', false, 9),
  mem('astralis', 'astralis', 'gotcha', 'python3 fails (encodings missing); use node instead', 'codex', true, 0),
  mem('astralis', 'astralis', 'decision', 'Sync is per vault; peers can never write outside shared vaults', 'claude-code', false, 7),
  mem('astralis', 'astralis', 'howto', 'Run the app with ./run.sh app; second window: ./run.sh second', 'claude-code', true, 4),
  mem('astralis', 'astralis', 'context', 'Old WebGL graph experiment (abandoned)', 'codex', false, 0, 80),
  mem('astralis', 'astralis', 'decision', 'Note text merges as a yrs CRDT; the editor binds to Yjs', 'claude-code', true, 2),
]
const extraMems = (() => {
  const kinds = ['decision', 'gotcha', 'howto', 'fact', 'context', 'preference']
  const projects: [string, number][] = [['astralis', 40], ['website', 22], ['dotfiles', 12], ['Me', 6]]
  const out: ReturnType<typeof mem>[] = []
  for (const [p, n] of projects) for (let i = 0; i < n; i++) out.push(mem(p === 'Me' ? 'me' : p, p, kinds[(i * 5 + p.length) % kinds.length], `${p} memory ${i + 1}`, i % 3 ? 'claude-code' : 'codex', i % 7 === 0, (i * 3) % 9))
  return out
})()
const docs = new Map<string, Y.Doc>()
const docFor = (noteId: string) => {
  const n = notes.find((x) => x.id === noteId)!
  let d = docs.get(noteId)
  if (!d || d.getText('body').toString() !== n.body) {
    d = new Y.Doc()
    d.getText('body').insert(0, n.body)
    docs.set(noteId, d)
  }
  return d
}

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
note('Welcome to Astralis', 'Your notes, as a universe. Everything you write stays on this computer.\n\n## Start here\n- **New note**: Ctrl+N, or the **New** button\n- **Link notes** with double brackets, like [[Project Apollo]] and [[Ideas]]\n- **Ask Astralis**: open the side panel and switch to **Chat**\n- **See your universe**: the graph turns your notes into galaxies\n\n## Try it\n- [x] Open Astralis\n- [ ] Write your first note\n- [ ] Link it to another one\n- [ ] Ask Astralis if your idea will pay off\n\n> **Tip:** Ctrl+K searches everything.', ['astralis', 'start'], true)
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
    const v: Vault = { id: id(), name, createdAt: now(), encrypted: !!password, unlocked: true, noteCount: 0, owner: null, sharedWith: [], cloud: null }
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
  // `id` lets the preview pretend an image arrives from someone else (it's chosen by the app otherwise).
  add_attachment: ({ vaultId, name, mime, data, id: given }) => {
    const id = typeof given === 'string' ? given : crypto.randomUUID()
    images.set(id, { mime, data })
    const ext = mime.split('/')[1]?.replace('jpeg', 'jpg').replace('svg+xml', 'svg') ?? 'bin'
    return { id, vaultId: vaultOf(vaultId), name, mime, size: data.length, path: `attachments/${id}.${ext}`, createdAt: now() }
  },
  get_attachment: ({ id: k }) => {
    const img = images.get(k)
    if (!img) throw 'attachment not found'
    return img
  },
  p2p_status: () => ({ ready: true, id: 'mock-device', name: mockMe.name, color: mockMe.color, peers: [] }),
  p2p_set_name: ({ name }) => void (mockMe.name = name),
  // Google Drive (pretend): set a client, "connect", make cloud vaults, invite.
  google_status: () => ({ configured: !!google.clientId, builtin: false, clientId: google.clientId, email: google.email }),
  google_set_client: ({ clientId }) => void (google.clientId = clientId),
  google_connect: () => (google.email = 'you@gmail.com'),
  google_disconnect: () => void (google.email = null),
  cloud_create_vault: ({ name }) => {
    const v: Vault = { id: id(), name, createdAt: now(), encrypted: false, unlocked: true, noteCount: 0, owner: null, sharedWith: [], cloud: 'google-drive' }
    vaults.push(v)
    return v
  },
  cloud_invites: ({ vaultId }) => invites.get(vaultId) ?? [],
  cloud_invite: ({ vaultId, email }) => {
    if (email.includes('third') && !granted.has(vaultId)) {
      granted.add(vaultId) // pretend everyone updates before the next try
      throw 'invalid input: members-files-in-folder: someone in this vault still has files in its Drive folder from an older Astralis.'
    }
    invites.set(vaultId, [...(invites.get(vaultId) ?? []), { email, acceptedBy: null }])
  },
  cloud_uninvite: ({ vaultId, email }) => void invites.set(vaultId, (invites.get(vaultId) ?? []).filter((i: { email: string }) => i.email !== email)),
  cloud_join: () => 'Shared vault',
  cloud_sync_now: () => undefined,
  // Ask Astralis (pretend: canned replies).
  assistant_status: () => ({ ready: assistantReady, downloading: false, loaded: assistantReady, quickPrompts: [
    ['Be brutally honest', 'Is this a good idea? Be brutally honest.'],
    ['Will it pay off?', 'Will this pay off?'],
    ['Make a plan', 'Make a concrete plan.'],
    ['Write it up', 'Rewrite this note so it explains the idea, lists its features and ends with a plan.'],
    ['Give me ideas', 'Give me some ideas worth building.'],
  ] }),
  assistant_download: () => void (assistantReady = true),
  assistant_chat: ({ message }) =>
    /idea/i.test(message)
      ? "### 1. Plant Doctor — snap a photo of a sick houseplant, get the likely cause\n**Why it could work:** new plant owners panic and search for help.\n**Competition:** crowded (PictureThis, Planta).\n**Will it pay off?** Long shot — 2–3 months for a likely few hundred users.\n\n### 2. <your city> Gig Board — local one-off jobs for students\n**Why it could work:** small, local, easy to test.\n**Competition:** Craigslist, Facebook groups.\n**Will it pay off?** Worth it as a learning project — a weekend to build.\n\n**My pick:** the gig board: you can test it this week with a simple form."
      : /pay off|honest/i.test(message)
      ? "**Verdict:** Weak idea — fun premise, but nothing yet that makes players pick it.\n\nA cozy farming game is one of the most crowded genres on Steam, and \"farming + space\" alone isn't a hook players will notice.\n\n**ROI:** 12–18 months solo for a likely few thousand sales. Unless you can show a unique mechanic in a 30-second clip, it won't pay off.\n\n**Biggest risks**\n- No clear reason to pick it over Stardew Valley\n- Scope creep (crops × planets × seasons)\n\n**Make it better:** prototype the one mechanic that's new (e.g. low-gravity crops) in 2 weeks and test it on 20 players before committing."
      : "# Starfield Farm\n\nA cozy farming game set on small planets, where gravity and day length change how crops grow.\n\n## Features\n- Planet-hopping farms with different gravity\n- Crops that adapt to each planet\n- Trading between colonies\n\n## Plan\n1. **Weeks 1–2:** prototype low-gravity crop growth\n2. **Weeks 3–6:** one planet, full farming loop\n3. **Weeks 7–10:** second planet + trading\n4. **Weeks 11–12:** polish, Steam page, demo\n\n**First step today:** sketch the crop-growth rules on paper.",
  assistant_cancel: () => undefined,
  assistant_unload: () => undefined,
  agents_notes_access: () => agentsAccess,
  agents_set_notes_access: ({ access }) => void (agentsAccess = access),
  // Updates (pretend: a newer release is out).
  update_check: () => ({ current: '1.0.0', latest: '1.1.0', canInstall: true, install: 'AppImage', notes: '## What’s new\n- Faster galaxy graph\n- Ask Astralis remembers chats\n- Fixes for shared vaults', page: 'https://github.com/noxilith/Astralis/releases/latest', size: 98_400_000 }),
  update_install: () => new Promise((_, reject) => setTimeout(() => reject(new Error('(Preview) Updating only works in the desktop app.')), 1500)),
  // The Brain (pretend).
  brain_projects: () => [
    { folder: 'me', name: 'Me', count: 2, newCount: 0 },
    { folder: 'astralis', name: 'astralis', count: brainMems.length - 2, newCount: brainMems.filter((x) => x.new && x.folder === 'astralis').length },
  ],
  brain_memories: ({ folder }) => brainMems.filter((x) => !folder || x.folder === folder),
  brain_set_reviewed: ({ id: k, reviewed }) => Object.assign(find(brainMems, k), { new: !reviewed }),
  brain_forget: ({ id: k }) => void brainMems.splice(brainMems.indexOf(find(brainMems, k)), 1),
  brain_tidy: () => ({
    duplicates: [{ a: brainMems[2].id, b: brainMems[3].id, aHook: brainMems[2].hook, bHook: brainMems[3].hook, score: 0.91 }],
    similar: [],
    stale: [brainMems[6].id],
  }),
  brain_merge: ({ keep }) => find(brainMems, keep),
  brain_compact: ({ ids }) => find(brainMems, ids[0]),
  brain_graph: () => {
    const nodes = [...brainMems, ...extraMems]
    const links: { a: string; b: string; strength: number }[] = []
    nodes.forEach((n, i) => {
      for (let k = 1; k <= 2; k++) {
        const j = (i * 7 + k * 13) % nodes.length
        if (j !== i && (nodes[j].project === n.project || (i + k) % 5 === 0)) links.push({ a: n.id, b: nodes[j].id, strength: 0.3 + ((i * k) % 5) / 10 })
      }
    })
    return { nodes, links }
  },
  brain_stats: () => ({ recalls: 148, tokensServed: 23100, tokensSaved: 412800 }),
  brain_setup: () => ({ server: '/home/you/Projects/Astralis/target/debug/astralis-mcp', serverExists: true, args: [], mirror: '/home/you/Astralis Brain', vault: 'brain' }),
  brain_set_mirror: ({ enabled }) => (enabled ? '/home/you/Astralis Brain' : null),
  brain_skill_status: () => skills,
  brain_install_skill: ({ target }) => {
    const s = skills.find((x) => x.target === target)!
    s.state = 'installed'
    return s.path
  },
  brain_skill_text: () => '---\nname: astralis-brain\n…',
  open_url: ({ url }) => void window.open(url, '_blank'),
  vault_share: ({ vaultId, peer }) => { const v = find(vaults, vaultId); v.sharedWith = [...new Set([...v.sharedWith, peer])]; return v },
  vault_unshare: ({ vaultId, peer }) => { const v = find(vaults, vaultId); v.sharedWith = v.sharedWith.filter((p: string) => p !== peer); return v },
  vault_leave: ({ vaultId }) => void vaults.splice(vaults.indexOf(find(vaults, vaultId)), 1),
  p2p_set_color: ({ color }) => void (mockMe.color = color),
  p2p_start_pairing: () => ({ code: '482913', invite: 'astralis:mockdeviceid:482913' }),
  p2p_cancel_pairing: () => undefined,
  p2p_join: () => { throw 'no Astralis devices found nearby — make sure both are on the same network' },
  p2p_unpair: () => undefined,
  // Text CRDTs, rebuilt from the body when it was changed outside the editor.
  note_crdt: ({ noteId }) => Array.from(Y.encodeStateAsUpdate(docFor(noteId))),
  note_edit: ({ noteId, update }) => {
    const d = docFor(noteId)
    Y.applyUpdate(d, Uint8Array.from(update))
    const n = find(notes, noteId)
    const body = d.getText('body').toString()
    if (body === n.body) return null
    Object.assign(n, { body, updatedAt: now() })
    return body
  },
  live_awareness: () => undefined,
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
