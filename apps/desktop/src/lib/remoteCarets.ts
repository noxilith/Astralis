// Other people's carets and selections in the editor, each with a colored name tag.
import { Annotation, StateEffect, StateField, Transaction, type Range } from '@codemirror/state'
import { Decoration, EditorView, WidgetType, type DecorationSet } from '@codemirror/view'

export interface RemoteCaret {
  id: string
  name: string
  color: string
  anchor: number
  head: number
}

/** Marks transactions that apply someone else's text (so they aren't echoed back). */
export const remoteChange = Annotation.define<boolean>()
export const setRemoteCarets = StateEffect.define<RemoteCaret[]>()

class CaretWidget extends WidgetType {
  readonly name: string
  readonly color: string
  constructor(name: string, color: string) {
    super()
    this.name = name
    this.color = color
  }
  eq(other: CaretWidget) {
    return other.name === this.name && other.color === this.color
  }
  toDOM() {
    const caret = document.createElement('span')
    caret.className = 'cm-remote-caret'
    caret.style.borderColor = this.color
    const tag = document.createElement('span')
    tag.className = 'cm-remote-tag'
    tag.textContent = this.name
    tag.style.background = this.color
    caret.append(tag)
    return caret
  }
  ignoreEvent() {
    return true
  }
}

function build(carets: RemoteCaret[], docLength: number): DecorationSet {
  const ranges: Range<Decoration>[] = []
  for (const c of carets) {
    const head = Math.min(Math.max(0, c.head), docLength)
    const anchor = Math.min(Math.max(0, c.anchor), docLength)
    if (anchor !== head) {
      const [from, to] = anchor < head ? [anchor, head] : [head, anchor]
      ranges.push(Decoration.mark({ attributes: { style: `background-color: ${c.color}33` } }).range(from, to))
    }
    ranges.push(Decoration.widget({ widget: new CaretWidget(c.name, c.color), side: 1 }).range(head))
  }
  return Decoration.set(ranges, true)
}

export const remoteCaretsField = StateField.define<{ carets: RemoteCaret[]; deco: DecorationSet }>({
  create: () => ({ carets: [], deco: Decoration.none }),
  update(value, tr) {
    let carets = value.carets
    let changed = false
    for (const e of tr.effects) {
      if (e.is(setRemoteCarets)) {
        carets = e.value
        changed = true
      }
    }
    if (tr.docChanged && !changed) {
      // Keep carets anchored to their text as the document changes.
      carets = carets.map((c) => ({ ...c, anchor: tr.changes.mapPos(c.anchor), head: tr.changes.mapPos(c.head) }))
      changed = true
    }
    return changed ? { carets, deco: build(carets, tr.state.doc.length) } : value
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
})

/** Replaces the editor text with `text` as a minimal splice (keeps the local caret in place). */
export function applyRemoteText(view: EditorView, text: string) {
  const old = view.state.doc.toString()
  if (old === text) return
  let start = 0
  while (start < old.length && start < text.length && old[start] === text[start]) start++
  let endOld = old.length
  let endNew = text.length
  while (endOld > start && endNew > start && old[endOld - 1] === text[endNew - 1]) {
    endOld--
    endNew--
  }
  view.dispatch({
    changes: { from: start, to: endOld, insert: text.slice(start, endNew) },
    annotations: [remoteChange.of(true), Transaction.addToHistory.of(false)],
  })
}
