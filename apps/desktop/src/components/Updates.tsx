import { useCallback, useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { ArrowUpCircle, CheckCircle2, Download, ExternalLink, RefreshCw } from 'lucide-react'
import { api, type UpdateInfo } from '../api'
import { useApp } from '../context'
import { Markdown } from './Markdown'

const mb = (bytes: number) => `${(bytes / 1048576).toFixed(bytes > 104857600 ? 0 : 1)} MB`

/** Checks GitHub for a newer release a few seconds after start (unless turned off in Settings)
 *  and offers it in a toast. */
let checkedAtStart = false

export function useStartupUpdateCheck() {
  const { toast, setView } = useApp()
  useEffect(() => {
    if (checkedAtStart) return
    const timer = setTimeout(async () => {
      // Once per run (set here, not before the timer: a cancelled timer mustn't count).
      if (checkedAtStart) return
      checkedAtStart = true
      try {
        if ((await api.getSetting('updates.auto')) === 'off') return
        const info = await api.updateCheck()
        if (!info.latest || (await api.getSetting('updates.skip')) === info.latest) return
        toast(`Astralis ${info.latest} is out`, {
          timeout: 15000,
          actions: [
            { label: 'See what’s new', run: () => setView('settings') },
            { label: 'Skip this one', run: () => void api.setSetting('updates.skip', info.latest!) },
          ],
        })
      } catch {
        // Offline or GitHub unreachable: say nothing, it'll check next time.
      }
    }, 6000)
    return () => clearTimeout(timer)
  }, [toast, setView])
}

/** Settings → Updates. */
export function UpdatesSection() {
  const { fail } = useApp()
  const [auto, setAuto] = useState(true)
  const [info, setInfo] = useState<UpdateInfo | null>(null)
  const [checking, setChecking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [installing, setInstalling] = useState(false)
  const [progress, setProgress] = useState<{ stage: string; done: number; total: number } | null>(null)

  const check = useCallback(async () => {
    setChecking(true)
    setError(null)
    try {
      setInfo(await api.updateCheck())
    } catch (e) {
      setError(String((e as { message?: string })?.message ?? e))
    } finally {
      setChecking(false)
    }
  }, [])

  useEffect(() => {
    api.getSetting('updates.auto').then((v) => setAuto(v !== 'off')).catch(() => {})
    void check()
  }, [check])
  useEffect(() => {
    const off = listen<{ stage: string; done: number; total: number }>('update-progress', ({ payload }) => setProgress(payload))
    return () => void off.then((f) => f())
  }, [])

  const install = async () => {
    setInstalling(true)
    setProgress({ stage: 'download', done: 0, total: info?.size ?? 0 })
    try {
      // On success the app quits and the new version starts.
      await api.updateInstall()
    } catch (e) {
      setInstalling(false)
      setProgress(null)
      fail(e)
    }
  }

  const pct = progress && progress.total ? Math.round((progress.done / progress.total) * 100) : null
  const stageText = !progress
    ? ''
    : progress.stage === 'download'
      ? `Downloading${progress.total ? ` · ${mb(progress.done)} of ${mb(progress.total)}` : '…'}`
      : progress.stage === 'verify'
        ? 'Checking the download…'
        : 'Installing — Astralis will restart…'

  return (
    <div className="updates">
      <div className="row" style={{ gap: 28, alignItems: 'flex-start' }}>
        <div className="stat">
          <b style={{ fontSize: 15 }}>{info?.current ?? '…'}</b>
          <span className="small muted">This version</span>
        </div>
        <div className="stat">
          <b style={{ fontSize: 15 }}>{info?.install ?? '…'}</b>
          <span className="small muted">Installed as</span>
        </div>
      </div>

      {error ? (
        <p className="small muted">Couldn’t check for updates: {error}</p>
      ) : info && !info.latest ? (
        <p className="row small" style={{ gap: 6 }}>
          <CheckCircle2 size={14} style={{ color: 'var(--success)' }} /> You have the newest version.
        </p>
      ) : info?.latest ? (
        <div className="update-card">
          <div className="row" style={{ gap: 8 }}>
            <ArrowUpCircle size={18} style={{ color: 'var(--accent-strong)' }} />
            <b className="grow">Astralis {info.latest} is available</b>
            {info.size > 0 && <span className="small faint">{mb(info.size)}</span>}
          </div>
          {info.notes.trim() && (
            <details className="update-notes">
              <summary className="small">What’s new</summary>
              <Markdown source={info.notes} />
            </details>
          )}
          {installing ? (
            <div className="assistant-progress">
              <div className="assistant-bar">
                <span style={{ width: `${pct ?? 3}%` }} />
              </div>
              <span className="small muted">{stageText}</span>
            </div>
          ) : info.canInstall ? (
            <div className="row" style={{ gap: 8 }}>
              <button className="btn primary" onClick={install}>
                <Download size={14} /> Update now
              </button>
              <button className="btn ghost" onClick={() => api.openUrl(info.page).catch(fail)}>
                <ExternalLink size={14} /> Release page
              </button>
            </div>
          ) : (
            <div className="row" style={{ gap: 8 }}>
              <button className="btn primary" onClick={() => api.openUrl(info.page).catch(fail)}>
                <ExternalLink size={14} /> Download from GitHub
              </button>
              <span className="small faint">This copy ({info.install}) can’t update itself.</span>
            </div>
          )}
        </div>
      ) : null}

      <div className="row" style={{ gap: 14 }}>
        <button className="btn sm" onClick={check} disabled={checking || installing}>
          <RefreshCw size={13} className={checking ? 'spin' : ''} /> {checking ? 'Checking…' : 'Check now'}
        </button>
        <label className="checkbox small">
          <input
            type="checkbox"
            checked={auto}
            onChange={(e) => {
              setAuto(e.target.checked)
              api.setSetting('updates.auto', e.target.checked ? 'on' : 'off').catch(fail)
            }}
          />
          Check for updates when Astralis starts
        </label>
      </div>
    </div>
  )
}
