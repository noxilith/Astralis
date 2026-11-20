import { useEffect, useRef, useState } from 'react'
import { Cloud, FileCode, PanelRight, Play, Square, Terminal, X } from 'lucide-react'
import { CodeEditor } from '../components/CodeEditor'
import { ProjectsSync } from '../components/ProjectsSync'
import { Empty, useConfirm } from '../components/ui'
import { useApp } from '../context'
import { COMPACT_WIDTH, PHONE } from '../lib/platform'
import {
  activateFile,
  clearOutput,
  closeFile,
  editFile,
  key,
  resumeRunIfAny,
  saveFile,
  setOutputOpen,
  startRun,
  stopRun,
  useProjectsState,
} from '../lib/projectsState'

const ANSI = /\x1b\[[0-9;?]*[A-Za-z]/g // eslint-disable-line no-control-regex
const base = (p: string) => p.slice(p.lastIndexOf('/') + 1)

/** Projects: your code like your notes. Files in the sidebar, open files here, Run at the top right, sync on the right. */
export function ProjectsView() {
  const s = useProjectsState()
  const { fail, toast } = useApp()
  const confirm = useConfirm()
  const [showPanel, setShowPanel] = useState(() => !(PHONE || window.innerWidth <= COMPACT_WIDTH))
  const out = useRef<HTMLPreElement>(null)

  const hash = s.current
  const project = s.projects.find((p) => p.hash === hash)
  const tabs = hash ? s.tabs[hash] : undefined
  const active = tabs?.active ?? null
  const file = hash && active ? s.loaded[key(hash, active)] : undefined
  const draft = hash && active ? s.drafts[key(hash, active)] : undefined
  const run = hash ? s.run[hash] : undefined
  const info = s.runInfo
  const running = !!run?.running || !!info?.running

  useEffect(() => resumeRunIfAny(), [hash, info?.running])
  useEffect(() => {
    if (out.current) out.current.scrollTop = out.current.scrollHeight
  }, [run?.lines.length])

  const save = (path: string) => saveFile(path).then(() => toast('Saved')).catch(fail)

  const onRun = async () => {
    if (!hash || !project) return
    if (running) return void stopRun().catch(fail)
    if (!info) return toast('No run command yet. Ask your AI tool to add one: it writes .astralis/run.json for the Run button.')
    let confirmed = false
    if (!info.trusted) {
      const ok = await confirm.ask(`Run “${info.command}” in ${project.root}? It comes from ${info.file}, and runs with your permissions.`, 'Run')
      if (!ok) return
      confirmed = true
    }
    startRun(confirmed).catch(fail)
  }

  const status = running ? 'Running…' : run?.exit != null ? (run.exit === 0 ? 'Finished' : `Exited with code ${run.exit}`) : ''

  return (
    <div className={'split' + (showPanel ? ' split-editor' : '')} style={showPanel ? undefined : { gridTemplateColumns: 'minmax(0,1fr)' }}>
      {confirm.element}
      <div className="pane proj-main">
        <div className="editor-toolbar proj-bar">
          <div className="proj-tabs" role="tablist">
            {(tabs?.open ?? []).map((p) => {
              const dirty = !!hash && s.drafts[key(hash, p)] !== undefined
              return (
                <div key={p} role="tab" aria-selected={p === active} className={'proj-tab' + (p === active ? ' active' : '')} onClick={() => activateFile(p)} title={p}>
                  <span className="proj-tab-name">{base(p)}</span>
                  {dirty && <span className="proj-dirty" title="Unsaved changes" />}
                  <button
                    className="proj-tab-x"
                    aria-label={`Close ${base(p)}`}
                    onClick={(e) => {
                      e.stopPropagation()
                      if (dirty && !window.confirm(`Close ${base(p)} without saving?`)) return
                      closeFile(p)
                    }}
                  >
                    <X size={12} />
                  </button>
                </div>
              )
            })}
          </div>
          <span className="grow" />
          <button
            className={'btn sm proj-run' + (running ? ' danger' : info ? ' primary' : '')}
            onClick={onRun}
            disabled={!project}
            title={info ? `${info.command}  (from ${info.file})` : 'No run command yet. Your AI tool adds one in .astralis/run.json'}
          >
            {running ? <Square size={13} /> : <Play size={13} />} {running ? 'Stop' : info?.name && info.name !== 'Run' ? `Run · ${info.name}` : 'Run'}
          </button>
          <button className={'icon-btn side-btn' + (showPanel ? ' active' : '')} title="Side panel: cloud sync" onClick={() => setShowPanel((v) => !v)}>
            <PanelRight size={16} />
          </button>
        </div>

        <div className="proj-editor">
          {!project ? (
            <Empty icon={<FileCode size={28} />} title="No project open">
              Pick a project in the sidebar, or add a folder in the Sync tab.
            </Empty>
          ) : !active ? (
            <Empty icon={<FileCode size={28} />} title={project.name}>
              <span className="small faint">Open a file from the sidebar. Ctrl S saves.</span>
            </Empty>
          ) : !file ? null : file.binary ? (
            <Empty title="Binary file">{base(active)} can't be shown as text.</Empty>
          ) : file.tooBig ? (
            <Empty title="File too large">{base(active)} is over 1.5 MB, so it isn't opened here.</Empty>
          ) : (
            <CodeEditor key={(hash ?? '') + active} path={active} value={draft ?? file.text ?? ''} onChange={(t) => editFile(active, t)} onSave={() => save(active)} />
          )}
        </div>

        {run && (run.lines.length > 0 || run.running) && s.outputOpen && (
          <div className="proj-output">
            <div className="proj-output-bar">
              <Terminal size={13} />
              <strong>{info?.name ?? 'Output'}</strong>
              <span className="small faint">{status}</span>
              <span className="grow" />
              <button className="btn ghost sm" onClick={clearOutput}>Clear</button>
              <button className="icon-btn" title="Hide output" onClick={() => setOutputOpen(false)}><X size={14} /></button>
            </div>
            <pre className="proj-output-body" ref={out}>{run.lines.map((l) => l.replace(ANSI, '')).join('\n')}</pre>
          </div>
        )}
        {run && (run.lines.length > 0 || run.running) && !s.outputOpen && (
          <button className="proj-output-show" onClick={() => setOutputOpen(true)}>
            <Terminal size={13} /> Show output {status && `· ${status}`}
          </button>
        )}
      </div>

      {showPanel && (
        <div className="pane side-pane">
          <div className="side-tabs" role="tablist">
            <button role="tab" aria-selected className="active">
              <Cloud size={14} /> Sync
            </button>
          </div>
          <div className="side-body">
            <ProjectsSync />
          </div>
        </div>
      )}
    </div>
  )
}
