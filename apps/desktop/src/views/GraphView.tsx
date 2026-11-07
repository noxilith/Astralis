import { useEffect, useRef, useState } from 'react'
import { forceCenter, forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, type SimulationLinkDatum, type SimulationNodeDatum } from 'd3-force'
import { Network } from 'lucide-react'
import { api, type GraphNode } from '../api'
import { useApp } from '../context'
import { Empty } from '../components/ui'

interface Node extends SimulationNodeDatum, GraphNode {}
type Link = SimulationLinkDatum<Node>

const hue = (s: string) => {
  let h = 0
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360
  return h
}

/** Force-directed graph of linked notes. Drag to pan, scroll to zoom, click a node to open it. */
export function GraphView() {
  const { openNote, version, fail, vaultId } = useApp()
  const canvas = useRef<HTMLCanvasElement>(null)
  const [orphans, setOrphans] = useState(true)
  const [query, setQuery] = useState('')
  const [count, setCount] = useState<number | null>(null)
  const queryRef = useRef(query)
  queryRef.current = query

  useEffect(() => {
    let disposed = false
    let cleanup = () => {}
    api
      .graph(vaultId, orphans)
      .then((g) => {
        if (disposed || !canvas.current) return
        setCount(g.nodes.length)
        cleanup = draw(canvas.current, g.nodes, g.edges, openNote, () => queryRef.current)
      })
      .catch(fail)
    return () => {
      disposed = true
      cleanup()
    }
  }, [orphans, version, openNote, fail, vaultId])

  return (
    <div className="graph-wrap">
      <canvas ref={canvas} className="graph-canvas" />
      <div className="graph-controls">
        <input className="input" style={{ width: 180, padding: '4px 8px' }} placeholder="Highlight…" value={query} onChange={(e) => setQuery(e.target.value)} />
        <label className="checkbox">
          <input type="checkbox" checked={orphans} onChange={(e) => setOrphans(e.target.checked)} /> Show unlinked notes
        </label>
        {count != null && <span className="small faint">{count} notes</span>}
      </div>
      {count === 0 && (
        <div style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}>
          <Empty icon={<Network size={32} />} title={orphans ? "No notes yet" : "No linked notes yet"}>
            <span className="small">Link notes with [[Note title]] to grow your graph.</span>
          </Empty>
        </div>
      )}
    </div>
  )
}

function draw(
  el: HTMLCanvasElement,
  rawNodes: GraphNode[],
  edges: { source: string; target: string }[],
  onOpen: (id: string) => void,
  getQuery: () => string,
) {
  const ctx = el.getContext('2d')!
  const nodes: Node[] = rawNodes.map((n) => ({ ...n }))
  const links: Link[] = edges.map((e) => ({ source: e.source, target: e.target }))
  const neighbours = new Map<string, Set<string>>()
  for (const e of edges) {
    if (!neighbours.has(e.source)) neighbours.set(e.source, new Set())
    if (!neighbours.has(e.target)) neighbours.set(e.target, new Set())
    neighbours.get(e.source)!.add(e.target)
    neighbours.get(e.target)!.add(e.source)
  }
  const radius = (n: Node) => 4 + Math.sqrt(n.degree) * 2.5
  let view = { x: 0, y: 0, k: 1 }
  let hover: Node | null = null
  let frame = 0
  const styles = getComputedStyle(document.documentElement)
  const color = (v: string) => styles.getPropertyValue(v).trim()

  const resize = () => {
    const dpr = window.devicePixelRatio || 1
    el.width = el.clientWidth * dpr
    el.height = el.clientHeight * dpr
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    render()
  }

  const sim = forceSimulation(nodes)
    .force('link', forceLink<Node, Link>(links).id((d) => d.id).distance(60).strength(0.6))
    .force('charge', forceManyBody().strength(-160))
    .force('center', forceCenter(0, 0))
    // Gentle gravity keeps unlinked notes near the cluster instead of drifting away.
    .force('x', forceX(0).strength(0.06))
    .force('y', forceY(0).strength(0.06))
    .force('collide', forceCollide<Node>().radius((d) => radius(d) + 4))
    .on('tick', () => render())

  // Simulation ticks are batched into animation frames; user input (zoom, pan, hover) repaints
  // right away so the view never lags behind the gesture.
  function render() {
    cancelAnimationFrame(frame)
    frame = requestAnimationFrame(paint)
  }
  function paintNow() {
    cancelAnimationFrame(frame)
    paint()
  }
  function paint() {
    {
      const accent = color("--accent")
      const edge = color("--border-strong")
      const textColor = color("--text")
      const labelColor = color("--text-muted")
      const w = el.clientWidth
      const h = el.clientHeight
      ctx.clearRect(0, 0, w, h)
      ctx.save()
      ctx.translate(w / 2 + view.x, h / 2 + view.y)
      ctx.scale(view.k, view.k)
      const q = getQuery().trim().toLowerCase()
      const focus = hover ? neighbours.get(hover.id) ?? new Set<string>() : null
      const dim = (n: Node) => (focus ? n !== hover && !focus.has(n.id) : q ? !n.title.toLowerCase().includes(q) : false)

      ctx.lineWidth = 1 / view.k
      for (const l of links) {
        const s = l.source as Node
        const t = l.target as Node
        const active = hover && (s === hover || t === hover)
        ctx.strokeStyle = active ? accent : edge
        ctx.globalAlpha = focus && !active ? 0.15 : 0.8
        ctx.beginPath()
        ctx.moveTo(s.x!, s.y!)
        ctx.lineTo(t.x!, t.y!)
        ctx.stroke()
      }
      for (const n of nodes) {
        ctx.globalAlpha = dim(n) ? 0.2 : 1
        ctx.fillStyle = n.tags.length ? `hsl(${hue(n.tags[0])} 70% 68%)` : accent
        ctx.beginPath()
        if (n.kind === 'whiteboard') ctx.rect(n.x! - radius(n), n.y! - radius(n), radius(n) * 2, radius(n) * 2)
        else ctx.arc(n.x!, n.y!, radius(n), 0, Math.PI * 2)
        ctx.fill()
        if (n.pinned) {
          ctx.strokeStyle = textColor
          ctx.stroke()
        }
        if (view.k > 0.7 || n === hover || (focus && focus.has(n.id))) {
          ctx.fillStyle = labelColor
          ctx.font = `${12 / Math.max(view.k, 1)}px system-ui`
          ctx.textAlign = 'center'
          ctx.fillText(n.title || 'Untitled', n.x!, n.y! + radius(n) + 12 / Math.max(view.k, 1))
        }
      }
      ctx.restore()
    }
  }

  const toWorld = (e: MouseEvent) => {
    const r = el.getBoundingClientRect()
    return { x: (e.clientX - r.left - r.width / 2 - view.x) / view.k, y: (e.clientY - r.top - r.height / 2 - view.y) / view.k }
  }
  const hit = (e: MouseEvent) => {
    const p = toWorld(e)
    return nodes.find((n) => Math.hypot(n.x! - p.x, n.y! - p.y) <= radius(n) + 3) ?? null
  }

  let drag: { node: Node | null; startX: number; startY: number; moved: boolean; vx: number; vy: number } | null = null
  const onDown = (e: MouseEvent) => {
    const node = hit(e)
    drag = { node, startX: e.clientX, startY: e.clientY, moved: false, vx: view.x, vy: view.y }
    if (node) {
      node.fx = node.x
      node.fy = node.y
      sim.alphaTarget(0.3).restart()
    }
    el.classList.add('dragging')
  }
  const onMove = (e: MouseEvent) => {
    if (!drag) {
      const h = hit(e)
      if (h !== hover) {
        hover = h
        el.style.cursor = h ? "pointer" : ""
        paintNow()
      }
      return
    }
    const dx = e.clientX - drag.startX
    const dy = e.clientY - drag.startY
    if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true
    if (drag.node) {
      const p = toWorld(e)
      drag.node.fx = p.x
      drag.node.fy = p.y
    } else {
      view = { ...view, x: drag.vx + dx, y: drag.vy + dy }
      paintNow()
    }
  }
  const onUp = () => {
    if (drag?.node) {
      if (!drag.moved) onOpen(drag.node.id)
      drag.node.fx = null
      drag.node.fy = null
      sim.alphaTarget(0)
    }
    drag = null
    el.classList.remove('dragging')
  }
  /** Zooms by `factor`, keeping the screen point (clientX, clientY) fixed. */
  const zoomAt = (factor: number, clientX: number, clientY: number) => {
    const k = Math.min(4, Math.max(0.2, view.k * factor))
    const r = el.getBoundingClientRect()
    const px = clientX - r.left - r.width / 2
    const py = clientY - r.top - r.height / 2
    const wx = (px - view.x) / view.k
    const wy = (py - view.y) / view.k
    view = { k, x: px - wx * k, y: py - wy * k }
    paintNow()
  }
  const onWheel = (e: WheelEvent) => {
    e.preventDefault()
    // Proportional to scroll distance (trackpads send many tiny events). Trackpad pinches
    // arrive as ctrl+wheel with small deltas, so they get a stronger factor.
    const delta = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaMode === 2 ? e.deltaY * 400 : e.deltaY
    zoomAt(Math.exp(-delta * (e.ctrlKey ? 0.01 : 0.0008)), e.clientX, e.clientY)
  }
  // WebKit pinch gestures (where supported) report a cumulative scale.
  let gestureScale = 1
  const onGestureStart = (e: Event) => {
    e.preventDefault()
    gestureScale = 1
  }
  const onGestureChange = (e: Event) => {
    e.preventDefault()
    const g = e as Event & { scale: number; clientX: number; clientY: number }
    zoomAt(g.scale / gestureScale, g.clientX, g.clientY)
    gestureScale = g.scale
  }
  const ro = new ResizeObserver(resize)
  ro.observe(el)
  el.addEventListener('mousedown', onDown)
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
  el.addEventListener('wheel', onWheel, { passive: false })
  el.addEventListener('gesturestart', onGestureStart)
  el.addEventListener('gesturechange', onGestureChange)
  const queryTimer = setInterval(render, 300)
  resize()

  return () => {
    sim.stop()
    ro.disconnect()
    clearInterval(queryTimer)
    cancelAnimationFrame(frame)
    el.removeEventListener('mousedown', onDown)
    window.removeEventListener('mousemove', onMove)
    window.removeEventListener('mouseup', onUp)
    el.removeEventListener('wheel', onWheel)
    el.removeEventListener('gesturestart', onGestureStart)
    el.removeEventListener('gesturechange', onGestureChange)
  }
}
