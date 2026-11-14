import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  AlignLeft, ArrowUp, ArrowDownToLine, BookOpenCheck, Briefcase, Copy, Cpu, Download, FileText, ListChecks, type LucideIcon, MessageSquareText, Minimize2, Maximize2, PenLine,
  Replace, Smile, Sparkles, SpellCheck, Square, SquarePen, Type, Wand2,
} from 'lucide-react'
import { api, type AssistantAction, type AssistantOutput, type AssistantStatus, type ChatTurn, type Note } from '../api'
import { useApp } from '../context'
import { Markdown } from './Markdown'
import { useAssistantProfile } from '../lib/assistantProfile'

interface Message extends ChatTurn {
  /** Still being written. */
  streaming?: boolean
  /** Loading the model before the first word. */
  warming?: boolean
  /** What the reply is for (decides the buttons under it). */
  output?: AssistantOutput
  /** The selected text it worked on (if any), so the reply goes back to the right place. */
  target?: string
}

// Conversations per note, kept while the app is open.
const chats = new Map<string, Message[]>()

/** How each action looks (by id). */
const LOOK: Record<string, { icon: LucideIcon; tone: string }> = {
  improve: { icon: Wand2, tone: 'violet' },
  expand: { icon: Maximize2, tone: 'blue' },
  shorten: { icon: Minimize2, tone: 'amber' },
  grammar: { icon: SpellCheck, tone: 'green' },
  format: { icon: AlignLeft, tone: 'violet' },
  professional: { icon: Briefcase, tone: 'blue' },
  casual: { icon: Smile, tone: 'amber' },
  checklist: { icon: ListChecks, tone: 'green' },
  continue: { icon: PenLine, tone: 'violet' },
  summarize: { icon: FileText, tone: 'blue' },
  titles: { icon: Type, tone: 'amber' },
  feedback: { icon: BookOpenCheck, tone: 'red' },
}
const look = (id: string) => LOOK[id] ?? { icon: Sparkles, tone: 'violet' }

/** The AI writes Markdown, never HTML: text like "<your app>" is shown as written instead of being
 *  read as a tag (which can hide the rest of the answer). Code blocks are left alone. */
const asText = (md: string) => md.split(/(```[\s\S]*?(?:```|$))/).map((part, i) => (i % 2 ? part : part.replace(/<(?=[A-Za-z/!?])/g, '&lt;'))).join('')

const snippet = (s: string, n = 60) => {
  const t = s.replace(/\s+/g, ' ').trim()
  return t.length > n ? t.slice(0, n - 1) + '…' : t
}

function Orb({ size = 28, busy = false }: { size?: number; busy?: boolean }) {
  const { avatar } = useAssistantProfile()
  if (avatar) return <img className={'assistant-orb avatar' + (busy ? ' busy' : '')} style={{ width: size, height: size }} src={avatar} alt="" />
  return (
    <span className={'assistant-orb' + (busy ? ' busy' : '')} style={{ width: size, height: size }}>
      <Sparkles size={Math.round(size * 0.5)} />
    </span>
  )
}

type Apply = (text: string, mode: 'replace' | 'after', target?: string) => void

function Reply({ m, apply, onCopy }: { m: Message; apply: Apply; onCopy: () => void }) {
  if (!m.text) {
    return (
      <div className="assistant-thinking">
        <span className="assistant-dots"><i /><i /><i /></span> {m.warming ? 'Waking up the model…' : 'Writing…'}
      </div>
    )
  }
  const scope = m.target ? 'selection' : 'note'
  return (
    <>
      <div className={'assistant-card' + (m.streaming ? ' streaming' : '')}>
        <Markdown source={asText(m.text)} />
      </div>
      {!m.streaming && (
        <div className="assistant-actions">
          {m.output === 'rewrite' && (
            <button className="assistant-action primary" title={`Put this in place of the ${scope} (undo with Ctrl+Z)`} onClick={() => apply(m.text, 'replace', m.target)}>
              <Replace size={12} /> Replace {scope}
            </button>
          )}
          <button
            className={'assistant-action' + (m.output === 'insert' ? ' primary' : '')}
            title={m.target ? 'Add this below the selection' : 'Add this to the end of the note'}
            onClick={() => apply(m.text, 'after', m.target)}
          >
            <ArrowDownToLine size={12} /> {m.output === 'insert' ? 'Add to note' : 'Add below'}
          </button>
          <button className="assistant-action icon" title="Copy" onClick={onCopy}>
            <Copy size={12} />
          </button>
        </div>
      )}
    </>
  )
}

/**
 * "Ask Astralis": the assistant for the open note. One click rewrites, expands, shortens, fixes or
 * formats it (or just the text you've selected), or type what you want. Replies can be put into
 * the note with a click. Runs on this device.
 */
export function AssistantPanel({
  note,
  getSelection,
  onApply,
}: {
  note: Note
  getSelection: () => string
  onApply: Apply
}) {
  const { fail, toast } = useApp()
  const { name } = useAssistantProfile()
  const [status, setStatus] = useState<AssistantStatus | null>(null)
  const [messages, setMessages] = useState<Message[]>(() => chats.get(note.id) ?? [])
  const [input, setInput] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [selection, setSelection] = useState('')
  const [progress, setProgress] = useState<{ file: string; done: number; total: number | null } | null>(null)
  const scroller = useRef<HTMLDivElement>(null)
  const box = useRef<HTMLTextAreaElement>(null)

  const refresh = useCallback(() => api.assistantStatus().then(setStatus).catch(() => {}), [])
  useEffect(() => void refresh(), [refresh])
  // What's selected in the note (it keeps its selection while you click here).
  useEffect(() => {
    const t = window.setInterval(() => setSelection((s) => (getSelection() === s ? s : getSelection())), 400)
    return () => window.clearInterval(t)
  }, [getSelection])
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

  /** Runs an action (with whatever is typed as extra wishes) or a typed request. */
  const run = async (action: AssistantAction | null, typed: string) => {
    const message = typed.trim()
    if ((!action && !message) || busy) return
    const target = getSelection().trim()
    const history: ChatTurn[] = messages.filter((m) => !m.streaming).map(({ role, text }) => ({ role, text }))
    const id = crypto.randomUUID()
    const label = action ? (message ? `${action.label} — ${message}` : action.label) : message
    setInput('')
    setBusy(id)
    setMessages((ms) => [
      ...ms,
      { role: 'user', text: label, target: target || undefined },
      { role: 'assistant', text: '', streaming: true, warming: !status?.loaded, output: action?.output, target: target || undefined },
    ])
    try {
      const reply = await api.assistantChat(id, note.id, history, action?.id ?? null, message, target || null)
      setMessages((ms) => {
        const last = ms[ms.length - 1]
        return [...ms.slice(0, -1), { ...last, text: reply, streaming: false, warming: false }]
      })
    } catch (e) {
      setMessages((ms) => ms.slice(0, -2))
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
        <span>{name === 'Astralis' ? 'Ask Astralis' : name}</span>
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
          <h3>{name === 'Astralis' ? 'Your writing assistant' : `Meet ${name}`}</h3>
          <p className="muted">Works on the note you're on: rewrite it, expand it, fix it, make it look better.</p>
          <ul className="assistant-features">
            {status.actions.slice(0, 6).map((a) => {
              const { icon: Icon, tone } = look(a.id)
              return (
                <li key={a.id}>
                  <span className={'assistant-tile ' + tone}><Icon size={14} /></span>
                  <span><b>{a.label}</b><small>{a.hint}</small></span>
                </li>
              )
            })}
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
              <Download size={15} /> Download the model · 1.1 GB
            </button>
          )}
          <p className="small faint">One-time download of Alibaba's Qwen2.5 1.5B (Apache-2.0).</p>
        </div>
      </div>
    )
  }

  const groups = ['Rewrite', 'Create', 'Ask'].map((g) => [g, status.actions.filter((a) => a.group === g)] as const).filter(([, list]) => list.length)

  return (
    <div className="assistant">
      {head}
      <div className="assistant-log" ref={scroller}>
        {messages.length === 0 ? (
          <div className="assistant-welcome notes">
            <Orb size={44} />
            <h3>{selection ? 'Work on the selected text' : `What should we do with “${note.title || 'this note'}”?`}</h3>
            <p className="muted">{selection ? 'Pick what to do with it.' : 'Pick a job, or type what you want. Select part of the note to work on just that.'}</p>
            {groups.map(([group, list]) => (
              <div key={group} className="assistant-group">
                <div className="assistant-group-title">{group}</div>
                <div className="assistant-cards">
                  {list.map((a) => {
                    const { icon: Icon, tone } = look(a.id)
                    return (
                      <button key={a.id} className="assistant-quick-card" onClick={() => run(a, input)}>
                        <span className={'assistant-tile ' + tone}><Icon size={15} /></span>
                        <b>{a.label}</b>
                        <small>{a.hint}</small>
                      </button>
                    )
                  })}
                </div>
              </div>
            ))}
          </div>
        ) : (
          messages.map((m, i) =>
            m.role === 'user' ? (
              <div key={i} className="assistant-msg mine">
                <div className="assistant-bubble">
                  {m.text}
                  {m.target && <span className="assistant-bubble-sel">on “{snippet(m.target, 40)}”</span>}
                </div>
              </div>
            ) : (
              <div key={i} className="assistant-msg ai">
                <Orb size={24} busy={m.streaming} />
                <div className="assistant-body">
                  <Reply m={m} apply={onApply} onCopy={() => navigator.clipboard?.writeText(m.text).then(() => toast('Copied'))} />
                </div>
              </div>
            ),
          )
        )}
      </div>
      <div className="assistant-foot">
        {messages.length > 0 && !busy && (
          <div className="assistant-chips">
            {status.actions.map((a) => {
              const { icon: Icon, tone } = look(a.id)
              return (
                <button key={a.id} className={'assistant-chip ' + tone} onClick={() => run(a, input)}>
                  <Icon size={12} /> {a.label}
                </button>
              )
            })}
          </div>
        )}
        {selection.trim() && (
          <div className="assistant-selection" title={selection}>
            <MessageSquareText size={12} /> Working on the selection: “{snippet(selection)}”
          </div>
        )}
        <div className={'assistant-composer' + (busy ? ' busy' : '')}>
          <textarea
            ref={box}
            rows={1}
            placeholder={busy ? 'Writing…' : selection.trim() ? 'What should I do with the selection?' : 'Tell me what to change, or ask about the note…'}
            value={input}
            disabled={!!busy}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                run(null, input)
              }
            }}
          />
          {busy ? (
            <button className="assistant-send stop" title="Stop" onClick={() => api.assistantCancel(busy)}>
              <Square size={12} fill="currentColor" />
            </button>
          ) : (
            <button className="assistant-send" title="Send (Enter)" disabled={!input.trim()} onClick={() => run(null, input)}>
              <ArrowUp size={15} />
            </button>
          )}
        </div>
        <div className="assistant-hint">Enter to send · typing before a job adds your own wishes to it</div>
      </div>
    </div>
  )
}
