// Typed bridge to the Rust core (src-tauri/src/commands.rs). Field names mirror the core's
// serde camelCase output; timestamps are Unix milliseconds.
import { invoke } from '@tauri-apps/api/core'

export type Id = string
export type Ms = number

export type NoteKind = 'note' | 'whiteboard'
export interface Note {
  id: Id
  kind: NoteKind
  title: string
  body: string
  createdAt: Ms
  updatedAt: Ms
  pinned: boolean
  archivedAt: Ms | null
  vaultId: Id
  /** Folder within the vault; null = vault root. */
  folderId: Id | null
  /** The note's vault is encrypted. */
  encrypted: boolean
  tags: string[]
}
export interface NoteRef { id: Id; title: string }
export interface NewNote { kind?: NoteKind; title?: string; body?: string; tags?: string[]; pinned?: boolean; vaultId?: Id | null; folderId?: Id | null }
export interface Folder { id: Id; vaultId: Id; parentId: Id | null; name: string; createdAt: Ms }
export interface NotePatch { title?: string; body?: string }
export type ArchiveFilter = 'active' | 'archived' | 'all'
export interface ListFilter { archive?: ArchiveFilter; pinnedOnly?: boolean; vaultId?: Id | null; tag?: string | null; kind?: NoteKind | null }
export interface TagCount { name: string; count: number }
export interface SearchHit { id: Id; kind: NoteKind; title: string; snippet: string; archived: boolean; updatedAt: Ms; score: number }
export interface Backlink { note: NoteRef; context: string }
export interface OutgoingLink { title: string; notes: NoteRef[] }
export interface GraphNode { id: Id; kind: NoteKind; title: string; tags: string[]; degree: number; pinned: boolean }
export interface Graph { nodes: GraphNode[]; edges: { source: Id; target: Id }[] }
export interface NoteVersion { id: Id; noteId: Id; title: string; body: string; createdAt: Ms }
export interface Template { id: Id; name: string; title: string; body: string; builtin: boolean; createdAt: Ms; updatedAt: Ms }
export interface TemplateInput { name: string; title: string; body: string }

export type TaskStatus = 'todo' | 'doing' | 'done'
export type Frequency = 'daily' | 'weekly' | 'monthly' | 'yearly'
export interface Recurrence { freq: Frequency; interval: number; until?: Ms | null }
export interface Task {
  id: Id
  title: string
  notes: string
  status: TaskStatus
  priority: number
  dueAt: Ms | null
  allDay: boolean
  remindAt: Ms | null
  reminded: boolean
  recurrence: Recurrence | null
  plannerId: Id | null
  noteId: Id | null
  completedAt: Ms | null
  completionCount: number
  position: number
  createdAt: Ms
  updatedAt: Ms
}
export interface NewTask {
  title: string
  notes?: string
  status?: TaskStatus
  priority?: number
  dueAt?: Ms | null
  allDay?: boolean
  remindAt?: Ms | null
  recurrence?: Recurrence | null
  plannerId?: Id | null
  noteId?: Id | null
}
/** For nullable fields, `null` clears the value and omitting the key leaves it unchanged. */
export type TaskPatch = Partial<Omit<NewTask, 'title'>> & { title?: string; position?: number }
export interface TaskFilter {
  includeDone?: boolean
  plannerId?: Id | null
  noteId?: Id | null
  dueBefore?: Ms | null
  dueAfter?: Ms | null
  noDueDate?: boolean
}
export type ReminderKind = 'task' | 'event'
export interface Reminder { kind: ReminderKind; id: Id; title: string; remindAt: Ms; at: Ms | null }

export interface CalEvent {
  id: Id
  title: string
  description: string
  location: string
  startAt: Ms
  endAt: Ms
  allDay: boolean
  remindAt: Ms | null
  reminded: boolean
  color: string | null
  createdAt: Ms
  updatedAt: Ms
}
export interface EventInput {
  title: string
  description?: string
  location?: string
  startAt: Ms
  endAt: Ms
  allDay?: boolean
  remindAt?: Ms | null
  color?: string | null
}
export interface CalendarItem {
  kind: 'event' | 'task'
  id: Id
  title: string
  start: Ms
  end: Ms | null
  allDay: boolean
  done: boolean
  priority: number | null
  color: string | null
  projected: boolean
}

export interface Planner {
  id: Id
  name: string
  description: string
  color: string
  createdAt: Ms
  updatedAt: Ms
  taskCount: number
  doneCount: number
}
export interface PlannerInput { name: string; description?: string; color?: string | null }

export interface Vault {
  id: Id; name: string; createdAt: Ms; encrypted: boolean; unlocked: boolean; noteCount: number
  /** The paired device that shared this vault with us (null = ours). */
  owner: string | null
  /** Devices we share it with. */
  sharedWith: string[]
  /** The drive it lives on ("google-drive"), if it's a cloud vault. */
  cloud: string | null
}
export type MemoryKind = 'decision' | 'preference' | 'gotcha' | 'howto' | 'fact' | 'context' | 'summary' | 'codemap'
export interface Memory {
  id: Id; short: string; folder: Id; project: string; kind: MemoryKind; hook: string; content: string
  source: string; new: boolean; pinned: boolean; updatedAt: Ms; recalledAt: Ms | null; recallCount: number
}
export interface BrainGraph { nodes: Memory[]; links: { a: Id; b: Id; strength: number }[] }
export interface BrainProject { folder: Id; name: string; count: number; newCount: number }
export interface BrainStats { recalls: number; tokensServed: number; tokensSaved: number }
export interface TidyPair { a: Id; b: Id; aHook: string; bHook: string; score: number }
export interface TidyReport { duplicates: TidyPair[]; similar: TidyPair[]; stale: Id[] }
export interface SkillStatus { target: 'claude' | 'codex'; path: string; state: 'missing' | 'installed' | 'outdated' }
export interface BrainSetup { server: string; serverExists: boolean; args: string[]; mirror: string | null; vault: Id }
export interface ChatTurn { role: 'user' | 'assistant'; text: string }
export interface AssistantStatus { ready: boolean; downloading: boolean; loaded: boolean; quickPrompts: [string, string][] }
export interface GoogleStatus { configured: boolean; builtin: boolean; clientId: string | null; email: string | null }
export interface CloudInvite { email: string; acceptedBy: string | null }

export interface GrammarFix { kind: 'replace' | 'insert' | 'remove'; text: string }
export interface GrammarIssue { start: number; end: number; message: string; kind: string; fixes: GrammarFix[] }

export interface BrainStatus { loaded: boolean; model: string; neural: boolean; indexed: number; pending: number }
export interface RelatedNote { note: NoteRef; score: number; linked: boolean }
export interface DuplicatePair { a: NoteRef; b: NoteRef; score: number }
export interface Cluster { label: string; suggestedTag: string; notes: NoteRef[] }
export interface TagSuggestion { tag: string; score: number; source: 'keyword' | 'similar' }
export interface Summary { sentences: string[]; text: string }
export interface TidyResult { body: string; changes: string[] }
export interface Attachment { id: Id; vaultId: Id; name: string; mime: string; size: number; path: string; createdAt: Ms }

const call = <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args)

export const api = {
  // notes
  createNote: (input: NewNote) => call<Note>('create_note', { input }),
  quickCapture: (vaultId: Id | null, text: string) => call<Note>('quick_capture', { vaultId, text }),
  getNote: (id: Id) => call<Note>('get_note', { id }),
  updateNote: (id: Id, patch: NotePatch) => call<Note>('update_note', { id, patch }),
  listNotes: (filter: ListFilter = {}) => call<Note[]>('list_notes', { filter }),
  setPinned: (id: Id, pinned: boolean) => call<Note>('set_pinned', { id, pinned }),
  archiveNote: (id: Id) => call<Note>('archive_note', { id }),
  unarchiveNote: (id: Id) => call<Note>('unarchive_note', { id }),
  deleteNote: (id: Id) => call<void>('delete_note', { id }),
  setTags: (id: Id, tags: string[]) => call<Note>('set_tags', { id, tags }),
  addTag: (id: Id, tag: string) => call<Note>('add_tag', { id, tag }),
  removeTag: (id: Id, tag: string) => call<Note>('remove_tag', { id, tag }),
  listTags: (vaultId: Id | null) => call<TagCount[]>('list_tags', { vaultId }),
  mergeNotes: (noteIds: Id[], title?: string, archiveOriginals = true) =>
    call<Note>('merge_notes', { request: { noteIds, title, archiveOriginals } }),
  search: (vaultId: Id | null, query: string, includeArchived = false) =>
    call<SearchHit[]>('search', { vaultId, query, includeArchived }),
  createWhiteboard: (vaultId: Id | null, title = '') => call<Note>('create_whiteboard', { vaultId, title }),
  saveWhiteboard: (id: Id, scene: string) => call<void>('save_whiteboard', { id, scene }),

  // folders
  listFolders: (vaultId: Id | null) => call<Folder[]>('list_folders', { vaultId }),
  createFolder: (vaultId: Id | null, parentId: Id | null, name: string) => call<Folder>('create_folder', { vaultId, parentId, name }),
  renameFolder: (id: Id, name: string) => call<Folder>('rename_folder', { id, name }),
  moveFolder: (id: Id, parentId: Id | null) => call<Folder>('move_folder', { id, parentId }),
  deleteFolder: (id: Id) => call<void>('delete_folder', { id }),
  moveNoteToFolder: (noteId: Id, folderId: Id | null) => call<Note>('move_note_to_folder', { noteId, folderId }),

  // image attachments (bytes travel as base64)
  addAttachment: (vaultId: Id | null, name: string, mime: string, data: string) =>
    call<Attachment>('add_attachment', { vaultId, name, mime, data }),
  getAttachment: (id: Id) => call<{ mime: string; data: string }>('get_attachment', { id }),

  // links
  backlinks: (id: Id) => call<Backlink[]>('backlinks', { id }),
  outgoingLinks: (id: Id) => call<OutgoingLink[]>('outgoing_links', { id }),
  findByTitle: (vaultId: Id, title: string) => call<NoteRef[]>('find_by_title', { vaultId, title }),
  suggestTitles: (vaultId: Id | null, query: string) => call<NoteRef[]>('suggest_titles', { vaultId, query }),
  graph: (vaultId: Id | null, includeOrphans: boolean) => call<Graph>('graph', { vaultId, includeOrphans }),

  // history
  noteHistory: (id: Id) => call<NoteVersion[]>('note_history', { id }),
  snapshotNote: (id: Id) => call<void>('snapshot_note', { id }),
  restoreVersion: (versionId: Id) => call<Note>('restore_version', { versionId }),

  // templates
  listTemplates: () => call<Template[]>('list_templates'),
  createTemplate: (input: TemplateInput) => call<Template>('create_template', { input }),
  updateTemplate: (id: Id, input: TemplateInput) => call<Template>('update_template', { id, input }),
  deleteTemplate: (id: Id) => call<void>('delete_template', { id }),
  templateFromNote: (noteId: Id, name: string) => call<Template>('template_from_note', { noteId, name }),
  noteFromTemplate: (templateId: Id, title?: string, vaultId?: Id | null) =>
    call<Note>('note_from_template', { templateId, title: title ?? null, vaultId: vaultId ?? null }),

  // tasks
  createTask: (input: NewTask) => call<Task>('create_task', { input }),
  updateTask: (id: Id, patch: TaskPatch) => call<Task>('update_task', { id, patch }),
  completeTask: (id: Id) => call<Task>('complete_task', { id }),
  reopenTask: (id: Id) => call<Task>('reopen_task', { id }),
  deleteTask: (id: Id) => call<void>('delete_task', { id }),
  listTasks: (filter: TaskFilter = {}) => call<Task[]>('list_tasks', { filter }),
  snoozeReminder: (kind: ReminderKind, id: Id, until: Ms) => call<void>('snooze_reminder', { kind, id, until }),

  // calendar
  createEvent: (input: EventInput) => call<CalEvent>('create_event', { input }),
  updateEvent: (id: Id, input: EventInput) => call<CalEvent>('update_event', { id, input }),
  deleteEvent: (id: Id) => call<void>('delete_event', { id }),
  getEvent: (id: Id) => call<CalEvent>('get_event', { id }),
  calendar: (start: Ms, end: Ms) => call<CalendarItem[]>('calendar', { start, end }),

  // planners
  listPlanners: () => call<Planner[]>('list_planners'),
  createPlanner: (input: PlannerInput) => call<Planner>('create_planner', { input }),
  updatePlanner: (id: Id, input: PlannerInput) => call<Planner>('update_planner', { id, input }),
  deletePlanner: (id: Id) => call<void>('delete_planner', { id }),

  // vaults
  listVaults: () => call<Vault[]>('list_vaults'),
  defaultVault: () => call<Id>('default_vault'),
  /** With a password the vault is encrypted. */
  createVault: (name: string, password: string | null) => call<Vault>('create_vault', { name, password }),
  renameVault: (id: Id, name: string) => call<Vault>('rename_vault', { id, name }),
  unlockVault: (id: Id, password: string) => call<Vault>('unlock_vault', { id, password }),
  lockVault: (id: Id) => call<void>('lock_vault', { id }),
  lockAllVaults: () => call<void>('lock_all_vaults'),
  changeVaultPassword: (id: Id, old: string, newPassword: string) =>
    call<void>('change_vault_password', { id, old, new: newPassword }),
  deleteVault: (id: Id, password: string | null) => call<void>('delete_vault', { id, password }),
  shareVault: (vaultId: Id, peer: string) => call<Vault>('vault_share', { vaultId, peer }),
  unshareVault: (vaultId: Id, peer: string) => call<Vault>('vault_unshare', { vaultId, peer }),
  leaveVault: (vaultId: Id) => call<void>('vault_leave', { vaultId }),

  // Google Drive & cloud vaults
  googleStatus: () => call<GoogleStatus>('google_status'),
  googleSetClient: (clientId: string, clientSecret: string) => call<void>('google_set_client', { clientId, clientSecret }),
  googleConnect: () => call<string>('google_connect'),
  googleDisconnect: () => call<void>('google_disconnect'),
  createCloudVault: (name: string) => call<Vault>('cloud_create_vault', { name }),
  cloudInvites: (vaultId: Id) => call<CloudInvite[]>('cloud_invites', { vaultId }),
  cloudInvite: (vaultId: Id, email: string) => call<void>('cloud_invite', { vaultId, email }),
  cloudUninvite: (vaultId: Id, email: string) => call<void>('cloud_uninvite', { vaultId, email }),
  /** Opens the Google Picker to choose a shared vault folder; returns the vault's name. */
  cloudJoin: () => call<string>('cloud_join'),
  cloudSyncNow: () => call<void>('cloud_sync_now'),
  openUrl: (url: string) => call<void>('open_url', { url }),

  // Ask Astralis (the local writing model)
  assistantStatus: () => call<AssistantStatus>('assistant_status'),
  assistantDownload: () => call<void>('assistant_download'),
  assistantChat: (requestId: string, noteId: Id, history: ChatTurn[], message: string) =>
    call<string>('assistant_chat', { requestId, noteId, history, message }),
  assistantCancel: (requestId: string) => call<void>('assistant_cancel', { requestId }),
  assistantUnload: () => call<void>('assistant_unload'),

  // The Brain (memory for AI agents)
  brainProjects: () => call<BrainProject[]>('brain_projects'),
  brainMemories: (folder: Id | null) => call<Memory[]>('brain_memories', { folder }),
  brainSetReviewed: (id: Id, reviewed: boolean) => call<Memory>('brain_set_reviewed', { id, reviewed }),
  brainForget: (id: Id) => call<void>('brain_forget', { id }),
  brainTidy: (folder: Id | null) => call<TidyReport>('brain_tidy', { folder }),
  brainMerge: (keep: Id, other: Id) => call<Memory>('brain_merge', { keep, other }),
  brainCompact: (ids: Id[], hook: string) => call<Memory>('brain_compact', { ids, hook }),
  brainStats: () => call<BrainStats>('brain_stats'),
  brainGraph: () => call<BrainGraph>('brain_graph'),
  brainSetup: () => call<BrainSetup>('brain_setup'),
  brainSetMirror: (enabled: boolean, dir: string | null) => call<string | null>('brain_set_mirror', { enabled, dir }),
  brainSkillStatus: () => call<SkillStatus[]>('brain_skill_status'),
  brainInstallSkill: (target: 'claude' | 'codex') => call<string>('brain_install_skill', { target }),
  brainSkillText: () => call<string>('brain_skill_text'),
  moveNoteToVault: (noteId: Id, vaultId: Id) => call<Note>('move_note_to_vault', { noteId, vaultId }),

  // settings
  getSetting: (key: string) => call<string | null>('get_setting', { key }),
  setSetting: (key: string, value: string) => call<void>('set_setting', { key, value }),

  // grammar
  checkGrammar: (text: string) => call<GrammarIssue[]>('check_grammar', { text }),

  // brain
  brainStatus: () => call<BrainStatus>('brain_status'),
  relatedNotes: (id: Id) => call<RelatedNote[]>('related_notes', { id }),
  findDuplicates: (vaultId: Id | null, threshold?: number) =>
    call<DuplicatePair[]>('find_duplicates', { vaultId, threshold: threshold ?? null }),
  organizeNotes: (vaultId: Id | null) => call<Cluster[]>('organize_notes', { vaultId }),
  suggestTags: (id: Id) => call<TagSuggestion[]>('suggest_tags', { id }),
  summarizeNote: (id: Id, sentences = 3) => call<Summary>('summarize_note', { id, sentences }),
  /** Preview of a tidied note (not saved). */
  tidyNote: (text: string, fixSpelling: boolean) => call<TidyResult>('tidy_note', { text, fixSpelling }),
}

export const errorMessage = (e: unknown) => (typeof e === 'string' ? e : e instanceof Error ? e.message : String(e))

// ---- peer-to-peer & live collaboration ----
export interface PeerStatus { id: string; name: string; color: string; trusted: boolean; connected: boolean; nearby: boolean }
export interface P2pStatus { ready: boolean; id: string; name: string; color: string; peers: PeerStatus[] }
export type Live =
  | { kind: 'presence'; noteId: string | null; anchor: number | null; head: number | null; pointer: [number, number] | null }
  | { kind: 'update'; noteId: string; update: number[] }
  | { kind: 'awareness'; noteId: string; update: number[] }
  | { kind: 'scene'; noteId: string; elements: string }
export type P2pEvent =
  | { type: 'peers'; peers: PeerStatus[] }
  | { type: 'synced'; peer: string; notes: string[] }
  | { type: 'live'; peer: string; name: string; color: string; live: Live }
  | { type: 'paired'; peer: string; name: string }
  | { type: 'vaultsChanged' }

export const p2p = {
  status: () => call<P2pStatus>('p2p_status'),
  setName: (name: string) => call<void>('p2p_set_name', { name }),
  setColor: (color: string) => call<void>('p2p_set_color', { color }),
  startPairing: () => call<{ code: string; invite: string }>('p2p_start_pairing'),
  cancelPairing: () => call<void>('p2p_cancel_pairing'),
  join: (code: string) => call<PeerStatus>('p2p_join', { code }),
  unpair: (id: string) => call<void>('p2p_unpair', { id }),
  /** The note's text CRDT (a Yjs update) for the editor. */
  noteCrdt: (noteId: Id) => call<number[]>('note_crdt', { noteId }),
  /** Stores an editor edit (a Yjs update) and sends it to collaborators; returns the new text if it changed. */
  noteEdit: (noteId: Id, update: Uint8Array) => call<string | null>('note_edit', { noteId, update: Array.from(update) }),
  liveAwareness: (noteId: Id, update: Uint8Array) => call<void>('live_awareness', { noteId, update: Array.from(update) }),
  livePresence: (noteId: Id | null, anchor: number | null, head: number | null, pointer: [number, number] | null) =>
    call<void>('live_presence', { noteId, anchor, head, pointer }),
  liveScene: (noteId: Id, elements: string) => call<void>('live_scene', { noteId, elements }),
}
