import { useCallback, useEffect, useState } from 'react'
import { FileText, Plus, Trash2 } from 'lucide-react'
import { api, type Template } from '../api'
import { useApp } from '../context'
import { Empty, useConfirm } from '../components/ui'

const HELP = 'Placeholders: {{date}} {{time}} {{datetime}} {{weekday}} {{year}} {{title}}'

export function TemplatesView() {
  const { version, bump, fail, toast, openNote, vaultId } = useApp()
  const [templates, setTemplates] = useState<Template[]>([])
  const [selected, setSelected] = useState<Template | null>(null)
  const [draft, setDraft] = useState({ name: '', title: '', body: '' })
  const confirm = useConfirm()

  const load = useCallback(() => api.listTemplates().then(setTemplates).catch(fail), [fail])
  useEffect(() => {
    load()
  }, [load, version])

  const select = (t: Template) => {
    setSelected(t)
    setDraft({ name: t.name, title: t.title, body: t.body })
  }

  const dirty = selected && (draft.name !== selected.name || draft.title !== selected.title || draft.body !== selected.body)

  return (
    <div className="split split-2">
      <div className="pane">
        <div className="pane-header">
          <strong className="grow">Templates</strong>
          <button
            className="btn primary sm"
            onClick={() =>
              api
                .createTemplate({ name: 'New template', title: '{{title}}', body: '' })
                .then((t) => (bump(), select(t)))
                .catch(fail)
            }
          >
            <Plus size={14} /> New
          </button>
        </div>
        <div className="list">
          {templates.map((t) => (
            <div key={t.id} className={'list-item' + (t.id === selected?.id ? ' active' : '')} onClick={() => select(t)}>
              <div className="title">{t.name}</div>
              <div className="meta">{t.builtin ? 'Built-in' : 'Custom'}</div>
            </div>
          ))}
        </div>
      </div>
      <div className="pane">
        {selected ? (
          <div style={{ padding: 20, maxWidth: 820 }}>
            <div className="field">
              <label>Name</label>
              <input className="input" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
            </div>
            <div className="field">
              <label>Note title</label>
              <input className="input" value={draft.title} onChange={(e) => setDraft({ ...draft, title: e.target.value })} />
            </div>
            <div className="field">
              <label>Body (Markdown)</label>
              <textarea
                className="textarea"
                style={{ minHeight: 320, fontFamily: 'var(--mono)', fontSize: 13 }}
                value={draft.body}
                onChange={(e) => setDraft({ ...draft, body: e.target.value })}
              />
              <span className="small faint">{HELP}</span>
            </div>
            <div className="row">
              <button
                className="btn primary"
                disabled={!dirty || !draft.name.trim()}
                onClick={() => api.updateTemplate(selected.id, draft).then((t) => (select(t), bump(), toast('Template saved'))).catch(fail)}
              >
                Save
              </button>
              <button
                className="btn"
                onClick={() => api.noteFromTemplate(selected.id, undefined, vaultId).then((n) => (bump(), openNote(n.id))).catch(fail)}
              >
                <FileText size={14} /> New note from template
              </button>
              <span className="grow" />
              <button
                className="btn danger"
                onClick={async () => {
                  if (await confirm.ask(`Delete template “${selected.name}”?`, 'Delete')) {
                    api.deleteTemplate(selected.id).then(() => (setSelected(null), bump())).catch(fail)
                  }
                }}
              >
                <Trash2 size={14} /> Delete
              </button>
            </div>
          </div>
        ) : (
          <Empty icon={<FileText size={32} />} title="Reusable templates">
            <span className="small">Meetings, journals, projects… pick one to edit, or save any note as a template.</span>
          </Empty>
        )}
      </div>
      {confirm.element}
    </div>
  )
}
