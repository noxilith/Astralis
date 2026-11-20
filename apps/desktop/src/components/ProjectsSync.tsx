import { useCallback, useEffect, useState } from 'react'
import { Cloud, CloudDownload, FolderGit2, FolderPlus, RefreshCw, X } from 'lucide-react'
import { api, type ProjectRow, type ProjectsInfo } from '../api'
import { useApp } from '../context'
import { Spinner, useConfirm } from './ui'
import { relativeTime } from '../lib/time'
import { refreshProjects } from '../lib/projectsState'

const STATUS: Record<string, { label: string; hint: string }> = {
  synced: { label: 'In the cloud', hint: 'The cloud copy is up to date with this device.' },
  waiting: { label: 'Waiting to upload', hint: 'A snapshot is taken when your AI tool saves a hand-off, then uploaded by Astralis.' },
  behind: { label: 'Cloud is newer', hint: 'Another device has newer work. Update this project from the cloud; until you do, this device does not upload over it.' },
  off: { label: 'Not synced', hint: 'This project stays on this device.' },
  restore: { label: 'Not on this device', hint: 'This project is in your cloud drive. Restore it into a folder here.' },
  missing: { label: 'Folder not found', hint: 'The project folder is gone from this device and there is no cloud copy.' },
}

const size = (b: number) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1e3))} KB`)

/** Keeping projects in your cloud drive: what is synced, restoring on a new computer, updating. */
export function ProjectsSync() {
  const { fail, toast } = useApp()
  const [info, setInfo] = useState<ProjectsInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [adding, setAdding] = useState('')
  const [restoring, setRestoring] = useState<string | null>(null)
  const [dest, setDest] = useState('')
  const confirm = useConfirm()

  const load = useCallback(() => api.projectsList().then(setInfo).catch(fail), [fail])
  useEffect(() => {
    load()
    const t = window.setInterval(load, 10_000)
    return () => window.clearInterval(t)
  }, [load])

  const run = async (job: () => Promise<unknown>, done?: string) => {
    setBusy(true)
    try {
      await job()
      if (done) toast(done)
    } catch (e) {
      fail(e)
    } finally {
      setBusy(false)
      load()
      void refreshProjects()
    }
  }

  // Opening the panel on a new computer looks in the cloud once.
  useEffect(() => {
    api.projectsSyncNow().then(load).catch(() => {})
  }, [load])

  if (!info) return <div className="pad"><Spinner /></div>

  const restore = (r: ProjectRow) =>
    run(async () => {
      const where = await api.projectsRestore(r.hash, dest)
      setRestoring(null)
      toast(`Restored to ${where}`)
    })

  return (
    <div className="proj-sync">
      {confirm.element}
      <p className="proj-intro">
        Your projects, kept in your own cloud drive (end-to-end encrypted) so you can pick them up on another computer. Each time your AI tool
        saves a hand-off, the project is added here if it isn't yet and a snapshot is uploaded, uncommitted work included. Nothing is ever
        downloaded unless you press Restore or Update. Files git ignores (<code>.env</code>, builds, <code>node_modules</code>) are left out; add
        more to a <code>.astralisignore</code> file in the project (your AI tool can write it).
      </p>

      {!info.drive ? (
        <div className="proj-note">
          <Cloud size={16} /> Connect Google Drive or OneDrive in Settings → Connected drives first.
        </div>
      ) : (
        <div className="proj-top">
          <label className="proj-auto">
            <input type="checkbox" checked={info.auto} onChange={(e) => run(() => api.projectsSetAuto(e.target.checked))} />
            <span>
              Keep every project my AI tools work in <span className="muted">· {info.drive}</span>
            </span>
          </label>
          <button className="btn sm" disabled={busy} onClick={() => run(() => api.projectsSyncNow(), 'Projects synced')} title="Take a snapshot of the synced projects here, upload it, and look in the cloud">
            <RefreshCw size={13} className={busy ? 'spin' : ''} /> Sync now
          </button>
        </div>
      )}

      <div className="proj-list">
        {info.rows.length === 0 && info.drive && (
          <div className="proj-empty">
            <FolderGit2 size={28} />
            <div>No projects yet. Start your AI tool in a project, or add a folder below.</div>
          </div>
        )}
        {info.rows.map((r) => {
          const st = STATUS[r.status] ?? STATUS.off
          return (
            <div key={r.hash} className={'proj-row ' + r.status}>
              <div className="proj-main">
                <div className="proj-name">
                  {r.name}
                  <span className={'proj-chip ' + r.status} title={st.hint}>{st.label}</span>
                </div>
                <div className="proj-sub">
                  {r.here ? r.root : r.root ? `${r.root} (not found)` : 'Not on this device'}
                  {r.cloud && (
                    <>
                      {' · '}cloud copy from {r.cloud.deviceName}, {relativeTime(r.cloud.ts)} · {r.cloud.branch} · {size(r.cloud.bytes)}
                    </>
                  )}
                </div>
                {restoring === r.hash && (
                  <div className="proj-restore">
                    <input className="input" value={dest} onChange={(e) => setDest(e.target.value)} placeholder="Folder for the project" spellCheck={false} autoFocus />
                    <button className="btn primary sm" disabled={busy || !dest.trim()} onClick={() => restore(r)}>
                      {busy ? <Spinner /> : 'Restore here'}
                    </button>
                    <button className="icon-btn" onClick={() => setRestoring(null)} title="Cancel"><X size={14} /></button>
                  </div>
                )}
              </div>
              <div className="proj-actions">
                {r.status === 'behind' && (
                  <button className="btn sm primary" disabled={busy} onClick={() => run(() => api.projectsUpdate(r.hash), `Updated ${r.name} from the cloud`)}>
                    <CloudDownload size={13} /> Update from cloud
                  </button>
                )}
                {r.cloud && !r.here && restoring !== r.hash && (
                  <button className="btn sm primary" onClick={() => (setDest(r.suggest), setRestoring(r.hash))}>
                    <CloudDownload size={13} /> Restore…
                  </button>
                )}
                {r.here && (
                  <label className="proj-toggle" title="Keep this project's snapshot in your cloud drive">
                    <input type="checkbox" checked={r.sync} disabled={busy || !info.drive} onChange={(e) => run(() => api.projectsSetSync(r.hash, e.target.checked))} />
                    <span>Sync</span>
                  </label>
                )}
                {!r.cloud && (
                  <button
                    className="icon-btn"
                    title="Forget this project on this device"
                    onClick={async () => (await confirm.ask(`Forget “${r.name}” on this device? Its files aren't touched.`, 'Forget')) && run(() => api.projectsForget(r.hash))}
                  >
                    <X size={14} />
                  </button>
                )}
              </div>
            </div>
          )
        })}
      </div>

      {(
        <form
          className="proj-add"
          onSubmit={(e) => {
            e.preventDefault()
            if (adding.trim()) run(() => api.projectsAdd(adding), 'Project added').then(() => setAdding(''))
          }}
        >
          <input className="input" value={adding} onChange={(e) => setAdding(e.target.value)} placeholder="Add a project folder, e.g. ~/Projects/my-app" spellCheck={false} />
          <button className="btn sm" type="submit" disabled={busy || !adding.trim()}>
            <FolderPlus size={13} /> Add
          </button>
        </form>
      )}
    </div>
  )
}
