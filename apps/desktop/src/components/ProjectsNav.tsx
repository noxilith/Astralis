import { useEffect, useMemo, useState } from 'react'
import { Check, ChevronDown, ChevronRight, ChevronsUpDown, File, FileCode, FileText, Folder, FolderGit2, Search } from 'lucide-react'
import { openFile, refreshFiles, refreshProjects, selectProject, useProjectsState } from '../lib/projectsState'

interface Node {
  name: string
  path: string
  children: Node[]
  file: boolean
}

function buildTree(paths: string[]): Node[] {
  const root: Node = { name: '', path: '', children: [], file: false }
  for (const p of paths) {
    let at = root
    const parts = p.split('/')
    parts.forEach((name, i) => {
      const path = parts.slice(0, i + 1).join('/')
      let next = at.children.find((c) => c.path === path)
      if (!next) {
        next = { name, path, children: [], file: i === parts.length - 1 }
        at.children.push(next)
      }
      at = next
    })
  }
  const order = (n: Node) => {
    n.children.sort((a, b) => Number(a.file) - Number(b.file) || a.name.localeCompare(b.name))
    n.children.forEach(order)
  }
  order(root)
  return root.children
}

const CODE = /\.(rs|ts|tsx|js|jsx|mjs|cjs|py|go|java|kt|c|h|cpp|hpp|cs|rb|php|swift|sh|bash|zsh|fish|lua|sql|css|scss|html|vue|svelte|toml|yaml|yml|json)$/i
const fileIcon = (name: string) => (/\.(md|mdx|txt|rst)$/i.test(name) ? <FileText size={13} /> : CODE.test(name) ? <FileCode size={13} /> : <File size={13} />)

/** The sidebar while Projects is open: pick a project, browse its files. */
export function ProjectsNav() {
  const s = useProjectsState()
  const [filter, setFilter] = useState('')
  const [closed, setClosed] = useState<Set<string>>(new Set())
  const [menu, setMenu] = useState(false)

  useEffect(() => {
    void refreshProjects()
    // Claude and your editor change files while this is open.
    const t = window.setInterval(() => {
      void refreshProjects()
      void refreshFiles()
    }, 8000)
    return () => window.clearInterval(t)
  }, [])

  const tree = useMemo(() => buildTree(s.files), [s.files])
  const q = filter.trim().toLowerCase()
  const matches = useMemo(() => (q ? s.files.filter((f) => f.toLowerCase().includes(q)).slice(0, 300) : []), [s.files, q])
  const active = s.current ? s.tabs[s.current]?.active : null

  const rows = (nodes: Node[], depth: number): React.ReactNode =>
    nodes.map((n) => {
      if (n.file)
        return (
          <div key={n.path} className={'tree-row' + (n.path === active ? ' active' : '')} style={{ paddingLeft: 8 + depth * 14 + 15 }} onClick={() => openFile(n.path)} title={n.path}>
            <span className="tree-icon">{fileIcon(n.name)}</span>
            <span className="tree-label">{n.name}</span>
          </div>
        )
      const open = !closed.has(n.path)
      return (
        <div key={n.path}>
          <div
            className="tree-row"
            style={{ paddingLeft: 8 + depth * 14 }}
            onClick={() => setClosed((c) => { const x = new Set(c); if (open) x.add(n.path); else x.delete(n.path); return x })}
          >
            <span className="tree-caret">{open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</span>
            <span className="tree-icon folder"><Folder size={13} /></span>
            <span className="tree-label">{n.name}</span>
          </div>
          {open && rows(n.children, depth + 1)}
        </div>
      )
    })

  return (
    <div className="notes-nav">
      <div className="notes-nav-head">
        <div className="row">
          <div style={{ position: 'relative' }} className="grow">
            <button className="vault-btn" onClick={() => setMenu((o) => !o)} title="Switch project">
              <FolderGit2 size={15} />
              <span className="grow ellipsis">{s.projects.find((p) => p.hash === s.current)?.name ?? 'No projects yet'}</span>
              <ChevronsUpDown size={14} className="faint" />
            </button>
            {menu && (
              <>
                <div className="menu-backdrop" onClick={() => setMenu(false)} />
                <div className="menu vault-menu">
                  <div className="menu-label">Projects</div>
                  {s.projects.length === 0 && <div className="small faint" style={{ padding: '6px 10px 8px' }}>None on this device yet.</div>}
                  {s.projects.map((p) => (
                    <button key={p.hash} onClick={() => (selectProject(p.hash), setMenu(false))}>
                      <FolderGit2 size={14} />
                      <span className="grow ellipsis">{p.name}</span>
                      {p.status === 'behind' && <span className="faint small">cloud is newer</span>}
                      {p.status === 'off' && <span className="faint small">not synced</span>}
                      {p.hash === s.current ? <Check size={14} /> : <span style={{ width: 14 }} />}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
        </div>
        <div className="search-box grow">
          <Search size={13} className="faint" />
          <input placeholder="Filter files…" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </div>
      </div>
      <div className="notes-nav-body">
        {s.projects.length === 0 ? (
          <div className="tree-root-hint">
            <FolderGit2 size={16} />
            <div>Projects appear here when your AI tool works in one, or add a folder in the Sync tab on the right.</div>
          </div>
        ) : s.filesError ? (
          <div className="small faint" style={{ padding: '10px 16px' }}>{s.filesError}</div>
        ) : q ? (
          matches.length === 0 ? (
            <div className="small faint" style={{ padding: '10px 16px' }}>No files match.</div>
          ) : (
            matches.map((f) => (
              <div key={f} className={'tree-row' + (f === active ? ' active' : '')} style={{ paddingLeft: 12 }} onClick={() => openFile(f)} title={f}>
                <span className="tree-icon">{fileIcon(f)}</span>
                <span className="tree-label">{f}</span>
              </div>
            ))
          )
        ) : (
          <div className="tree">{rows(tree, 0)}</div>
        )}
      </div>
    </div>
  )
}
