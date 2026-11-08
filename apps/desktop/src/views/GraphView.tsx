import { useEffect, useRef, useState } from 'react'
import { Orbit, Search, Telescope } from 'lucide-react'
import { api } from '../api'
import { useApp } from '../context'
import { createSpace, type GalaxyInput, type SpaceHandle } from './space'

/** Every vault as a 3D galaxy: linked notes form solar systems (the most-linked note is the sun),
 *  unlinked notes are lone stars. Drag to turn, Ctrl + scroll to zoom, right-drag to slide; click
 *  a galaxy or system to fly there, a planet or star to open it. */
export function GraphView() {
  const { openNote, version, fail, vaultId, vaults } = useApp()
  const canvas = useRef<HTMLCanvasElement>(null)
  const space = useRef<SpaceHandle | null>(null)
  const [orphans, setOrphans] = useState(true)
  const [query, setQuery] = useState('')
  const [galaxies, setGalaxies] = useState<{ id: string; name: string; count: number }[]>([])
  const [focused, setFocused] = useState<string | null>(vaultId)
  const queryRef = useRef(query)
  queryRef.current = query
  // Only rebuild the scene when what's in it changes (not on every vault-list refresh).
  const vaultKey = vaults.map((v) => `${v.id}:${v.name}:${v.encrypted && !v.unlocked}`).join('|')

  useEffect(() => {
    let disposed = false
    Promise.all(
      vaults.map(async (v): Promise<GalaxyInput> => {
        const locked = v.encrypted && !v.unlocked
        const g = locked ? { nodes: [], edges: [] } : await api.graph(v.id, orphans).catch(() => ({ nodes: [], edges: [] }))
        return { id: v.id, name: v.name, locked, encrypted: v.encrypted, current: v.id === vaultId, nodes: g.nodes, edges: g.edges }
      }),
    )
      .then((inputs) => {
        if (disposed || !canvas.current) return
        space.current?.destroy()
        space.current = createSpace(canvas.current, inputs, openNote, setFocused)
        space.current.setQuery(queryRef.current)
        setGalaxies(inputs.map((g) => ({ id: g.id, name: g.name, count: g.nodes.length })))
        setFocused(vaultId)
      })
      .catch(fail)
    return () => {
      disposed = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [orphans, version, vaultKey, openNote, fail])

  useEffect(() => () => space.current?.destroy(), [])
  // Dev only: `__astralisSpace.benchmark()` in the console times a frame.
  useEffect(() => {
    if (import.meta.env.DEV) (window as unknown as { __astralisSpace?: () => SpaceHandle | null }).__astralisSpace = () => space.current
  }, [])
  useEffect(() => space.current?.setQuery(query), [query])

  const total = galaxies.reduce((n, g) => n + g.count, 0)
  const go = (id: string | null) => {
    setFocused(id)
    space.current?.flyTo(id)
  }

  return (
    <div className="graph-wrap space">
      <canvas ref={canvas} className="graph-canvas" />
      <div className="space-hud top">
        <div className="space-search">
          <Search size={13} />
          <input placeholder="Find a star…" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <label className="space-toggle">
          <input type="checkbox" checked={orphans} onChange={(e) => setOrphans(e.target.checked)} /> Unlinked notes
        </label>
        <span className="space-count">
          {total} {total === 1 ? 'star' : 'stars'} · {galaxies.length} {galaxies.length === 1 ? 'galaxy' : 'galaxies'}
        </span>
      </div>
      <div className="space-hud bottom">
        <button className={'space-chip' + (focused === null ? ' active' : '')} onClick={() => go(null)} title="See every galaxy">
          <Telescope size={13} /> All galaxies
        </button>
        {galaxies.map((g) => (
          <button key={g.id} className={'space-chip' + (focused === g.id ? ' active' : '')} onClick={() => go(g.id)}>
            <Orbit size={13} /> {g.name}
          </button>
        ))}
      </div>
      <div className="space-hint">Drag to turn · Ctrl + scroll to zoom · Right-drag to slide · Click a system to fly in · Esc to go back</div>
      {total === 0 && galaxies.length > 0 && (
        <div className="space-empty">
          <b>Empty space, for now</b>
          <span>Every note you write becomes a star here; notes you [[link]] together form solar systems.</span>
        </div>
      )}
    </div>
  )
}
