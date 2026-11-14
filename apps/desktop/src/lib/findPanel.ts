import { EditorView, type Panel, type ViewUpdate } from '@codemirror/view'
import { SearchQuery, closeSearchPanel, findNext, findPrevious, getSearchQuery, replaceAll, replaceNext, setSearchQuery } from '@codemirror/search'

const MAX_COUNT = 5000

/** "3 of 12" for the current search. */
function tally(view: EditorView, q: SearchQuery): { total: number; current: number; capped: boolean } {
  if (!q.search || !q.valid) return { total: 0, current: 0, capped: false }
  const sel = view.state.selection.main
  const cur = q.getCursor(view.state)
  let total = 0
  let current = 0
  for (let r = cur.next(); !r.done; r = cur.next()) {
    total++
    if (r.value.from === sel.from && r.value.to === sel.to) current = total
    if (total >= MAX_COUNT) return { total, current, capped: true }
  }
  return { total, current, capped: false }
}

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string) => {
  const e = document.createElement(tag)
  e.className = cls
  if (text) e.textContent = text
  return e
}

/** The find / replace bar: live match count, next/previous, case, whole word, regex, replace. */
export function findPanel(view: EditorView): Panel {
  const start = getSearchQuery(view.state)
  const dom = el('div', 'find-panel')

  const row = el('div', 'find-row')
  const caret = el('button', 'find-btn find-toggle', '›')
  caret.title = 'Replace'
  const find = el('input', 'find-input')
  find.placeholder = 'Find'
  find.setAttribute('main-field', 'true')
  find.spellcheck = false
  find.value = start.search
  const count = el('span', 'find-count')
  const opts = el('div', 'find-opts')
  const mk = (label: string, title: string) => {
    const b = el('button', 'find-btn find-opt', label)
    b.title = title
    b.type = 'button'
    return b
  }
  const bCase = mk('Aa', 'Match case')
  const bWord = mk('ab', 'Whole word')
  const bRe = mk('.*', 'Regular expression')
  opts.append(bCase, bWord, bRe)
  const prev = el('button', 'find-btn', '↑')
  prev.title = 'Previous (Shift+Enter)'
  const next = el('button', 'find-btn', '↓')
  next.title = 'Next (Enter)'
  const close = el('button', 'find-btn', '✕')
  close.title = 'Close (Esc)'
  row.append(caret, find, count, opts, prev, next, close)

  const row2 = el('div', 'find-row find-replace')
  const replace = el('input', 'find-input')
  replace.placeholder = 'Replace with'
  replace.spellcheck = false
  replace.value = start.replace
  const rOne = el('button', 'find-btn find-text', 'Replace')
  const rAll = el('button', 'find-btn find-text', 'All')
  row2.append(replace, rOne, rAll)
  dom.append(row, row2)

  let caseOn = start.caseSensitive
  let wordOn = start.wholeWord
  let reOn = start.regexp
  let open = !!start.replace
  const paint = () => {
    bCase.classList.toggle('on', caseOn)
    bWord.classList.toggle('on', wordOn)
    bRe.classList.toggle('on', reOn)
    dom.classList.toggle('with-replace', open)
    caret.classList.toggle('open', open)
  }
  const query = () => new SearchQuery({ search: find.value, replace: replace.value, caseSensitive: caseOn, wholeWord: wordOn, regexp: reOn })
  const readout = () => {
    const q = getSearchQuery(view.state)
    const t = tally(view, q)
    const bad = !!q.search && !q.valid
    find.classList.toggle('bad', bad || (!!q.search && t.total === 0))
    count.textContent = !q.search ? '' : bad ? 'Invalid' : t.total === 0 ? 'No results' : `${t.current || '–'} of ${t.total}${t.capped ? '+' : ''}`
  }
  const push = () => {
    const q = query()
    if (!q.eq(getSearchQuery(view.state))) view.dispatch({ effects: setSearchQuery.of(q) })
    readout()
  }
  const go = (back: boolean) => {
    push()
    ;(back ? findPrevious : findNext)(view)
    readout()
  }

  find.addEventListener('input', () => {
    push()
    // Jump to the first match from the cursor while typing (like browsers do).
    if (find.value) findNext(view)
    readout()
    find.focus()
  })
  replace.addEventListener('input', push)
  const toggle = (b: HTMLElement, set: () => void) => b.addEventListener('click', () => (set(), paint(), push(), find.focus()))
  toggle(bCase, () => (caseOn = !caseOn))
  toggle(bWord, () => (wordOn = !wordOn))
  toggle(bRe, () => (reOn = !reOn))
  caret.addEventListener('click', () => ((open = !open), paint(), (open ? replace : find).focus()))
  prev.addEventListener('click', () => go(true))
  next.addEventListener('click', () => go(false))
  close.addEventListener('click', () => (closeSearchPanel(view), view.focus()))
  rOne.addEventListener('click', () => (push(), replaceNext(view), readout()))
  rAll.addEventListener('click', () => (push(), replaceAll(view), readout()))
  find.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') (e.preventDefault(), go(e.shiftKey))
  })
  replace.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') (e.preventDefault(), push(), e.ctrlKey || e.metaKey ? replaceAll(view) : replaceNext(view), readout())
  })
  dom.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') (e.preventDefault(), closeSearchPanel(view), view.focus())
    // Ctrl+F again while open: back to the field with everything selected.
    else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') (e.preventDefault(), find.focus(), find.select())
    else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'h') (e.preventDefault(), (open = true), paint(), replace.focus())
    else if (e.key === 'F3') (e.preventDefault(), go(e.shiftKey))
  })

  paint()
  readout()
  return {
    dom,
    top: true,
    mount() {
      find.focus()
      find.select()
    },
    update(u: ViewUpdate) {
      for (const tr of u.transactions) {
        for (const e of tr.effects) {
          if (e.is(setSearchQuery)) {
            const q = e.value
            if (find.value !== q.search) find.value = q.search
            if (replace.value !== q.replace) replace.value = q.replace
            caseOn = q.caseSensitive
            wordOn = q.wholeWord
            reOn = q.regexp
            if (q.replace) open = true
            paint()
          }
        }
      }
      if (u.docChanged || u.selectionSet || u.transactions.some((t) => t.effects.length)) readout()
    },
  }
}
