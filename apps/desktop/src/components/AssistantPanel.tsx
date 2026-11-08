import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { ArrowUp, Copy, Cpu, Download, FilePlus2, Flame, Lightbulb, ListChecks, type LucideIcon, PenLine, Replace, Sparkles, Square, SquarePen, TrendingUp } from 'lucide-react'
import { api, type AssistantStatus, type ChatTurn, type Note } from '../api'
import { useApp } from '../context'
import { Markdown } from './Markdown'

interface Message extends ChatTurn {
  /** Still being written. */
  streaming?: boolean
  /** Loading the model before the first word. */
  warming?: boolean
}

// Conversations per note, kept while the app is open.
const chats = new Map<string, Message[]>()

/** How each quick prompt looks (by its label). */
const QUICK_LOOK: Record<string, { icon: LucideIcon; hint: string; tone: string }> = {
  'Be brutally honest': { icon: Flame, hint: "What's wrong with it", tone: 'red' },
  'Will it pay off?': { icon: TrendingUp, hint: 'Cost against payoff', tone: 'green' },
  'Make a plan': { icon: ListChecks, hint: 'Phases and a first step', tone: 'blue' },
  'Write it up': { icon: PenLine, hint: 'Turn it into a clean note', tone: 'violet' },
  'Give me ideas': { icon: Lightbulb, hint: 'Ideas worth building', tone: 'amber' },
}
const look = (label: string) => QUICK_LOOK[label] ?? { icon: Sparkles, hint: '', tone: 'violet' }

/** A reply that opens with "**Verdict:** …" or "**ROI verdict:** …" gets that shown as a banner. */
function splitVerdict(text: string): { kind: string; verdict: string; tone: string; rest: string } | null {
  const m = /^\s*\*\*((?:ROI )?verdict):?\*\*:?\s*(.+?)\s*(?:\n|$)/i.exec(text)
  if (!m) return null
  const verdict = m[2].replace(/\*\*/g, '')
  const v = verdict.toLowerCase()
  const tone = /^(bad|not worth|no\b)/.test(v) ? 'bad' : /^(good|worth it|yes\b|strong)/.test(v) ? 'good' : 'meh'
  return { kind: /roi/i.test(m[1]) ? 'Return on investment' : 'Verdict', verdict, tone, rest: text.slice(m[0].length).trim() }
}

/** The AI writes Markdown, never HTML: text like "<your app>" is shown as written instead of being
 *  read as a tag (which can hide the rest of the answer). Code blocks are left alone. */
const asText = (md: string) => md.split(/(```[\s\S]*?(?:```|$))/).map((part, i) => (i % 2 ? part : part.replace(/<(?=[A-Za-z/!?])/g, '&lt;'))).join('')

function Orb({ size = 28, busy = false }: { size?: number; busy?: boolean }) {
  return (
    <span className={'assistant-orb' + (busy ? ' busy' : '')} style={{ width: size, height: size }}>
      <Sparkles size={Math.round(size * 0.5)} />
    </span>
  )
}

function Reply({ m, onReplace, onAppend, onCopy }: { m: Message; onReplace: () => void; onAppend: () => void; onCopy: () => void }) {
  if (m.warming && !m.text) {
    return (
      <div className="assistant-thinking">
        <span className="assistant-dots"><i /><i /><i /></span> Waking up the model…
      </div>
    )
  }
  if (!m.text) {
    return (
      <div className="assistant-thinking">
        <span className="assistant-dots"><i /><i /><i /></span> Thinking…
      </div>
    )
  }
  const v = splitVerdict(m.text)
  return (
    <>
      <div className={'assistant-card' + (m.streaming ? ' streaming' : '')}>
        {v && (
          <div className={'assistant-verdict ' + v.tone}>
            <span className="assistant-verdict-kind">{v.kind}</span>
            <span className="assistant-verdict-text">{v.verdict}</span>
          </div>
        )}
        <Markdown source={asText(v ? v.rest : m.text)} />
      </div>
      {!m.streaming && (
        <div className="assistant-actions">
          <button className="assistant-action" title="Replace the note with this (undo with Ctrl+Z)" onClick={onReplace}>
            <Replace size={12} /> Replace note
          </button>
          <button className="assistant-action" title="Add this to the end of the note" onClick={onAppend}>
            <FilePlus2 size={12} /> Add to note
          </button>
          <button className="assistant-action icon" title="Copy" onClick={onCopy}>
            <Copy size={12} />
          </button>
        </div>
      )}
    </>
  )
}

/** "Ask Astralis": talk to the local AI about the open note, or have it write the note. */
export function AssistantPanel({ note, onReplace, onAppend }: { note: Note; onReplace: (text: string) => void; onAppend: (text: string) => void }) {
  const { fail, toast } = useApp()
  const [status, setStatus] = useState<AssistantStatus | null>(null)
  const [messages, setMessages] = useState<Message[]>(() => chats.get(note.id) ?? [])
  const [input, setInput] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [progress, setProgress] = useState<{ file: string; done: number; total: number | null } | null>(null)
  const scroller = useRef<HTMLDivElement>(null)
  const box = useRef<HTMLTextAreaElement>(null)

  const refresh = useCallback(() => api.assistantStatus().then(setStatus).catch(() => {}), [])
  useEffect(() => void refresh(), [refresh])
  useEffect(() => {
    chats.set(note.id, messages)
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight, behavior: 'smooth' })
  }, [messages, note.id])
  // The box grows with what's typed (up to a few lines).
  useEffect(() => {
    const t = box.current
    if (!t) return
    t.style.height = 'auto'
    t.style.height = `${Math.min(t.scrollHeight, 160)}px`
  }, [input])

  useEffect(() => {
    const offTok = listen<{ id: string; text: string }>('assistant-token', ({ payload }) => {
      if (payload.id !== busy) return
      setMessages((ms) => {
        const last = ms[ms.length - 1]
        if (!last?.streaming) return ms
        return [...ms.slice(0, -1), { ...last, text: last.text + payload.text, warming: payload.text === '' && !last.text }]
      })
    })
    const offDl = listen<{ file: string; done: number; total: number | null }>('assistant-download', ({ payload }) => setProgress(payload))
    return () => {
      offTok.then((f) => f())
      offDl.then((f) => f())
    }
  }, [busy])

  const send = async (text: string) => {
    const message = text.trim()
    if (!message || busy) return
    const history: ChatTurn[] = messages.filter((m) => !m.streaming).map(({ role, text }) => ({ role, text }))
    const id = crypto.randomUUID()
    setInput('')
    setBusy(id)
    setMessages((ms) => [...ms, { role: 'user', text: message }, { role: 'assistant', text: '', streaming: true, warming: !status?.loaded }])
    try {
      const reply = await api.assistantChat(id, note.id, history, message)
      setMessages((ms) => [...ms.slice(0, -1), { role: 'assistant', text: reply }])
    } catch (e) {
      setMessages((ms) => ms.slice(0, -1))
      fail(e)
    } finally {
      setBusy(null)
      refresh()
      box.current?.focus()
    }
  }

  const download = async () => {
    setProgress({ file: '', done: 0, total: null })
    try {
      await api.assistantDownload()
      toast('The assistant is ready')
    } catch (e) {
      fail(e)
    } finally {
      setProgress(null)
      refresh()
    }
  }

  const head = (
    <div className="assistant-head">
      <Orb size={30} busy={!!busy} />
      <div className="assistant-title">
        <span>Ask Astralis</span>
        <span className="assistant-sub">
          <i className={'assistant-led' + (status?.ready ? (status.loaded ? ' on' : ' idle') : '')} />
          {!status?.ready ? 'Not set up' : busy ? 'Writing…' : status.loaded ? 'Ready · on this device' : 'Getting ready…'}
        </span>
      </div>
      <span className="grow" />
      {messages.length > 0 && !busy && (
        <button className="icon-btn" title="New chat" onClick={() => setMessages([])}>
          <SquarePen size={15} />
        </button>
      )}
    </div>
  )

  if (!status) return <div className="assistant">{head}</div>

  if (!status.ready) {
    const pct = progress?.total ? Math.round((progress.done / progress.total) * 100) : null
    return (
      <div className="assistant">
        {head}
        <div className="assistant-setup">
          <Orb size={56} />
          <h3>Your own AI, right in your notes</h3>
          <p className="muted">Talk about the note you're on, or tell it what the note should be.</p>
          <ul className="assistant-features">
            {Object.entries(QUICK_LOOK).map(([label, { icon: Icon, hint, tone }]) => (
              <li key={label}>
                <span className={'assistant-tile ' + tone}><Icon size={14} /></span>
                <span><b>{label}</b><small>{hint}</small></span>
              </li>
            ))}
          </ul>
          <div className="assistant-private">
            <Cpu size={14} /> Runs on this computer: nothing leaves your device and it uses no Claude tokens.
          </div>
          {progress ? (
            <div className="assistant-progress">
              <div className="assistant-bar">
                <span style={{ width: `${pct ?? 3}%` }} />
              </div>
              <span className="small muted">
                {progress.file ? `Downloading ${progress.file}` : 'Starting…'}
                {progress.total ? ` · ${(progress.done / 1e9).toFixed(2)} of ${(progress.total / 1e9).toFixed(2)} GB` : ''}
                {pct !== null ? ` · ${pct}%` : ''}
              </span>
            </div>
          ) : (
            <button className="btn primary assistant-download" onClick={download} disabled={status.downloading}>
              <Download size={15} /> Download the model · 2.4 GB
            </button>
          )}
          <p className="small faint">One-time download of Microsoft's Phi-3 mini.</p>
        </div>
      </div>
    )
  }

  return (
    <div className="assistant">
      {head}
      <div className="assistant-log" ref={scroller}>
        {messages.length === 0 ? (
          <div className="assistant-welcome">
            <Orb size={52} />
            <h3>What should we do with “{note.title || 'this note'}”?</h3>
            <p className="muted">I'll be honest, even when it's not what you want to hear.</p>
            <div className="assistant-cards">
              {status.quickPrompts.map(([label, prompt]) => {
                const { icon: Icon, hint, tone } = look(label)
                return (
                  <button key={label} className="assistant-quick-card" onClick={() => send(prompt)}>
                    <span className={'assistant-tile ' + tone}><Icon size={15} /></span>
                    <b>{label}</b>
                    <small>{hint}</small>
                  </button>
                )
              })}
            </div>
          </div>
        ) : (
          messages.map((m, i) =>
            m.role === 'user' ? (
              <div key={i} className="assistant-msg mine">
                <div className="assistant-bubble">{m.text}</div>
              </div>
            ) : (
              <div key={i} className="assistant-msg ai">
                <Orb size={24} busy={m.streaming} />
                <div className="assistant-body">
                  <Reply
                    m={m}
                    onReplace={() => onReplace(m.text)}
                    onAppend={() => onAppend(m.text)}
                    onCopy={() => navigator.clipboard?.writeText(m.text).then(() => toast('Copied'))}
                  />
                </div>
              </div>
            ),
          )
        )}
      </div>
      <div className="assistant-foot">
        {messages.length > 0 && !busy && (
          <div className="assistant-chips">
            {status.quickPrompts.map(([label, prompt]) => {
              const { icon: Icon, tone } = look(label)
              return (
                <button key={label} className={'assistant-chip ' + tone} onClick={() => send(prompt)}>
                  <Icon size={12} /> {label}
                </button>
              )
            })}
          </div>
        )}
        <div className={'assistant-composer' + (busy ? ' busy' : '')}>
          <textarea
            ref={box}
            rows={1}
            placeholder={busy ? 'Writing…' : 'Ask, or say what this note should be…'}
            value={input}
            disabled={!!busy}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                send(input)
              }
            }}
          />
          {busy ? (
            <button className="assistant-send stop" title="Stop" onClick={() => api.assistantCancel(busy)}>
              <Square size={12} fill="currentColor" />
            </button>
          ) : (
            <button className="assistant-send" title="Send (Enter)" disabled={!input.trim()} onClick={() => send(input)}>
              <ArrowUp size={15} />
            </button>
          )}
        </div>
        <div className="assistant-hint">Enter to send · Shift+Enter for a new line</div>
      </div>
    </div>
  )
}
