import { useEffect, useState } from 'react'
import { Cloud, Sparkles } from 'lucide-react'
import { api, type BrainStatus } from '../api'
import { Devices } from '../components/Devices'
import { GoogleDriveSettings } from '../components/GoogleDrive'
import { UpdatesSection } from '../components/Updates'
import { ACCENT_PRESETS, MATUGEN_CONFIG, MATUGEN_TEMPLATE, type Theme } from '../lib/theme'
import { CopyBlock } from './BrainView'
import { loadNoteWidth, saveNoteWidth, type NoteWidth } from '../lib/noteWidth'
import { DEFAULT_NAME, avatarFromFile, setAssistantAvatar, setAssistantName, useAssistantProfile } from '../lib/assistantProfile'
import { loadIgnored, onIgnoredChange, removeIgnored } from '../lib/ignored'

interface AppearanceProps {
  theme: Theme
  setTheme: (t: Theme) => void
  accent: string | null
  setAccent: (a: string | null) => void
  matugenPath: string
  matugenFound: boolean
}

export function SettingsView({ theme, setTheme, accent, setAccent, matugenPath, matugenFound }: AppearanceProps) {
  const [grammar, setGrammar] = useState(true)
  const [width, setWidth] = useState<NoteWidth>('readable')
  const [ignored, setIgnored] = useState<string[]>([])
  const assistant = useAssistantProfile()
  const [nameDraft, setNameDraft] = useState<string | null>(null)
  const [avatarError, setAvatarError] = useState('')
  useEffect(() => {
    void loadNoteWidth().then(setWidth)
    const refresh = () => void loadIgnored().then((l) => setIgnored([...l]))
    refresh()
    return onIgnoredChange(refresh)
  }, [])
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
          <button className={'btn' + (theme === 'matugen' ? ' primary' : '')} onClick={() => setTheme('matugen')} title="Colors from your wallpaper, made by matugen">Matugen</button>
        </div>
        {theme === 'matugen' && (
          <div style={{ marginTop: 12 }}>
            <p className="small muted" style={{ marginTop: 0 }}>
              {matugenFound ? (
                <>Following the newest palette matugen wrote, from <span className="kbd">{matugenPath}</span>. They change when your wallpaper does.</>
              ) : (
                <>Waiting for matugen's colors at <span className="kbd">{matugenPath}</span>. Set it up once below, then run matugen (or change your wallpaper).</>
              )}
            </p>
            <CopyBlock label={"1. Save as ~/.config/matugen/templates/astralis.json"} text={MATUGEN_TEMPLATE} />
            <CopyBlock label="2. Add to ~/.config/matugen/config.toml" text={MATUGEN_CONFIG} />
          </div>
        )}
        <div style={{ marginTop: 14 }}>
          <div className="small muted" style={{ marginBottom: 6 }}>Accent color{theme === 'matugen' ? ' (overrides matugen\u2019s)' : ''}</div>
          <div className="row" style={{ gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
            {ACCENT_PRESETS.map((c) => (
              <button
                key={c}
                aria-label={'Accent ' + c}
                title={c}
                onClick={() => setAccent(c === ACCENT_PRESETS[0] ? null : c)}
                style={{ width: 26, height: 26, borderRadius: '50%', background: c, cursor: 'pointer', border: (accent ?? ACCENT_PRESETS[0]) === c ? '2px solid var(--text)' : '2px solid transparent' }}
              />
            ))}
            <label className="btn" style={{ display: 'inline-flex', gap: 8, alignItems: 'center', cursor: 'pointer' }}>
              Custom
              <input type="color" value={accent ?? ACCENT_PRESETS[0]} onChange={(e) => setAccent(e.target.value)} style={{ width: 24, height: 20, padding: 0, border: 0, background: 'none' }} />
            </label>
            {accent && <button className="btn ghost" onClick={() => setAccent(null)}>Reset</button>}
          </div>
        </div>
      </div>
      <div className="section">
        <h2>Notes</h2>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={width === 'full'}
            onChange={(e) => {
              const w = e.target.checked ? 'full' : 'readable'
              setWidth(w)
              saveNoteWidth(w)
            }}
          />
          Fill the whole width (otherwise notes keep a comfortable reading width)
        </label>
      </div>
      <div className="section">
        <h2>Devices</h2>
        <p className="small muted" style={{ marginTop: -6 }}>
          Pair with a device, then share a vault with it from the vault menu (<b>Share with…</b>). Only shared vaults
          sync — directly, no server, end-to-end encrypted — and you edit them together live. Everything else stays private.
        </p>
        <Devices />
      </div>
      <div className="section">
        <h2>Connected drives</h2>
        <p className="small muted" style={{ marginTop: -6 }}>
          Connect your own cloud storage to create cloud vaults. Each drive gets one <span className="kbd">Astralis/</span> folder with a
          subfolder per vault, holding only end-to-end encrypted data. Invite people by email; you then edit together live, P2P.
        </p>
        <GoogleDriveSettings />
        {['Microsoft OneDrive', 'Box', 'Dropbox', 'pCloud'].map((p) => (
          <div key={p} className="row" style={{ padding: '7px 0', borderBottom: '1px solid var(--border)', maxWidth: 520 }}>
            <Cloud size={15} className="muted" />
            <span className="grow">{p}</span>
            <span className="small faint">Coming soon</span>
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
        <p className="small muted" style={{ margin: '10px 0 0' }}>
          Right-click a flagged word, or selected text, in a note to ignore it. Ignored words and sentences are never flagged again, in any note.
        </p>
        {ignored.length > 0 && (
          <div className="ignored-list">
            {ignored.map((w) => (
              <span key={w} className="ignored-chip" title={w}>
                <span>{w}</span>
                <button aria-label={'Stop ignoring ' + w} title="Check this again" onClick={() => void removeIgnored(w)}>×</button>
              </span>
            ))}
          </div>
        )}
      </div>
      <div className="section">
        <h2>Assistant</h2>
        <div className="row" style={{ gap: 14, alignItems: 'center', flexWrap: 'wrap' }}>
          <span className="assistant-orb avatar-preview" style={{ width: 56, height: 56 }}>
            {assistant.avatar ? <img src={assistant.avatar} alt="" /> : <Sparkles size={26} />}
          </span>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 220 }}>
            <label className="small muted" style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              Name
              <input
                className="input"
                value={nameDraft ?? assistant.name}
                maxLength={30}
                placeholder={DEFAULT_NAME}
                onChange={(e) => setNameDraft(e.target.value)}
                onBlur={() => (nameDraft !== null && setAssistantName(nameDraft), setNameDraft(null))}
                onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
              />
            </label>
            <div className="row" style={{ gap: 6 }}>
              <label className="btn" style={{ cursor: 'pointer' }}>
                Choose picture…
                <input
                  type="file"
                  accept="image/*"
                  hidden
                  onChange={(e) => {
                    const f = e.target.files?.[0]
                    e.target.value = ''
                    if (!f) return
                    setAvatarError('')
                    avatarFromFile(f).then(setAssistantAvatar).catch((err: Error) => setAvatarError(err.message))
                  }}
                />
              </label>
              {assistant.avatar && <button className="btn ghost" onClick={() => setAssistantAvatar(null)}>Remove</button>}
              {(assistant.name !== DEFAULT_NAME) && <button className="btn ghost" onClick={() => (setNameDraft(null), setAssistantName(DEFAULT_NAME))}>Reset name</button>}
            </div>
            {avatarError && <span className="small" style={{ color: 'var(--danger)' }}>{avatarError}</span>}
          </div>
        </div>
        <p className="small muted" style={{ margin: '10px 0 0' }}>Shown in the note chat. The picture is cropped to a square and kept on this device.</p>
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
        <h2>Updates</h2>
        <UpdatesSection />
      </div>
      <div className="section">
        <h2>Privacy</h2>
        <ul className="small muted" style={{ margin: 0, paddingLeft: 18, lineHeight: 1.9 }}>
          <li>Everything is stored locally in a SQLite database in your app data folder.</li>
          <li>The AI runs on this device ({brain ? (brain.neural ? 'neural MiniLM model' : 'built-in lexical model') : '…'}). No note ever leaves your computer.</li>
          <li>Vault notes are encrypted at rest and excluded from search, links, history and the AI.</li>
          <li>Shared and cloud vaults sync end-to-end encrypted: device to device (P2P), and through your own drive, which only ever holds ciphertext. The vault key never goes to the drive.</li>
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
