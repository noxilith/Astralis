import { useEffect, useRef, useState } from 'react'
import { ChevronDown, ChevronUp, X } from 'lucide-react'

type Highlights = { set(name: string, h: unknown): void; delete(name: string): void }
const highlights = () => (CSS as unknown as { highlights?: Highlights }).highlights
const HighlightCtor = () => (window as unknown as { Highlight?: new (...r: Range[]) => unknown }).Highlight

/** All matches of `q` in the text under `root`, as DOM ranges. */
function findRanges(root: HTMLElement, q: string, matchCase: boolean): Range[] {
  if (!q) return []
  const needle = matchCase ? q : q.toLowerCase()
  const out: Range[] = []
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  for (let n = walker.nextNode(); n && out.length < 5000; n = walker.nextNode()) {
    const text = matchCase ? n.nodeValue ?? '' : (n.nodeValue ?? '').toLowerCase()
    for (let i = text.indexOf(needle); i !== -1 && out.length < 5000; i = text.indexOf(needle, i + needle.length)) {
      const r = document.createRange()
      r.setStart(n, i)
      r.setEnd(n, i + needle.length)
      out.push(r)
    }
  }
  return out
}

/** Find in a rendered (preview) note: the same bar as the editor's, highlighting every match. */
export function PreviewFind({ host, onClose }: { host: React.RefObject<HTMLElement | null>; onClose: () => void }) {
  const [q, setQ] = useState(() => window.getSelection()?.toString().trim().slice(0, 80) ?? '')
  const [matchCase, setMatchCase] = useState(false)
  const [at, setAt] = useState(0)
  const [ranges, setRanges] = useState<Range[]>([])
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [])

  // Re-search when the query or the rendered text changes.
  useEffect(() => {
    const run = () => {
      if (!host.current) return
      const found = findRanges(host.current, q, matchCase)
      setRanges(found)
      setAt((a) => (found.length ? Math.min(a, found.length - 1) : 0))
    }
    run()
    const obs = host.current ? new MutationObserver(run) : null
    if (host.current) obs?.observe(host.current, { childList: true, subtree: true, characterData: true })
    return () => obs?.disconnect()
  }, [q, matchCase, host])

  // Paint all matches, the current one stronger, and scroll to it.
  useEffect(() => {
    const hl = highlights()
    const H = HighlightCtor()
    if (hl && H) {
      hl.set('find-all', new H(...ranges))
      hl.set('find-current', new H(...(ranges[at] ? [ranges[at]] : [])))
    }
    const cur = ranges[at]
    if (cur) {
      const el = cur.startContainer.parentElement
      el?.scrollIntoView({ block: 'center', behavior: 'smooth' })
      if (!hl) {
        const sel = window.getSelection()
        sel?.removeAllRanges()
        sel?.addRange(cur)
      }
    }
  }, [ranges, at])
  useEffect(
    () => () => {
      const hl = highlights()
      hl?.delete('find-all')
      hl?.delete('find-current')
    },
    [],
  )

  const step = (d: number) => ranges.length && setAt((a) => (a + d + ranges.length) % ranges.length)
  return (
    <div className="preview-find" onKeyDown={(e) => e.key === 'Escape' && (e.preventDefault(), onClose())}>
      <div className="find-row">
        <input
          ref={input}
          className={'find-input' + (q && !ranges.length ? ' bad' : '')}
          placeholder="Find"
          spellCheck={false}
          value={q}
          onChange={(e) => (setQ(e.target.value), setAt(0))}
          onKeyDown={(e) => {
            if (e.key === 'Enter') (e.preventDefault(), step(e.shiftKey ? -1 : 1))
            else if (e.key === 'F3') (e.preventDefault(), step(e.shiftKey ? -1 : 1))
          }}
        />
        <span className="find-count">{!q ? '' : ranges.length ? `${at + 1} of ${ranges.length}` : 'No results'}</span>
        <button className={'find-btn find-opt' + (matchCase ? ' on' : '')} title="Match case" onClick={() => setMatchCase((m) => !m)}>
          Aa
        </button>
        <button className="find-btn" title="Previous (Shift+Enter)" onClick={() => step(-1)}>
          <ChevronUp size={14} />
        </button>
        <button className="find-btn" title="Next (Enter)" onClick={() => step(1)}>
          <ChevronDown size={14} />
        </button>
        <button className="find-btn" title="Close (Esc)" onClick={onClose}>
          <X size={14} />
        </button>
      </div>
    </div>
  )
}
