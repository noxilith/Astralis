// Markdown formatting commands for the CodeMirror editor. Each one wraps or prefixes the
// selection, or — with nothing selected — inserts the syntax at the caret.
import { EditorSelection, type ChangeSpec } from '@codemirror/state'
import type { EditorView } from '@codemirror/view'
import { startCompletion } from '@codemirror/autocomplete'

export type MdCommand =
  | 'h1' | 'h2' | 'h3'
  | 'bold' | 'italic' | 'strike' | 'code'
  | 'link' | 'wikilink'
  | 'ul' | 'ol' | 'task' | 'quote'
  | 'codeblock' | 'table' | 'hr'

/** Wraps each selection in `before`/`after` (or unwraps if already wrapped). */
function wrap(view: EditorView, before: string, after: string, placeholder: string) {
  const { state } = view
  view.dispatch(
    state.changeByRange((range) => {
      const text = state.sliceDoc(range.from, range.to)
      const outerFrom = range.from - before.length
      const outerTo = range.to + after.length
      // Already wrapped just outside the selection → unwrap.
      if (outerFrom >= 0 && state.sliceDoc(outerFrom, range.from) === before && state.sliceDoc(range.to, outerTo) === after) {
        return {
          changes: [
            { from: outerFrom, to: range.from, insert: '' },
            { from: range.to, to: outerTo, insert: '' },
          ],
          range: EditorSelection.range(outerFrom, outerFrom + text.length),
        }
      }
      // Selection includes the markers → unwrap.
      if (text.length >= before.length + after.length && text.startsWith(before) && text.endsWith(after)) {
        const inner = text.slice(before.length, text.length - after.length)
        return { changes: { from: range.from, to: range.to, insert: inner }, range: EditorSelection.range(range.from, range.from + inner.length) }
      }
      const body = text || placeholder
      const start = range.from + before.length
      return {
        changes: { from: range.from, to: range.to, insert: before + body + after },
        range: EditorSelection.range(start, start + body.length),
      }
    }),
  )
}

const LIST_PREFIX = /^(\s*)(?:[-*+]\s+\[[ xX]\]\s+|[-*+]\s+|\d+[.)]\s+|>\s?|#{1,6}\s+)/

/** Sets (or toggles off) a line prefix on every line touched by the main selection. */
function prefixLines(view: EditorView, make: (i: number) => string, isSame: (line: string) => boolean) {
  const { state } = view
  const sel = state.selection.main
  const first = state.doc.lineAt(sel.from).number
  const last = state.doc.lineAt(sel.to).number
  const lines = []
  for (let n = first; n <= last; n++) lines.push(state.doc.line(n))
  const nonEmpty = lines.filter((l) => l.text.trim() !== '' || lines.length === 1)
  const allSame = nonEmpty.length > 0 && nonEmpty.every((l) => isSame(l.text))
  const changes: ChangeSpec[] = []
  let i = 0
  for (const line of lines) {
    if (line.text.trim() === '' && lines.length > 1) continue
    const m = LIST_PREFIX.exec(line.text)
    const indent = m ? m[1] : (/^\s*/.exec(line.text)?.[0] ?? '')
    const existing = m ? m[0].length : indent.length
    const insert = allSame ? indent : indent + make(i++)
    changes.push({ from: line.from, to: line.from + existing, insert })
  }
  view.dispatch({ changes, scrollIntoView: true })
}

/** Inserts a block on its own lines after the current line. */
function insertBlock(view: EditorView, block: string, caretOffset: number) {
  const { state } = view
  const line = state.doc.lineAt(state.selection.main.head)
  const atEmptyLine = line.text.trim() === ''
  const from = atEmptyLine ? line.from : line.to
  const insert = (atEmptyLine ? '' : '\n\n') + block
  view.dispatch({
    changes: { from, to: atEmptyLine ? line.to : line.to, insert },
    selection: { anchor: from + (atEmptyLine ? 0 : 2) + caretOffset },
    scrollIntoView: true,
  })
}

export function runMarkdown(view: EditorView, cmd: MdCommand) {
  const { state } = view
  const sel = state.selection.main
  const selected = state.sliceDoc(sel.from, sel.to)
  switch (cmd) {
    case 'h1':
    case 'h2':
    case 'h3': {
      const p = '#'.repeat(Number(cmd[1])) + ' '
      prefixLines(view, () => p, (l) => l.trimStart().startsWith(p) && !l.trimStart().startsWith(p.trim() + '#'))
      break
    }
    case 'bold':
      wrap(view, '**', '**', 'bold text')
      break
    case 'italic':
      wrap(view, '*', '*', 'italic text')
      break
    case 'strike':
      wrap(view, '~~', '~~', 'text')
      break
    case 'code':
      wrap(view, '`', '`', 'code')
      break
    case 'wikilink':
      wrap(view, '[[', ']]', '')
      if (!selected) startCompletion(view)
      break
    case 'link': {
      const text = selected || 'link text'
      const insert = `[${text}](url)`
      const urlStart = sel.from + text.length + 3
      view.dispatch({
        changes: { from: sel.from, to: sel.to, insert },
        selection: selected ? { anchor: urlStart, head: urlStart + 3 } : { anchor: sel.from + 1, head: sel.from + 1 + text.length },
      })
      break
    }
    case 'ul':
      prefixLines(view, () => '- ', (l) => /^\s*[-*+]\s+(?!\[[ xX]\])/.test(l))
      break
    case 'task':
      prefixLines(view, () => '- [ ] ', (l) => /^\s*[-*+]\s+\[[ xX]\]\s/.test(l))
      break
    case 'ol':
      prefixLines(view, (i) => `${i + 1}. `, (l) => /^\s*\d+[.)]\s/.test(l))
      break
    case 'quote':
      prefixLines(view, () => '> ', (l) => /^\s*>/.test(l))
      break
    case 'codeblock':
      if (selected) {
        const from = state.doc.lineAt(sel.from).from
        const to = state.doc.lineAt(sel.to).to
        view.dispatch({ changes: { from, to, insert: '```\n' + state.sliceDoc(from, to) + '\n```' } })
      } else {
        insertBlock(view, '```\n\n```', 3) // caret after the backticks: type the language
      }
      break
    case 'table':
      insertBlock(view, '| Column | Column |\n| --- | --- |\n| Cell | Cell |\n', 2)
      break
    case 'hr':
      insertBlock(view, '---\n', 4)
      break
  }
  view.focus()
}
