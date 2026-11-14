import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { Archive, Bot, Brain, Check, CheckCheck, Copy, Eye, Layers, Merge, Network, PlugZap, Scan, Search, Sparkles, Trash2, User } from 'lucide-react'
import { createNeural, KIND_HUE, type NeuralHandle } from './neural'
import { api, type BrainProject, type BrainSetup, type BrainStats, type HooksStatus, type Memory, type MemoryKind, type SkillStatus, type TidyReport } from '../api'
import { useApp } from '../context'
import { Empty, Spinner, useConfirm } from '../components/ui'
import { useCollab } from '../lib/collab'
import { relativeTime } from '../lib/time'

const KIND_ORDER: MemoryKind[] = ['context', 'decision', 'gotcha', 'howto', 'preference', 'fact', 'summary', 'codemap']
const KIND_LABEL: Record<MemoryKind, string> = {
  context: 'Current work',
  decision: 'Decisions',
  gotcha: 'Gotchas',
  howto: 'How-tos',
  preference: 'Preferences',
  fact: 'Facts',
  summary: 'Summaries',
  codemap: 'Code map',
}

const fmt = (n: number) => (n >= 10_000 ? `${(n / 1000).toFixed(0)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n))
const quote = (s: string) => (/^[\w./:@+-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`)

/** The Brain: what your AI tools remember, kept small and tidy, plus how to connect them. */
export function BrainView() {
  const { openNote, fail, toast, version } = useApp()
  const [projects, setProjects] = useState<BrainProject[] | null>(null)
  const [folder, setFolder] = useState<string | null>(null)
  const [memories, setMemories] = useState<Memory[]>([])
  const [stats, setStats] = useState<BrainStats | null>(null)
  const [tab, setTab] = useState<'memories' | 'graph' | 'tidy' | 'connect'>('memories')
  const [filter, setFilter] = useState('')
  const confirm = useConfirm()

  const load = useCallback(() => {
    api.brainProjects().then(setProjects).catch(fail)
    api.brainMemories(folder).then(setMemories).catch(fail)
    api.brainStats().then(setStats).catch(() => {})
  }, [folder, fail])
  useEffect(() => load(), [load, version])
  // Agents write while the app is open.
  useEffect(() => {
    const un = listen('brain-changed', () => load())
    return () => void un.then((f) => f())
  }, [load])

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase()
    return q ? memories.filter((m) => `${m.hook} ${m.content} ${m.project}`.toLowerCase().includes(q)) : memories
  }, [memories, filter])
  const groups = useMemo(() => {
    const g = new Map<MemoryKind, Memory[]>()
    for (const m of shown) g.set(m.kind, [...(g.get(m.kind) ?? []), m])
    return KIND_ORDER.filter((k) => g.has(k)).map((k) => [k, g.get(k)!] as const)
  }, [shown])
  const newOnes = memories.filter((m) => m.new)
  const current = projects?.find((p) => p.folder === folder)

  const act = (p: Promise<unknown>, msg?: string) =>
    p.then(() => (msg && toast(msg), load())).catch(fail)

  return (
    <div className="brain">
      <aside className="brain-side">
        <div className="brain-title">
          <Brain size={18} /> Brain
        </div>
        <p className="small faint" style={{ margin: '0 4px 10px' }}>
          Memory for your AI tools — so they remember instead of re-reading.
        </p>
        <button className={'brain-proj' + (folder === null ? ' active' : '')} onClick={() => setFolder(null)}>
          <Layers size={14} /> <span className="grow ellipsis">Everything</span>
          <span className="small faint">{projects?.reduce((n, p) => n + p.count, 0) ?? ''}</span>
        </button>
        {projects?.map((p) => (
          <button key={p.folder} className={'brain-proj' + (folder === p.folder ? ' active' : '')} onClick={() => setFolder(p.folder)}>
            {p.name === 'Me' ? <User size={14} /> : <Bot size={14} />}
            <span className="grow ellipsis">{p.name === 'Me' ? 'Me (everywhere)' : p.name}</span>
            {p.newCount > 0 && <span className="brain-new-dot" title={`${p.newCount} new`}>{p.newCount}</span>}
            <span className="small faint">{p.count}</span>
          </button>
        ))}
        {projects && projects.length === 0 && <p className="small faint" style={{ margin: 8 }}>Projects appear here once an AI tool uses the Brain in them.</p>}
      </aside>

      <section className="brain-main">
        <div className="brain-stats">
          <Stat
            value={fmt(stats?.recalls ?? 0)}
            label={`lookups by your AI tools${stats && stats.hits + stats.misses > 0 ? ` · ${Math.round((stats.hits / (stats.hits + stats.misses)) * 100)}% of searches found something` : ''}`}
          />
          <Stat value={`${fmt(stats?.tokensServed ?? 0)}`} label={`tokens of memory served${stats && stats.memories > 0 ? ` · ${stats.used} of ${stats.memories} memories ever used` : ''}`} />
          <Stat value={`~${fmt(stats?.tokensSaved ?? 0)}`} label="tokens saved vs. loading every memory (an upper bound)" accent />
        </div>

        <div className="brain-tabs">
          {(['memories', 'graph', 'tidy', 'connect'] as const).map((t) => (
            <button key={t} className={'brain-tab' + (tab === t ? ' active' : '')} onClick={() => setTab(t)}>
              {t === 'memories' ? <Brain size={13} /> : t === 'graph' ? <Network size={13} /> : t === 'tidy' ? <Sparkles size={13} /> : <PlugZap size={13} />}
              {t === 'memories' ? `Memories${newOnes.length ? ` · ${newOnes.length} new` : ''}` : t === 'graph' ? 'Graph' : t === 'tidy' ? 'Tidy' : 'Connect'}
            </button>
          ))}
        </div>

        {tab === 'memories' && (
          <div className="brain-body">
            <div className="row" style={{ marginBottom: 12 }}>
              <div className="space-search brain-search">
                <Search size={13} />
                <input placeholder={`Filter ${current ? current.name : 'all'} memories…`} value={filter} onChange={(e) => setFilter(e.target.value)} />
              </div>
              {newOnes.length > 0 && (
                <button
                  className="btn sm"
                  onClick={() => act(Promise.all(newOnes.map((m) => api.brainSetReviewed(m.id, true))), `Marked ${newOnes.length} reviewed`)}
                >
                  <CheckCheck size={13} /> Mark all reviewed
                </button>
              )}
            </div>
            {memories.length === 0 ? (
              <Empty icon={<Brain size={32} />} title="Nothing remembered yet">
                <span className="small">Connect an AI tool (Connect tab). As it works, it saves what it learns here.</span>
              </Empty>
            ) : (
              groups.map(([kind, list]) => (
                <div key={kind} className="brain-group">
                  <div className="brain-group-title">
                    {KIND_LABEL[kind]} <span className="faint">{list.length}</span>
                  </div>
                  {list.map((m) => (
                    <div key={m.id} className={'brain-mem' + (m.new ? ' is-new' : '')}>
                      <div className="grow" style={{ minWidth: 0 }}>
                        <div className="brain-hook">
                          {m.new && <span className="brain-new">new</span>}
                          <span className="ellipsis-2">{m.hook}</span>
                        </div>
                        <div className="brain-meta small faint">
                          <span className="kbd">{m.short}</span>
                          {folder === null && <span>{m.project}</span>}
                          <span>by {m.source}</span>
                          <span>{relativeTime(m.updatedAt)}</span>
                          {m.recallCount > 0 && <span>recalled {m.recallCount}×</span>}
                        </div>
                      </div>
                      <div className="brain-actions">
                        {m.new && (
                          <button className="btn ghost sm" title="Looks right — mark reviewed" onClick={() => act(api.brainSetReviewed(m.id, true))}>
                            <Check size={14} />
                          </button>
                        )}
                        <button className="btn ghost sm" title="Open to read or edit" onClick={() => openNote(m.id)}>
                          <Eye size={14} />
                        </button>
                        <button
                          className="btn ghost sm"
                          title="Forget (moves to the archive)"
                          onClick={async () => {
                            if (await confirm.ask(`Forget “${m.hook}”? Your AI tools won't see it any more (it's kept in the archive).`, 'Forget'))
                              act(api.brainForget(m.id), 'Forgotten')
                          }}
                        >
                          <Trash2 size={14} />
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              ))
            )}
          </div>
        )}

        {tab === 'graph' && <NeuralPanel memories={memories} />}
        {tab === 'tidy' && <TidyPanel folder={folder} memories={memories} onChange={load} />}
        {tab === 'connect' && <ConnectPanel />}
      </section>
      {confirm.element}
    </div>
  )
}

/** Every memory as a neuron in a brain-shaped network; related memories are wired together. */
function NeuralPanel({ memories }: { memories: Memory[] }) {
  const { openNote, fail } = useApp()
  const canvas = useRef<HTMLCanvasElement>(null)
  const net = useRef<NeuralHandle | null>(null)
  const [query, setQuery] = useState('')
  const [count, setCount] = useState<number | null>(null)
  const [surface, setSurface] = useState(true)
  const surfaceRef = useRef(surface)
  useEffect(() => {
    surfaceRef.current = surface
    net.current?.setSurface(surface)
  }, [surface])
  const queryRef = useRef(query)
  useEffect(() => {
    queryRef.current = query
  }, [query])
  // Rebuild when the memories change (new, edited, forgotten).
  // (Recalls don't rebuild it: they just light the memory up. The network is otherwise still.)
  const key = memories.map((m) => m.id + m.updatedAt).join()
  const netKey = useRef('')
  const seen = useRef<Map<string, string> | null>(null)
  const pending = useRef<string[]>([])
  const flush = (k: string) => {
    if (net.current && netKey.current === k && pending.current.length) {
      net.current.pulse(pending.current)
      pending.current = []
    }
  }
  // A memory an agent just recalled, saved or edited lights up.
  useEffect(() => {
    const now = new Map(memories.map((m) => [m.id, `${m.recallCount}:${m.updatedAt}`]))
    if (seen.current && net.current) {
      const hit = memories.filter((m) => seen.current!.get(m.id) !== now.get(m.id)).map((m) => m.id)
      pending.current.push(...hit)
      flush(key)
    }
    seen.current = now
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [memories])
  useEffect(() => {
    let off = false
    api
      .brainGraph()
      .then((g) => {
        if (off || !canvas.current) return
        net.current?.destroy()
        net.current = createNeural(canvas.current, g, openNote)
        net.current.setQuery(queryRef.current)
        net.current.setSurface(surfaceRef.current)
        netKey.current = key
        flush(key)
        setCount(g.nodes.length)
      })
      .catch(fail)
    return () => {
      off = true
    }
  }, [key, openNote, fail])
  useEffect(() => () => net.current?.destroy(), [])
  // Dev only: `__astralisNeural().benchmark()` in the console times a frame.
  useEffect(() => {
    if (import.meta.env.DEV) (window as unknown as { __astralisNeural?: () => NeuralHandle | null }).__astralisNeural = () => net.current
  }, [])
  useEffect(() => net.current?.setQuery(query), [query])
  const kinds = (['decision', 'gotcha', 'howto', 'preference', 'fact', 'context'] as MemoryKind[]).filter((k) => memories.some((m) => m.kind === k))
  return (
    <div className="neural-wrap">
      <canvas ref={canvas} className="graph-canvas" />
      <div className="space-hud top">
        <div className="space-search">
          <Search size={13} />
          <input placeholder="Find a memory…" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <label className="space-toggle" title="The brain's surface: tiny 3D points that turn with it">
          <input type="checkbox" checked={surface} onChange={(e) => setSurface(e.target.checked)} /> Surface
        </label>
        <button className="space-chip" onClick={() => net.current?.fit()} title="Back to the starting view">
          <Scan size={13} /> Reset view
        </button>
        <span className="space-count">{count ?? '…'} neurons · drag or scroll to turn, Ctrl + scroll to zoom</span>
      </div>
      <div className="space-hud bottom neural-legend">
        {kinds.map((k) => (
          <span key={k} className="space-chip">
            <span className="neural-dot" style={{ background: `hsl(${KIND_HUE[k][0]} ${KIND_HUE[k][1]}% ${KIND_HUE[k][2]}%)` }} /> {KIND_LABEL[k]}
          </span>
        ))}
      </div>
      {count === 0 && (
        <div className="space-empty">
          <b>No neurons yet</b>
          <span>Every memory your AI tools save becomes a neuron here, wired to the memories it relates to.</span>
        </div>
      )}
    </div>
  )
}

function Stat({ value, label, accent }: { value: string; label: string; accent?: boolean }) {
  return (
    <div className={'brain-stat' + (accent ? ' accent' : '')}>
      <b>{value}</b>
      <span>{label}</span>
    </div>
  )
}

function TidyPanel({ folder, memories, onChange }: { folder: string | null; memories: Memory[]; onChange: () => void }) {
  const { fail, toast } = useApp()
  const [report, setReport] = useState<TidyReport | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const byId = useMemo(() => new Map(memories.map((m) => [m.id, m])), [memories])
  const load = useCallback(() => {
    api.brainTidy(folder).then(setReport).catch(fail)
  }, [folder, fail])
  useEffect(() => load(), [load])
  const after = (msg: string) => () => (toast(msg), onChange(), load())

  if (!report) return <div className="brain-body"><Spinner /></div>
  const nothing = !report.duplicates.length && !report.similar.length && !report.stale.length
  return (
    <div className="brain-body">
      <p className="small muted" style={{ marginTop: 0 }}>
        A smaller Brain means fewer tokens for your AI tools. Merge repeats, check memories that may disagree, and compress old ones.
      </p>
      {nothing && <Empty icon={<Sparkles size={30} />} title="All tidy">Nothing to merge, check or compress right now.</Empty>}
      {report.duplicates.length > 0 && (
        <div className="brain-group">
          <div className="brain-group-title">Saying the same thing <span className="faint">{report.duplicates.length}</span></div>
          {report.duplicates.map((d) => (
            <div key={d.a + d.b} className="brain-pair">
              <div className="grow">
                <div className="ellipsis">{d.aHook}</div>
                <div className="ellipsis faint">{d.bHook}</div>
              </div>
              <span className="small faint">{Math.round(d.score * 100)}% alike</span>
              <button className="btn sm" onClick={() => api.brainMerge(d.a, d.b).then(after('Merged')).catch(fail)}>
                <Merge size={13} /> Merge
              </button>
            </div>
          ))}
        </div>
      )}
      {report.similar.length > 0 && (
        <div className="brain-group">
          <div className="brain-group-title">Same subject — check they still agree <span className="faint">{report.similar.length}</span></div>
          {report.similar.map((d) => (
            <div key={d.a + d.b} className="brain-pair">
              <div className="grow">
                <div className="ellipsis">{d.aHook}</div>
                <div className="ellipsis faint">{d.bHook}</div>
              </div>
              <button className="btn sm" onClick={() => api.brainMerge(d.a, d.b).then(after('Merged')).catch(fail)}>
                <Merge size={13} /> Merge
              </button>
            </div>
          ))}
        </div>
      )}
      {report.stale.length > 0 && (
        <div className="brain-group">
          <div className="brain-group-title">
            Not used in a while <span className="faint">{report.stale.length}</span>
            <span className="grow" />
            <button
              className="btn sm"
              disabled={!picked.size}
              onClick={() =>
                api
                  .brainCompact([...picked], '')
                  .then(() => setPicked(new Set()))
                  .then(after(`Compressed ${picked.size} into one summary`))
                  .catch(fail)
              }
            >
              <Archive size={13} /> Compress {picked.size || ''} into a summary
            </button>
          </div>
          {report.stale.map((id) => {
            const m = byId.get(id)
            if (!m) return null
            return (
              <label key={id} className="brain-pair" style={{ cursor: 'pointer' }}>
                <input
                  type="checkbox"
                  checked={picked.has(id)}
                  onChange={(e) => {
                    const next = new Set(picked)
                    if (e.target.checked) next.add(id)
                    else next.delete(id)
                    setPicked(next)
                  }}
                />
                <span className="grow ellipsis">{m.hook}</span>
                <span className="small faint">{m.recalledAt ? `last used ${relativeTime(m.recalledAt)}` : 'never recalled'}</span>
              </label>
            )
          })}
        </div>
      )}
    </div>
  )
}

export function CopyBlock({ label, text }: { label: string; text: string }) {
  const { toast } = useApp()
  return (
    <div className="brain-copy">
      <div className="brain-copy-head">
        <span className="small">{label}</span>
        <button className="btn ghost sm" onClick={() => navigator.clipboard?.writeText(text).then(() => toast('Copied'))}>
          <Copy size={12} /> Copy
        </button>
      </div>
      <pre>{text}</pre>
    </div>
  )
}

const SKILL_TOOL = { claude: 'Claude Code', codex: 'Codex' } as const

/** The astralis-brain skill: a playbook that teaches agents to use the Brain well. */
function SkillSection() {
  const { fail, toast } = useApp()
  const [status, setStatus] = useState<SkillStatus[] | null>(null)
  const load = useCallback(() => void api.brainSkillStatus().then(setStatus).catch(fail), [fail])
  useEffect(() => load(), [load])
  return (
    <div className="brain-copy brain-skill">
      <div className="brain-copy-head">
        <span className="small">
          <Sparkles size={12} /> Skill — teaches your AI how to use the Brain well
        </span>
        <button className="btn ghost sm" onClick={() => api.brainSkillText().then((t) => navigator.clipboard?.writeText(t)).then(() => toast('Skill copied'))}>
          <Copy size={12} /> Copy
        </button>
      </div>
      <div className="brain-skill-body">
        <p className="small muted" style={{ margin: '0 0 10px' }}>
          When to check the Brain, how to search it, what's worth saving and how to phrase it, keeping it true, and leaving a note when work is
          unfinished. Your AI loads it whenever a task calls for it.
        </p>
        {status?.map((s) => (
          <div key={s.target} className="row" style={{ marginBottom: 6 }}>
            <span className="grow small">
              <b>{SKILL_TOOL[s.target]}</b>{' '}
              <span className="faint">
                {s.state === 'installed' ? '· installed' : s.state === 'outdated' ? '· an older version is installed' : '· not installed'}
              </span>
            </span>
            <button
              className={'btn sm' + (s.state === 'installed' ? '' : ' primary')}
              disabled={s.state === 'installed'}
              onClick={() =>
                api
                  .brainInstallSkill(s.target)
                  .then((p) => (toast(`Installed for ${SKILL_TOOL[s.target]}: ${p}`), load()))
                  .catch(fail)
              }
            >
              {s.state === 'installed' ? <><Check size={13} /> Installed</> : s.state === 'outdated' ? 'Update' : 'Install'}
            </button>
          </div>
        ))}
      </div>
    </div>
  )
}

/**
 * Hooks make the Brain automatic: the index arrives when a session starts, and memories that clearly
 * match a message are added to it, so the AI uses what it saved instead of digging through chat logs.
 * (Claude Code runs them; they cost nothing when nothing matches.)
 */
function HooksSection({ server, args }: { server: string; args: string[] }) {
  const { fail, toast } = useApp()
  const [status, setStatus] = useState<HooksStatus | null>(null)
  const [stop, setStop] = useState(false)
  const [busy, setBusy] = useState(false)
  useEffect(() => void api.brainHooksStatus().then((s) => (setStatus(s), setStop(s.stop))).catch(() => {}), [])
  const quoteCmd = (s: string) => (/^[\w./:@+\\-]+$/.test(s) ? s : `"${s.replace(/"/g, '\\"')}"`)
  const cmd = (event: string) => [server, ...args, 'hook', event].map(quoteCmd).join(' ')
  const hook = (event: string) => [{ hooks: [{ type: 'command', command: cmd(event), timeout: 10 }] }]
  const main = JSON.stringify({ hooks: { SessionStart: hook('session-start'), UserPromptSubmit: hook('prompt') } }, null, 2)
  const stopOnly = JSON.stringify({ hooks: { Stop: hook('stop') } }, null, 2)
  const run = async (fn: () => Promise<HooksStatus>, done: string) => {
    setBusy(true)
    try {
      const s = await fn()
      setStatus(s)
      setStop(s.stop)
      toast(done)
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="brain-group" style={{ marginTop: 18 }}>
      <div className="brain-group-title">Make it automatic (Claude Code hooks)</div>
      <p className="small muted" style={{ marginTop: 0 }}>
        Without hooks the AI has to remember to ask the Brain. With them, the Brain's index is in its context when a session starts, and any memory that clearly matches what you
        just said is added to your message (once per session, so it isn't paid for twice). It then has no reason to read old chat logs. Nothing is added when nothing matches.
      </p>
      <p className="small" style={{ margin: '0 0 8px' }}>
        {status?.installed ? (
          <>
            <Check size={13} /> Installed in <span className="kbd">{status.path}</span>
          </>
        ) : (
          <>Not installed yet. One click adds the hooks to <span className="kbd">{status?.path ?? '~/.claude/settings.json'}</span> (your other settings stay as they are, and the original is kept as a backup).</>
        )}
      </p>
      <label className="checkbox" style={{ marginBottom: 8 }}>
        <input type="checkbox" checked={stop} disabled={busy} onChange={(e) => setStop(e.target.checked)} />
        Also remind the AI, when it's about to finish, to save what it learned and update the hand-off note (at most every 20 minutes; costs it one extra turn)
      </label>
      <div className="row" style={{ gap: 8 }}>
        <button className="btn primary" disabled={busy} onClick={() => run(() => api.brainInstallHooks(stop), status?.installed ? 'Hooks updated' : 'Hooks installed — new Claude Code sessions use them')}>
          {status?.installed ? 'Update hooks' : 'Install hooks'}
        </button>
        {status?.installed && (
          <button className="btn" disabled={busy} onClick={() => run(() => api.brainRemoveHooks(), 'Hooks removed')}>
            Remove hooks
          </button>
        )}
      </div>
      <details style={{ marginTop: 10 }}>
        <summary className="small muted" style={{ cursor: 'pointer' }}>Or add them by hand</summary>
        <CopyBlock label="Add to ~/.claude/settings.json (merge with an existing hooks section)" text={main} />
        <CopyBlock label="Optional save reminder" text={stopOnly} />
      </details>
    </div>
  )
}

/** What AI agents may do with the user's own notes (the notes_* MCP tools). */
function NotesAccess({ what = 'notes' }: { what?: 'notes' | 'plan' }) {
  const { fail } = useApp()
  const [access, setAccess] = useState<'edit' | 'read' | 'off' | null>(null)
  const plan = what === 'plan'
  useEffect(() => void (plan ? api.agentsPlanAccess() : api.agentsNotesAccess()).then(setAccess).catch(fail), [fail, plan])
  const choose = (a: 'edit' | 'read' | 'off') => {
    setAccess(a)
    ;(plan ? api.agentsSetPlanAccess(a) : api.agentsSetNotesAccess(a)).catch(fail)
  }
  const options: ['edit' | 'read' | 'off', string, string][] = [
    plan
      ? ['edit', 'Read and edit', 'Plan projects, add and tick off tasks, and put appointments and work sessions on your calendar.']
      : ['edit', 'Read and edit', 'Find, read, write, edit, move and delete notes, and organize folders. Every edit keeps the note\u2019s history, and deleted notes go to the archive first.'],
    plan ? ['read', 'Read only', 'See your tasks and calendar, but not change them.'] : ['read', 'Read only', 'Find and read notes, but not change them.'],
    plan ? ['off', 'No access', 'Tasks and calendar stay private.'] : ['off', 'No access', 'Only the Brain\u2019s memories.'],
  ]
  return (
    <div className="brain-group" style={{ marginTop: 18 }}>
      <div className="brain-group-title">{plan ? 'Tasks and calendar' : 'Your notes'}</div>
      <p className="small muted" style={{ marginTop: 0 }}>
        {plan
          ? 'AI agents can make planners (say, for building an app), manage your tasks and add appointments to your calendar when you ask them to. Changes show up here right away.'
          : 'Through the same server, AI agents can work with the notes in your vaults when you ask them to — never encrypted vaults. Changes show up here right away.'}
      </p>
      {access &&
        options.map(([value, label, hint]) => (
          <label key={value} className="radio-row">
            <input type="radio" name={plan ? "agents-plan" : "agents-notes"} checked={access === value} onChange={() => choose(value)} />
            <span>
              <b className="small">{label}</b>
              <span className="small faint"> — {hint}</span>
            </span>
          </label>
        ))}
    </div>
  )
}

/**
 * Cloud Brain: the Brain's memories sync through Google Drive. Other devices hook up by pairing
 * with this one and sharing the Brain vault; there, "Use as this device's Brain" switches over.
 */
function CloudBrain({ brainVault, reload }: { brainVault: string; reload: () => void }) {
  const { fail, toast, vaults, refreshVaults } = useApp()
  const { peers } = useCollab()
  const [busy, setBusy] = useState(false)
  const brain = vaults.find((v) => v.id === brainVault)
  const mine = !!brain && !brain.owner
  const others = vaults.filter((v) => v.id !== brainVault && v.cloud && v.owner && !v.encrypted)
  const run = async (fn: () => Promise<unknown>, done: string) => {
    setBusy(true)
    try {
      await fn()
      await refreshVaults()
      reload()
      toast(done)
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
    }
  }
  const paired = peers.filter((p) => p.trusted)
  return (
    <div className="brain-group" style={{ marginTop: 18 }}>
      <div className="brain-group-title">Cloud Brain</div>
      <p className="small muted" style={{ marginTop: 0 }}>
        Keep the Brain's memories on your Google Drive (end-to-end encrypted) so every device you use shares one memory. Needs Google Drive set up in
        Settings.
      </p>
      {brain && mine && !brain.cloud && (
        <button className="btn primary" disabled={busy} onClick={() => run(() => api.brainCloudOn(), 'The Brain is on Google Drive')}>
          Turn the Brain into a cloud vault
        </button>
      )}
      {brain && brain.cloud && mine && (
        <>
          <p className="small" style={{ margin: '0 0 8px' }}>
            <Check size={13} /> The Brain syncs through Google Drive. To hook up another device, pair it in Settings, then share the Brain with it here, and
            choose “Use as this device's Brain” on that device.
          </p>
          {paired.length === 0 && <p className="small faint">No paired devices yet.</p>}
          {paired.map((p) => {
            const on = brain.sharedWith.includes(p.id)
            return (
              <label key={p.id} className="checkbox">
                <input
                  type="checkbox"
                  checked={on}
                  disabled={busy}
                  onChange={(e) =>
                    run(
                      () => (e.target.checked ? api.shareVault(brain.id, p.id) : api.unshareVault(brain.id, p.id)),
                      e.target.checked ? `Sharing the Brain with ${p.name}` : `Stopped sharing with ${p.name}`,
                    )
                  }
                />
                <span>{p.name}</span>
              </label>
            )
          })}
          <button className="btn" style={{ marginTop: 8 }} disabled={busy} onClick={() => run(() => api.brainCloudOff(), 'The Brain stays on this device only')}>
            Stop syncing the Brain
          </button>
        </>
      )}
      {brain && !mine && (
        <p className="small" style={{ margin: 0 }}>
          <Check size={13} /> This device uses the cloud Brain from {brain.owner}.
        </p>
      )}
      {others.length > 0 && (
        <div style={{ marginTop: 10 }}>
          <div className="small muted">Shared from another device:</div>
          {others.map((v) => (
            <div key={v.id} className="row" style={{ gap: 8, marginTop: 4 }}>
              <span className="small grow">{v.name}</span>
              <button className="btn" disabled={busy} onClick={() => run(() => api.brainUseVault(v.id), `This device's Brain is now “${v.name}”`)}>
                Use as this device's Brain
              </button>
            </div>
          ))}
          <p className="small faint">Memories already in this device's own Brain vault stay there; move them across with right-click → Move to vault.</p>
        </div>
      )}
    </div>
  )
}

function ConnectPanel() {
  const { fail, toast } = useApp()
  const [setup, setSetup] = useState<BrainSetup | null>(null)
  useEffect(() => void api.brainSetup().then(setSetup).catch(fail), [fail])
  if (!setup) return <div className="brain-body"><Spinner /></div>
  const cmd = [setup.server, ...setup.args]
  const toml = `[mcp_servers.astralis-brain]\ncommand = ${JSON.stringify(setup.server)}\nargs = ${JSON.stringify(setup.args)}`
  const json = JSON.stringify({ mcpServers: { 'astralis-brain': { command: setup.server, args: setup.args } } }, null, 2)
  const hint = `Use the astralis-brain MCP tools as long-term memory. Never search old chats, transcripts or logs for what was decided or learned before: ask the Brain. Call brain_context (with the task in a sentence) at the start, brain_recall / brain_map before re-reading files, code_outline / code_find instead of reading whole files, save what you learn with brain_remember / brain_map_note as you go, and call brain_handoff (where things stand, what's next) before you stop.`
  return (
    <div className="brain-body">
      {!setup.serverExists && (
        <p className="brain-warn small">
          The Brain server isn't built yet. Run <span className="kbd">./run.sh app</span> again (it builds it), or{' '}
          <span className="kbd">cargo build -p astralis-mcp</span>.
        </p>
      )}
      <p className="small muted" style={{ marginTop: 0 }}>
        Your AI tools talk to the Brain over MCP, running on this computer. They get a short index, search by meaning, and a code map — small
        answers instead of re-reading files and old chats.
      </p>
      <CopyBlock label="Claude Code — run once in a terminal" text={`claude mcp add --scope user astralis-brain -- ${cmd.map(quote).join(' ')}`} />
      <CopyBlock label="Codex — add to ~/.codex/config.toml" text={toml} />
      <CopyBlock label="Cursor (~/.cursor/mcp.json) · Claude Desktop · other MCP apps" text={json} />
      <SkillSection />
      <HooksSection server={setup.server} args={setup.args} />
      <CopyBlock label="Without the skill — add this to your project's CLAUDE.md or AGENTS.md instead" text={hint} />
      <NotesAccess />
      <NotesAccess what="plan" />
      <CloudBrain brainVault={setup.vault} reload={() => void api.brainSetup().then(setSetup).catch(fail)} />

      <div className="brain-group" style={{ marginTop: 18 }}>
        <div className="brain-group-title">Markdown mirror</div>
        <p className="small muted" style={{ marginTop: 0 }}>
          A copy of the Brain as Markdown files — open it in Obsidian, or let any AI tool read it. Updated as memories change.
        </p>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={!!setup.mirror}
            onChange={(e) =>
              api
                .brainSetMirror(e.target.checked, null)
                .then((mirror) => (setSetup({ ...setup, mirror }), toast(mirror ? `Mirroring to ${mirror}` : 'Mirror off')))
                .catch(fail)
            }
          />
          {setup.mirror ? <span>Mirroring to <span className="kbd">{setup.mirror}</span></span> : 'Mirror the Brain to a Markdown folder'}
        </label>
      </div>
    </div>
  )
}
