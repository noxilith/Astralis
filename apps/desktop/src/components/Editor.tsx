import { useEffect, useRef } from 'react'
import { EditorState, Compartment, type Extension } from '@codemirror/state'
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
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search'
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, type CompletionContext } from '@codemirror/autocomplete'
import { linter, lintKeymap, type Diagnostic } from '@codemirror/lint'
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
        const issues = await api.checkGrammar(text).catch(() => [])
        if (view.state.doc.toString() !== text) return []
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
      { delay: 1200 },
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
    onReady?.(view)
    if (autoFocus) view.focus()
    return () => {
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
