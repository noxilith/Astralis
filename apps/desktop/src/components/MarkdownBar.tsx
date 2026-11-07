import { useRef, type ReactNode } from 'react'
import type { EditorView } from '@codemirror/view'
import {
  Bold,
  Brackets,
  Code,
  Heading1,
  Heading2,
  Heading3,
  ImagePlus,
  Italic,
  Link,
  List,
  ListChecks,
  ListOrdered,
  Minus,
  SquareCode,
  Strikethrough,
  Table,
  TextQuote,
} from 'lucide-react'
import { runMarkdown, type MdCommand } from '../lib/markdown'
import { imageFiles, insertImages } from '../lib/attachments'

const GROUPS: { cmd: MdCommand; label: string; icon: ReactNode }[][] = [
  [
    { cmd: 'h1', label: 'Heading 1', icon: <Heading1 size={16} /> },
    { cmd: 'h2', label: 'Heading 2', icon: <Heading2 size={16} /> },
    { cmd: 'h3', label: 'Heading 3', icon: <Heading3 size={16} /> },
  ],
  [
    { cmd: 'bold', label: 'Bold (Ctrl B)', icon: <Bold size={15} /> },
    { cmd: 'italic', label: 'Italic (Ctrl I)', icon: <Italic size={15} /> },
    { cmd: 'strike', label: 'Strikethrough', icon: <Strikethrough size={15} /> },
    { cmd: 'code', label: 'Inline code', icon: <Code size={15} /> },
  ],
  [
    { cmd: 'link', label: 'Link', icon: <Link size={15} /> },
    { cmd: 'wikilink', label: 'Link a note [[ ]]', icon: <Brackets size={15} /> },
  ],
  [
    { cmd: 'ul', label: 'Bulleted list', icon: <List size={16} /> },
    { cmd: 'ol', label: 'Numbered list', icon: <ListOrdered size={16} /> },
    { cmd: 'task', label: 'Checklist', icon: <ListChecks size={16} /> },
    { cmd: 'quote', label: 'Quote', icon: <TextQuote size={16} /> },
  ],
  [
    { cmd: 'codeblock', label: 'Code block', icon: <SquareCode size={16} /> },
    { cmd: 'table', label: 'Table', icon: <Table size={15} /> },
    { cmd: 'hr', label: 'Divider', icon: <Minus size={16} /> },
  ],
]

/** Formatting bar: applies Markdown at the caret, or to the selected text. */
export function MarkdownBar({ getView, vaultId }: { getView: () => EditorView | null; vaultId: string | null }) {
  const fileInput = useRef<HTMLInputElement>(null)
  return (
    <div className="md-bar" role="toolbar" aria-label="Formatting">
      <input
        ref={fileInput}
        type="file"
        accept="image/*"
        multiple
        hidden
        onChange={(e) => {
          const view = getView()
          const files = imageFiles(e.target.files)
          e.target.value = ""
          if (view && files.length) insertImages(view, files, vaultId, view.state.selection.main.head)
        }}
      />
      {GROUPS.map((group, i) => (
        <div key={i} className="md-group">
          {group.map((b) => (
            <button
              key={b.cmd}
              className="md-btn"
              title={b.label}
              aria-label={b.label}
              // Keep the editor's selection: don't take focus on mousedown.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                const view = getView()
                if (view) runMarkdown(view, b.cmd)
              }}
            >
              {b.icon}
            </button>
          ))}
        </div>
      ))}
      <div className="md-group">
        <button
          className="md-btn"
          title="Insert image (or paste / drop one into the note)"
          aria-label="Insert image"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => fileInput.current?.click()}
        >
          <ImagePlus size={16} />
        </button>
      </div>
    </div>
  )
}
