import { useEffect, useState } from 'react'
import { Cloud } from 'lucide-react'
import { api, type BrainStatus } from '../api'

export function SettingsView({ theme, setTheme }: { theme: 'dark' | 'light'; setTheme: (t: 'dark' | 'light') => void }) {
  const [grammar, setGrammar] = useState(true)
  const [brain, setBrain] = useState<BrainStatus | null>(null)

  useEffect(() => {
    api.getSetting('grammar').then((v) => setGrammar(v !== 'off')).catch(() => {})
    api.brainStatus().then(setBrain).catch(() => {})
  }, [])

  return (
    <div style={{ height: '100%', overflow: 'auto' }}>
      <div className="view-header"><h1>Settings</h1></div>
      <div className="section">
        <h2>Appearance</h2>
        <div className="row">
          <button className={'btn' + (theme === 'dark' ? ' primary' : '')} onClick={() => setTheme('dark')}>Dark</button>
          <button className={'btn' + (theme === 'light' ? ' primary' : '')} onClick={() => setTheme('light')}>Light</button>
        </div>
      </div>
      <div className="section">
        <h2>Connected drives</h2>
        <p className="small muted" style={{ marginTop: -6 }}>
          Connect your own cloud storage to create cloud vaults. Each drive gets one <span className="kbd">Astralis/</span> folder with a
          subfolder per vault. Coming soon.
        </p>
        {['Google Drive', 'Microsoft OneDrive', 'Box', 'Dropbox', 'pCloud'].map((p) => (
          <div key={p} className="row" style={{ padding: '7px 0', borderBottom: '1px solid var(--border)', maxWidth: 520 }}>
            <Cloud size={15} className="muted" />
            <span className="grow">{p}</span>
            <span className="small faint">Not connected</span>
            <button className="btn sm" disabled title="Coming soon">Connect</button>
          </div>
        ))}
      </div>
      <div className="section">
        <h2>Writing</h2>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={grammar}
            onChange={(e) => {
              setGrammar(e.target.checked)
              api.setSetting('grammar', e.target.checked ? 'on' : 'off')
            }}
          />
          Grammar &amp; spelling check (offline, rule-based — Harper)
        </label>
      </div>
      <div className="section">
        <h2>AI model</h2>
        <div className="row" style={{ gap: 28, alignItems: "flex-start" }}>
          <div className="stat">
            <b style={{ fontSize: 15 }}>{brain ? (brain.neural ? "Neural (MiniLM)" : "Built-in lexical") : "…"}</b>
            <span className="small muted">{brain?.loaded ? "Loaded in memory" : "Unloaded — loads when you use an AI tool"}</span>
          </div>
          <div className="stat">
            <b style={{ fontSize: 15 }}>{brain?.indexed ?? "–"}</b>
            <span className="small muted">notes understood</span>
          </div>
          <div className="stat">
            <b style={{ fontSize: 15 }}>{brain?.pending ?? "–"}</b>
            <span className="small muted">waiting</span>
          </div>
        </div>
        {brain && !brain.neural && (
          <p className="small faint" style={{ marginBottom: 0 }}>
            For smarter matching, install the neural model: <span className="kbd">./run.sh fetch-model</span>
          </p>
        )}
      </div>
      <div className="section">
        <h2>Privacy</h2>
        <ul className="small muted" style={{ margin: 0, paddingLeft: 18, lineHeight: 1.9 }}>
          <li>Everything is stored locally in a SQLite database in your app data folder.</li>
          <li>The AI runs on this device ({brain ? (brain.neural ? 'neural MiniLM model' : 'built-in lexical model') : '…'}). No note ever leaves your computer.</li>
          <li>Vault notes are encrypted at rest and excluded from search, links, history and the AI.</li>
          <li>Cloud sync and collaboration are coming in a later version — always end-to-end encrypted, to storage you own.</li>
        </ul>
      </div>
      <div className="section">
        <h2>Keyboard shortcuts</h2>
        <table className="small">
          <tbody>
            {[
              ['Ctrl K', 'Search & commands'],
              ['Ctrl N', 'New note'],
              ['Ctrl Shift T', 'Go to tasks'],
              ['[[', 'Link a note (in the editor)'],
              ['Ctrl click', 'Follow a link (in the editor)'],
              ['Ctrl F', 'Find in note'],
            ].map(([k, d]) => (
              <tr key={k}>
                <td style={{ padding: '4px 16px 4px 0' }}><span className="kbd">{k}</span></td>
                <td className="muted">{d}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
