import { useCallback, useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { Archive, Bot, Brain, Check, CheckCheck, Copy, Eye, Layers, Merge, PlugZap, Search, Sparkles, Trash2, User } from 'lucide-react'
import { api, type BrainProject, type BrainSetup, type BrainStats, type Memory, type MemoryKind, type TidyReport } from '../api'
import { useApp } from '../context'
import { Empty, Spinner, useConfirm } from '../components/ui'
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
  const [tab, setTab] = useState<'memories' | 'tidy' | 'connect'>('memories')
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
          <Stat value={fmt(stats?.recalls ?? 0)} label="lookups by your AI tools" />
          <Stat value={`${fmt(stats?.tokensServed ?? 0)}`} label="tokens of memory served" />
          <Stat value={`~${fmt(stats?.tokensSaved ?? 0)}`} label="tokens saved vs. loading everything" accent />
        </div>

        <div className="brain-tabs">
          {(['memories', 'tidy', 'connect'] as const).map((t) => (
            <button key={t} className={'brain-tab' + (tab === t ? ' active' : '')} onClick={() => setTab(t)}>
              {t === 'memories' ? <Brain size={13} /> : t === 'tidy' ? <Sparkles size={13} /> : <PlugZap size={13} />}
              {t === 'memories' ? `Memories${newOnes.length ? ` · ${newOnes.length} new` : ''}` : t === 'tidy' ? 'Tidy' : 'Connect'}
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

        {tab === 'tidy' && <TidyPanel folder={folder} memories={memories} onChange={load} />}
        {tab === 'connect' && <ConnectPanel />}
      </section>
      {confirm.element}
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

function CopyBlock({ label, text }: { label: string; text: string }) {
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

function ConnectPanel() {
  const { fail, toast } = useApp()
  const [setup, setSetup] = useState<BrainSetup | null>(null)
  useEffect(() => void api.brainSetup().then(setSetup).catch(fail), [fail])
  if (!setup) return <div className="brain-body"><Spinner /></div>
  const cmd = [setup.server, ...setup.args]
  const toml = `[mcp_servers.astralis-brain]\ncommand = ${JSON.stringify(setup.server)}\nargs = ${JSON.stringify(setup.args)}`
  const json = JSON.stringify({ mcpServers: { 'astralis-brain': { command: setup.server, args: setup.args } } }, null, 2)
  const hint = `Use the astralis-brain MCP tools as long-term memory: call brain_context at the start of a task, brain_recall / brain_map before re-reading files, and save what you learn with brain_remember / brain_map_note.`
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
      <CopyBlock label="Optional — add to your project's CLAUDE.md or AGENTS.md" text={hint} />

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
