import { useEffect, useRef } from 'react'
import { EditorState, Compartment, StateEffect, type Extension } from '@codemirror/state'
import {
  Decoration,
  EditorView,
  MatchDecorator,
  ViewPlugin,
  drawSelection,
  dropCursor,
  highlightSpecialChars,
  keymap,
  placeholder as cmPlaceholder,
  type DecorationSet,
  type ViewUpdate,
} from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { searchKeymap, highlightSelectionMatches, search } from '@codemirror/search'
import { findPanel } from '../lib/findPanel'
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, type CompletionContext } from '@codemirror/autocomplete'
import { linter, lintKeymap, forceLinting, forEachDiagnostic, type Diagnostic } from '@codemirror/lint'
import { addIgnored, ignoredSpans, loadIgnored, onIgnoredChange } from '../lib/ignored'
import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
import { languages } from '@codemirror/language-data'
import { HighlightStyle, syntaxHighlighting, syntaxTree, indentOnInput } from '@codemirror/language'
import { RangeSetBuilder } from '@codemirror/state'
import { codeHighlighter } from '../lib/highlight'
import { tags as t } from '@lezer/highlight'
import { api } from '../api'
import { runMarkdown } from '../lib/markdown'
import { yCollab, ySyncAnnotation, yUndoManagerKeymap } from 'y-codemirror.next'
import type { NoteSession } from '../lib/noteSession'
import { imageFiles, insertImages } from '../lib/attachments'

const highlight = HighlightStyle.define([
  { tag: t.heading1, fontSize: '1.6em', fontWeight: '700' },
  { tag: t.heading2, fontSize: '1.35em', fontWeight: '700' },
  { tag: t.heading3, fontSize: '1.15em', fontWeight: '650' },
  { tag: [t.heading4, t.heading5, t.heading6], fontWeight: '650' },
  { tag: t.strong, fontWeight: '700' },
  { tag: t.emphasis, fontStyle: 'italic' },
  { tag: t.strikethrough, textDecoration: 'line-through' },
  { tag: t.link, color: 'var(--accent-strong)' },
  { tag: t.url, color: 'var(--text-faint)' },
  { tag: t.monospace, fontFamily: 'var(--mono)', fontSize: '.9em', color: 'var(--accent-strong)' },
  { tag: t.quote, color: 'var(--text-muted)', fontStyle: 'italic' },
  { tag: [t.processingInstruction, t.meta, t.contentSeparator], color: 'var(--text-faint)' },
  { tag: t.list, color: 'var(--accent)' },
])

const WIKILINK_RE = /\[\[([^\]\n]+?)\]\]/g

/** Shades fenced code blocks so they read as a panel while editing. */
function codeBlockDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>()
  let lastLine = 0
  for (const { from, to } of view.visibleRanges) {
    syntaxTree(view.state).iterate({
      from,
      to,
      enter: (node) => {
        if (node.name !== 'FencedCode') return
        const first = view.state.doc.lineAt(node.from).number
        const last = view.state.doc.lineAt(node.to).number
        for (let n = Math.max(first, lastLine + 1); n <= last; n++) {
          const line = view.state.doc.line(n)
          const cls = 'cm-codeblock' + (n === first ? ' cm-codeblock-first' : '') + (n === last ? ' cm-codeblock-last' : '')
          builder.add(line.from, line.from, Decoration.line({ class: cls }))
          lastLine = n
        }
        return false
      },
    })
  }
  return builder.finish()
}

const codeBlockLines = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet
    constructor(view: EditorView) {
      this.decorations = codeBlockDecorations(view)
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) {
        this.decorations = codeBlockDecorations(u.view)
      }
    }
  },
  { decorations: (v) => v.decorations },
)

const wikilinkMarks = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet
    matcher = new MatchDecorator({ regexp: WIKILINK_RE, decoration: Decoration.mark({ class: 'cm-wikilink' }) })
    constructor(view: EditorView) {
      this.decorations = this.matcher.createDeco(view)
    }
    update(u: ViewUpdate) {
      this.decorations = this.matcher.updateDeco(u, this.decorations)
    }
  },
  { decorations: (v) => v.decorations },
)

/** Returns the link target if `pos` is inside a [[wikilink]]. */
function linkAt(view: EditorView, pos: number): string | null {
  const line = view.state.doc.lineAt(pos)
  const offset = pos - line.from
  for (const m of line.text.matchAll(WIKILINK_RE)) {
    const start = m.index ?? 0
    if (offset >= start && offset <= start + m[0].length) {
      return m[1].split(/[|#]/)[0].trim()
    }
  }
  return null
}

const wikilinkCompletions = (getVault: () => string | null) => async (ctx: CompletionContext) => {
  const before = ctx.matchBefore(/\[\[[^\]\n]*/)
  if (!before) return null
  const query = before.text.slice(2)
  const notes = await api.suggestTitles(getVault(), query).catch(() => [])
  const after = ctx.state.sliceDoc(ctx.pos, ctx.pos + 2)
  const close = after === ']]' ? '' : ']]'
  return {
    from: before.from + 2,
    filter: false,
    options: notes.map((n) => ({ label: n.title, type: 'text', apply: n.title + close })),
  }
}

export interface EditorProps {
  value: string
  /** Vault used for [[link]] suggestions. */
  vaultId?: string | null
  /** The text changed; `remote` is true when the change came from a collaborator. */
  onChange: (value: string, remote: boolean) => void
  /** Edit a shared note: the text lives in this Yjs document (then `value` is ignored), and
   *  collaborators' carets are drawn with their names. */
  session?: NoteSession
  onOpenLink?: (title: string) => void
  grammar?: boolean
  placeholder?: string
  autoFocus?: boolean
  /** Receives the CodeMirror view once created (null when destroyed), e.g. for the formatting bar. */
  onReady?: (view: EditorView | null) => void
}

/** Asks the grammar linter to run again. */
const recheck = StateEffect.define<null>()

/**
 * Right-click on a flagged word or phrase (or on selected text): suggestions, and "Ignore" —
 * remembered for every note, until removed in Settings. Returns whether it showed a menu (if not,
 * the normal one appears).
 */
function ignoreMenu(e: MouseEvent, view: EditorView): boolean {
  const pos = view.posAtCoords({ x: e.clientX, y: e.clientY })
  if (pos == null) return false
  let hit: { d: Diagnostic; from: number; to: number } | null = null
  forEachDiagnostic(view.state, (d, from, to) => {
    if (!hit && pos >= from && pos <= to) hit = { d, from, to }
  })
  const sel = view.state.selection.main
  const picked = !sel.empty && pos >= sel.from && pos <= sel.to ? view.state.sliceDoc(sel.from, sel.to) : ''
  const found = hit as { d: Diagnostic; from: number; to: number } | null
  if (!found && !picked.trim()) return false

  const backdrop = document.createElement('div')
  backdrop.className = 'menu-backdrop'
  const menu = document.createElement('div')
  menu.className = 'menu'
  menu.style.cssText = `position:fixed;right:auto;left:${Math.min(e.clientX, window.innerWidth - 250)}px;top:${Math.min(e.clientY, window.innerHeight - 260)}px;min-width:230px;max-width:320px`
  const close = () => (backdrop.remove(), menu.remove(), document.removeEventListener('keydown', onKey, true))
  const onKey = (k: KeyboardEvent) => k.key === 'Escape' && (k.stopPropagation(), close())
  const add = (label: string, run: () => void, hint?: string) => {
    const b = document.createElement('button')
    b.textContent = label.length > 48 ? label.slice(0, 47) + '…' : label
    if (hint) {
      const h = document.createElement('span')
      h.className = 'menu-hint'
      h.textContent = hint
      b.append(h)
      b.style.flexDirection = 'column'
      b.style.alignItems = 'flex-start'
      b.style.gap = '0'
    }
    b.onclick = () => (close(), run())
    menu.append(b)
  }
  const sep = () => menu.append(Object.assign(document.createElement('div'), { className: 'menu-sep' }))

  if (found) {
    const word = view.state.sliceDoc(found.from, found.to)
    const msg = document.createElement('div')
    msg.className = 'menu-label'
    msg.textContent = found.d.message.length > 70 ? found.d.message.slice(0, 69) + '…' : found.d.message
    menu.append(msg)
    for (const a of (found.d.actions ?? []).slice(0, 4)) add(a.name, () => a.apply(view, found.from, found.to))
    if (found.d.actions?.length) sep()
    add(`Ignore “${word.trim()}”`, () => void addIgnored(word), 'Not flagged anywhere, in any note')
  }
  if (picked.trim() && (!found || picked !== view.state.sliceDoc(found.from, found.to))) {
    if (found) sep()
    add('Ignore grammar & spelling for selection', () => void addIgnored(picked), picked.trim().length > 40 ? picked.trim().slice(0, 39) + '…' : picked.trim())
  }
  document.addEventListener('keydown', onKey, true)
  backdrop.onclick = close
  backdrop.oncontextmenu = (ev) => (ev.preventDefault(), close())
  document.body.append(backdrop, menu)
  return true
}

/** Markdown editor with [[link]] autocomplete, Ctrl/Cmd-click to follow links, and offline grammar checks. */
export function Editor({ value, vaultId = null, onChange, onOpenLink, session, grammar = true, placeholder, autoFocus, onReady }: EditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const vaultRef = useRef(vaultId)
  vaultRef.current = vaultId
  const viewRef = useRef<EditorView | null>(null)
  const cbs = useRef({ onChange, onOpenLink })
  cbs.current = { onChange, onOpenLink }
  const grammarSlot = useRef(new Compartment())
  const lintRef = useRef<Extension>([])

  useEffect(() => {
    const grammarLinter = linter(
      async (view): Promise<Diagnostic[]> => {
        const text = view.state.doc.toString()
        if (!text.trim()) return []
        const [found, ignored] = await Promise.all([api.checkGrammar(text).catch(() => []), loadIgnored()])
        if (view.state.doc.toString() !== text) return []
        // Nothing inside a word or sentence the user chose to ignore is flagged.
        const spans = ignoredSpans(text, ignored)
        const issues = found.filter((i) => !spans.some(([a, b]) => i.start >= a && i.end <= b))
        return issues.map((i) => ({
          from: i.start,
          to: Math.max(i.end, i.start + 1),
          severity: 'warning',
          message: i.message,
          actions: i.fixes.map((f) => ({
            name: f.kind === 'remove' ? 'Remove' : f.text || '(empty)',
            apply: (v: EditorView, from: number, to: number) => {
              const insert = f.kind === 'remove' ? '' : f.text
              if (f.kind === 'insert') v.dispatch({ changes: { from: to, insert } })
              else v.dispatch({ changes: { from, to, insert } })
            },
          })),
        }))
      },
      // Ignoring a word re-checks the note even though its text didn't change.
      { delay: 1200, needsRefresh: (u) => u.transactions.some((t) => t.effects.some((e) => e.is(recheck))) },
    )

    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: session ? session.ytext.toString() : value,
        extensions: [
          highlightSpecialChars(),
          session ? yCollab(session.ytext, session.awareness, { undoManager: session.undoManager }) : history(),
          drawSelection(),
          dropCursor(),
          indentOnInput(),
          closeBrackets(),
          highlightSelectionMatches(),
          search({ top: true, createPanel: findPanel }),
          EditorView.lineWrapping,
          markdown({ base: markdownLanguage, codeLanguages: languages }),
          syntaxHighlighting(highlight),
          syntaxHighlighting(codeHighlighter),
          codeBlockLines,
          wikilinkMarks,
          autocompletion({ override: [wikilinkCompletions(() => vaultRef.current)], activateOnTyping: true }),
          grammarSlot.current.of(grammar ? grammarLinter : []),
          cmPlaceholder(placeholder ?? 'Start writing… use [[ to link notes'),
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...(session ? yUndoManagerKeymap : historyKeymap),
            ...searchKeymap,
            ...completionKeymap,
            ...lintKeymap,
            { key: 'Mod-b', run: (v) => (runMarkdown(v, 'bold'), true) },
            { key: 'Mod-i', run: (v) => (runMarkdown(v, 'italic'), true) },
            indentWithTab,
          ]),
          EditorView.updateListener.of((u) => {
            if (!u.docChanged) return
            const remote = u.transactions.some((t) => t.annotation(ySyncAnnotation) !== undefined)
            cbs.current.onChange(u.state.doc.toString(), remote)
          }),
          EditorView.domEventHandlers({
            contextmenu: (e, v) => {
              const menu = ignoreMenu(e, v)
              if (menu) e.preventDefault()
              return menu
            },
            mousedown: (e, v) => {
              if (!(e.ctrlKey || e.metaKey)) return false
              const pos = v.posAtCoords({ x: e.clientX, y: e.clientY })
              const target = pos != null ? linkAt(v, pos) : null
              if (target && cbs.current.onOpenLink) {
                e.preventDefault()
                cbs.current.onOpenLink(target)
                return true
              }
              return false
            },
            // Pasting or dropping image files stores them in the vault and inserts them.
            paste: (e, v) => {
              const files = imageFiles(e.clipboardData?.files)
              if (!files.length) return false
              e.preventDefault()
              insertImages(v, files, vaultRef.current, v.state.selection.main.head)
              return true
            },
            drop: (e, v) => {
              const files = imageFiles(e.dataTransfer?.files)
              if (!files.length) return false
              e.preventDefault()
              const pos = v.posAtCoords({ x: e.clientX, y: e.clientY }) ?? v.state.selection.main.head
              insertImages(v, files, vaultRef.current, pos)
              return true
            },
          }),
        ],
      }),
    })
    viewRef.current = view
    lintRef.current = grammarLinter
    // Ignoring something (here or in Settings) re-checks the note right away.
    const offIgnored = onIgnoredChange(() => (view.dispatch({ effects: recheck.of(null) }), forceLinting(view)))
    onReady?.(view)
    if (autoFocus) view.focus()
    return () => {
      offIgnored()
      onReady?.(null)
      view.destroy()
    }
    // The editor is created once per mount; parents remount it (via `key`) to load another note.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({ effects: grammarSlot.current.reconfigure(grammar ? lintRef.current : []) })
  }, [grammar])

  return <div className="editor-host" ref={host} />
}
