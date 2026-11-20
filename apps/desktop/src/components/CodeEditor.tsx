import { useEffect, useRef } from 'react'
import { Compartment, EditorState } from '@codemirror/state'
import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { bracketMatching, indentOnInput, LanguageDescription, syntaxHighlighting } from '@codemirror/language'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { highlightSelectionMatches, search, searchKeymap } from '@codemirror/search'
import { languages } from '@codemirror/language-data'
import { findPanel } from '../lib/findPanel'
import { codeHighlighter } from '../lib/highlight'

/** A source file with syntax colors (the language comes from the file name; loaded on demand). */
export function CodeEditor({ path, value, onChange, onSave }: { path: string; value: string; onChange: (text: string) => void; onSave: () => void }) {
  const host = useRef<HTMLDivElement>(null)
  const callbacks = useRef({ onChange, onSave })
  callbacks.current = { onChange, onSave }

  // One editor per file (the parent keys it by path); the text only seeds it.
  useEffect(() => {
    const language = new Compartment()
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          highlightActiveLine(),
          history(),
          drawSelection(),
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          highlightSelectionMatches(),
          search({ top: true, createPanel: findPanel }),
          syntaxHighlighting(codeHighlighter),
          language.of([]),
          keymap.of([
            { key: 'Mod-s', preventDefault: true, run: () => (callbacks.current.onSave(), true) },
            indentWithTab,
            ...closeBracketsKeymap,
            ...searchKeymap,
            ...historyKeymap,
            ...defaultKeymap,
          ]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) callbacks.current.onChange(u.state.doc.toString())
          }),
        ],
      }),
    })
    let alive = true
    const desc = LanguageDescription.matchFilename(languages, path)
    desc?.load().then((support) => alive && view.dispatch({ effects: language.reconfigure(support) })).catch(() => {})
    return () => {
      alive = false
      view.destroy()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path])

  return <div className="code-editor" ref={host} />
}
