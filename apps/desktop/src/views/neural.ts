// The Brain as a 3D neural network shaped like a brain. Memories are neurons inside the brain's
// volume (each project in its own lobe, across both hemispheres), related memories are wired by
// synapses, and signals fire along them. The brain's surface is a cloud of tiny 3D points with a
// folded pattern — it turns with you; there's no flat image. Plain canvas 2D with our own 3D
// projection (no WebGL needed).

import type { BrainGraph, Memory, MemoryKind } from '../api'

export interface NeuralHandle {
  destroy(): void
  setQuery(q: string): void
  setSurface(on: boolean): void
  fit(): void
}

const TAU = Math.PI * 2
const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v))

/** Neuron colours by memory type (also used by the legend). */
export const KIND_HUE: Record<MemoryKind, [number, number, number]> = {
  decision: [262, 90, 72],
  gotcha: [28, 95, 62],
  howto: [188, 90, 60],
  preference: [328, 85, 70],
  fact: [214, 90, 70],
  context: [150, 70, 60],
  summary: [240, 15, 72],
  codemap: [240, 15, 72],
}
const kindColor = (k: MemoryKind, a = 1) => {
  const [h, s, l] = KIND_HUE[k]
  return `hsla(${h}, ${s}%, ${l}%, ${a})`
}

function rng(seed: string) {
  let h = 2166136261
  for (const c of seed) h = Math.imul(h ^ c.charCodeAt(0), 16777619)
  return () => {
    h += 0x6d2b79f5
    let t = h
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
const hashHue = (s: string) => {
  let h = 0
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360
  return h
}

function canvas(w: number, h: number) {
  const c = document.createElement('canvas')
  c.width = Math.max(1, Math.ceil(w))
  c.height = Math.max(1, Math.ceil(h))
  return c
}

// ---- the brain's volume -----------------------------------------------------------------
// x: front (−) to back (+), y: down (−) to up (+), z: left/right. Roughly a unit brain.

type V3 = [number, number, number]
interface Ellipsoid {
  c: V3
  r: V3
}

function hemisphereParts(s: number): Ellipsoid[] {
  return [
    // The cerebrum's main mass, one per hemisphere. They overlap, so from the front the top is
    // one dome with a gentle dip in the middle, not two separate lobes.
    { c: [0.02, 0.06, s * 0.2], r: [0.86, 0.6, 0.37] },
    // Frontal lobe, a little lower and narrower.
    { c: [-0.48, -0.04, s * 0.18], r: [0.42, 0.44, 0.33] },
    // Temporal lobe, low on the side.
    { c: [-0.08, -0.3, s * 0.3], r: [0.5, 0.22, 0.22] },
    // Occipital lobe at the back.
    { c: [0.58, -0.02, s * 0.18], r: [0.36, 0.42, 0.3] },
  ]
}
const CEREBRUM: Ellipsoid[] = [...hemisphereParts(1), ...hemisphereParts(-1)]
const CEREBELLUM: Ellipsoid[] = [
  { c: [0.56, -0.44, 0.2], r: [0.3, 0.2, 0.24] },
  { c: [0.56, -0.44, -0.2], r: [0.3, 0.2, 0.24] },
]
const STEM: Ellipsoid[] = [{ c: [0.3, -0.68, 0], r: [0.12, 0.34, 0.12] }]
const PARTS = [...CEREBRUM, ...CEREBELLUM, ...STEM]

const inside = (p: V3, parts: Ellipsoid[], shrink = 1) =>
  parts.some(({ c, r }) => {
    const dx = (p[0] - c[0]) / (r[0] * shrink)
    const dy = (p[1] - c[1]) / (r[1] * shrink)
    const dz = (p[2] - c[2]) / (r[2] * shrink)
    return dx * dx + dy * dy + dz * dz <= 1
  })
// A thin, shallow groove along the very top where the hemispheres meet.
const inBrain = (p: V3, shrink = 1) => inside(p, PARTS, shrink) && !(Math.abs(p[2]) < 0.012 && p[1] > 0.42 && inside(p, CEREBRUM, shrink))

/** Brain-fold pattern: squiggly bands, like gyri. */
const folds = (p: V3) => {
  const [x, y, z] = p
  return Math.sin(11 * x + 3.2 * Math.sin(7 * y + 2 * z)) * Math.sin(10 * y + 3 * Math.sin(8 * z - 3 * x)) + 0.6 * Math.sin(9 * z + 2.4 * Math.sin(9 * x))
}

/** The outward direction of the brain's surface at `p` (from the part it's on). */
function normalAt(p: V3): V3 {
  let best: Ellipsoid = PARTS[0]
  let bestV = -1
  for (const e of PARTS) {
    const v = ((p[0] - e.c[0]) / e.r[0]) ** 2 + ((p[1] - e.c[1]) / e.r[1]) ** 2 + ((p[2] - e.c[2]) / e.r[2]) ** 2
    if (v <= 1.0001 && v > bestV) {
      bestV = v
      best = e
    }
  }
  const n: V3 = [(p[0] - best.c[0]) / best.r[0] ** 2, (p[1] - best.c[1]) / best.r[1] ** 2, (p[2] - best.c[2]) / best.r[2] ** 2]
  const len = Math.hypot(...n) || 1
  return [n[0] / len, n[1] / len, n[2] / len]
}

/** Points on the brain's surface (with their normals), brighter along the folds. */
function surfacePoints(count: number): { p: V3; n: V3; a: number }[] {
  const r = rng('astralis-brain-surface')
  const out: { p: V3; n: V3; a: number }[] = []
  let guard = 0
  while (out.length < count && guard++ < count * 400) {
    const p: V3 = [-1.05 + r() * 2.1, -1.05 + r() * 1.75, -0.75 + r() * 1.5]
    if (!inBrain(p) || inBrain(p, 0.94)) continue
    const f = Math.abs(folds(p))
    const inCerebellum = inside(p, CEREBELLUM) && !inside(p, CEREBRUM)
    // A solid shell, brighter along the folds — cerebellum: fine parallel folds; cerebrum:
    // meandering gyri.
    const band = inCerebellum ? Math.abs(Math.sin(p[1] * 60 + p[0] * 8)) : f
    const onFold = band < (inCerebellum ? 0.4 : 0.3)
    out.push({ p, n: normalAt(p), a: onFold ? 0.75 + r() * 0.25 : 0.22 + r() * 0.25 })
  }
  return out
}

// Lobes projects live in (mirrored across hemispheres); "Me" sits deep in the middle.
// In an order that spreads the first few projects across the brain.
const LOBES: V3[] = [
  [-0.55, 0.05, 0.24], // frontal
  [0.6, 0.02, 0.22], // occipital
  [0.12, 0.4, 0.22], // parietal
  [-0.08, -0.26, 0.3], // temporal
  [0.56, -0.44, 0.18], // cerebellum
  [-0.25, 0.38, 0.2],
  [0.35, 0.3, 0.24],
  [-0.4, -0.15, 0.24],
  [0.3, -0.18, 0.26],
]
const ME_LOBE: V3 = [0.02, 0.04, 0.12]

interface Neuron extends Memory {
  p: V3
  r: number
  flash: number
  sx: number
  sy: number
  depth: number
  scale: number
  weight: number
}

interface Region {
  name: string
  c: V3
  w: number
  hue: number
}

export function createNeural(el: HTMLCanvasElement, graph: BrainGraph, onOpen: (id: string) => void): NeuralHandle {
  const ctx = el.getContext('2d')!
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches

  // ---- regions ----
  const counts = new Map<string, number>()
  for (const m of graph.nodes) counts.set(m.project, (counts.get(m.project) ?? 0) + 1)
  const regions: Region[] = [...counts.entries()]
    .filter(([n]) => n !== 'Me')
    .sort((a, b) => b[1] - a[1])
    .map(([name, n], i) => ({ name, c: LOBES[i % LOBES.length], w: Math.sqrt(n + 2), hue: hashHue(name) }))
  if (counts.has('Me')) regions.push({ name: 'Me', c: ME_LOBE, w: Math.sqrt((counts.get('Me') ?? 0) + 2) * 0.7, hue: 300 })
  /** Nearest lobe (mirror-symmetric: either hemisphere). */
  const regionOf = (p: V3) => {
    let best = regions[0]
    let bd = Infinity
    for (const g of regions) {
      const d = Math.hypot(p[0] - g.c[0], p[1] - g.c[1], Math.abs(p[2]) - g.c[2]) / g.w
      if (d < bd) {
        bd = d
        best = g
      }
    }
    return best
  }

  // ---- neurons: spread through their lobe, in both hemispheres ----
  const neurons: Neuron[] = []
  const byId = new Map<string, Neuron>()
  const ordered = [...graph.nodes].sort((a, b) => b.recallCount - a.recallCount || Number(b.pinned) - Number(a.pinned))
  for (const m of ordered) {
    const r = rng(m.id)
    const region = regions.find((g) => g.name === m.project)
    const side = m.project === 'Me' ? (r() < 0.5 ? -1 : 1) : r() < 0.5 ? -1 : 1
    let best: V3 = region ? [region.c[0], region.c[1], side * region.c[2]] : [0, 0, 0]
    let bestScore = -Infinity
    for (let k = 0; k < 50; k++) {
      const spread = 0.12 + k * 0.012
      const c = region?.c ?? [0, 0, 0]
      const p: V3 = [c[0] + (r() - 0.5) * 2 * spread, c[1] + (r() - 0.5) * 2 * spread, side * c[2] + (r() - 0.5) * 2 * spread * 0.8]
      if (!inBrain(p, 0.9) || (region && regionOf(p) !== region)) continue
      const nearest = neurons.reduce((d, o) => Math.min(d, Math.hypot(o.p[0] - p[0], o.p[1] - p[1], o.p[2] - p[2])), 9)
      const score = Math.min(nearest, 0.14) - spread * 0.05
      if (score > bestScore) {
        bestScore = score
        best = p
      }
    }
    const n: Neuron = {
      ...m,
      p: best,
      r: 2.6 + Math.sqrt(m.recallCount) * 1.1 + (m.pinned ? 1 : 0),
      flash: 0,
      sx: 0,
      sy: 0,
      depth: 0,
      scale: 1,
      weight: 1 + m.recallCount + (m.recalledAt && Date.now() - m.recalledAt < 3 * 86400000 ? 6 : 0),
    }
    neurons.push(n)
    byId.set(n.id, n)
  }

  // ---- wiring ----
  const synapses = graph.links.map((l) => ({ a: byId.get(l.a)!, b: byId.get(l.b)!, s: l.strength })).filter((l) => l.a && l.b)
  const dist3 = (a: V3, b: V3) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
  const dendrites: [Neuron, Neuron][] = []
  for (const n of neurons) {
    const near = neurons
      .filter((o) => o !== n)
      .map((o) => [o, dist3(o.p, n.p)] as const)
      .sort((a, b) => a[1] - b[1])
      .slice(0, 2)
    for (const [o] of near) if (!dendrites.some(([p, q]) => (p === o && q === n) || (p === n && q === o))) dendrites.push([n, o])
  }
  const wiresOf = new Map<Neuron, { to: Neuron; synapse: boolean }[]>()
  const addWire = (a: Neuron, b: Neuron, synapse: boolean) => wiresOf.set(a, [...(wiresOf.get(a) ?? []), { to: b, synapse }])
  for (const s of synapses) {
    addWire(s.a, s.b, true)
    addWire(s.b, s.a, true)
  }
  for (const [a, b] of dendrites) {
    addWire(a, b, false)
    addWire(b, a, false)
  }

  // ---- the surface: tiny 3D points ----
  const surface = surfacePoints(9000)
  let showSurface = true

  const sprites = new Map<string, HTMLCanvasElement>()
  const sprite = (color: string) => {
    let s = sprites.get(color)
    if (s) return s
    s = canvas(64, 64)
    const x = s.getContext('2d')!
    const g = x.createRadialGradient(32, 32, 0, 32, 32, 32)
    g.addColorStop(0, 'rgba(255,255,255,1)')
    g.addColorStop(0.14, 'rgba(255,255,255,0.9)')
    g.addColorStop(0.26, color)
    g.addColorStop(0.55, color.replace(/[\d.]+\)$/, '0.16)'))
    g.addColorStop(1, 'rgba(0,0,0,0)')
    x.fillStyle = g
    x.fillRect(0, 0, 64, 64)
    sprites.set(color, s)
    return s
  }

  // ---- camera: orbit around the brain ----
  let w = 0
  let h = 0
  let dpr = 1
  const view = { yaw: -0.65, pitch: 0.28, zoom: 1 }
  const goal = { ...view }
  let idleSince = performance.now()
  const CENTER: V3 = [0.05, -0.12, 0]
  const DIST = 3.2

  /** World → screen: rotate (yaw about y, then pitch about x), then perspective. */
  let cy = 0
  let sy = 0
  let cp = 0
  let sp = 0
  let focal = 1
  const project = (p: V3) => {
    const x0 = p[0] - CENTER[0]
    const y0 = p[1] - CENTER[1]
    const z0 = p[2] - CENTER[2]
    const x1 = x0 * cy + z0 * sy
    const z1 = -x0 * sy + z0 * cy
    const y2 = y0 * cp - z1 * sp
    const z2 = y0 * sp + z1 * cp
    const d = DIST + z2
    const f = focal / d
    // A little below centre: the search bar sits over the top of the view.
    return { x: w / 2 + x1 * f, y: h / 2 + 22 - y2 * f, depth: z2, scale: f / (focal / DIST) }
  }

  let query = ''
  let hover: Neuron | null = null
  let raf = 0
  let last = performance.now()
  let nextFire = 0
  const pulses: { from: Neuron; to: Neuron; t: number; speed: number; depth: number; synapse: boolean }[] = []
  const fire = (n: Neuron, depth: number) => {
    n.flash = 1
    if (pulses.length > 140) return
    for (const wire of wiresOf.get(n) ?? []) {
      if (!wire.synapse && Math.random() > 0.35) continue
      pulses.push({ from: n, to: wire.to, t: 0, speed: 0.7 + Math.random() * 0.6, depth, synapse: wire.synapse })
    }
  }
  const totalWeight = neurons.reduce((s, n) => s + n.weight, 0)
  const pick = () => {
    let r = Math.random() * totalWeight
    for (const n of neurons) {
      r -= n.weight
      if (r <= 0) return n
    }
    return neurons[0]
  }

  function frame(now: number) {
    raf = requestAnimationFrame(frame)
    if (document.hidden) return
    const dt = Math.min(0.05, (now - last) / 1000)
    last = now
    // Turn slowly on its own when left alone.
    if (!reduced && !drag && now - idleSince > 2500) goal.yaw += dt * 0.12
    const f = reduced ? 1 : 1 - Math.exp(-dt * 12)
    view.yaw += (goal.yaw - view.yaw) * f
    view.pitch += (goal.pitch - view.pitch) * f
    view.zoom = Math.exp(Math.log(view.zoom) + (Math.log(goal.zoom) - Math.log(view.zoom)) * f)

    if (!reduced && neurons.length && now > nextFire) {
      fire(pick(), 0)
      nextFire = now + 380 + Math.random() * 520
    }
    for (let i = pulses.length - 1; i >= 0; i--) {
      const p = pulses[i]
      p.t += dt * p.speed
      if (p.t >= 1) {
        pulses.splice(i, 1)
        p.to.flash = Math.max(p.to.flash, p.synapse ? 0.9 : 0.5)
        if (p.depth < 3 && Math.random() < (p.synapse ? 0.5 : 0.25)) fire(p.to, p.depth + 1)
      }
    }
    for (const n of neurons) n.flash = Math.max(0, n.flash - dt * 1.6)
    paint()
  }

  // Depth → brightness: the far side fades like it's behind a veil.
  const fade = (depth: number) => clamp(0.95 - depth * 0.55, 0.12, 1)

  function paint() {
    cy = Math.cos(view.yaw)
    sy = Math.sin(view.yaw)
    cp = Math.cos(view.pitch)
    sp = Math.sin(view.pitch)
    focal = Math.min(w, h - 110) * 1.95 * view.zoom
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.globalCompositeOperation = 'source-over'
    ctx.globalAlpha = 1
    ctx.fillStyle = bg
    ctx.fillRect(0, 0, w, h)

    // Brain surface: tiny points, batched by brightness.
    ctx.globalCompositeOperation = 'lighter'
    if (showSurface) {
      const buckets: Path2D[] = [new Path2D(), new Path2D(), new Path2D(), new Path2D()]
      for (const s of surface) {
        const q = project(s.p)
        // Rim light: points whose surface faces sideways to you trace the outline; points
        // facing away dim.
        const nz1 = -s.n[0] * sy + s.n[2] * cy
        const nz2 = s.n[1] * sp + nz1 * cp
        const rim = 1 - Math.abs(nz2)
        const facing = nz2 > 0 ? 0.45 : 1 // away from you
        const b = (0.3 + 1.6 * rim * rim) * facing * (0.45 + s.a * 0.75) * fade(q.depth * 0.5)
        const i = b > 0.7 ? 3 : b > 0.45 ? 2 : b > 0.25 ? 1 : 0
        const size = 1.6 * q.scale * Math.sqrt(view.zoom)
        buckets[i].rect(q.x, q.y, size, size)
      }
      const alphas = [0.07, 0.17, 0.33, 0.6]
      buckets.forEach((b, i) => {
        ctx.fillStyle = `rgba(165, 155, 255, ${alphas[i]})`
        ctx.fill(b)
      })
    }

    for (const n of neurons) {
      const q = project(n.p)
      n.sx = q.x
      n.sy = q.y
      n.depth = q.depth
      n.scale = q.scale
    }
    const query_ = query.trim().toLowerCase()
    const match = (n: Neuron) => !query_ || `${n.hook} ${n.content} ${n.project}`.toLowerCase().includes(query_)
    const lit = new Set<Neuron>()
    if (hover) {
      lit.add(hover)
      for (const wire of wiresOf.get(hover) ?? []) if (wire.synapse) lit.add(wire.to)
    }
    const dimAll = !!hover || !!query_

    // Dendrites (faint, batched) and synapses.
    const dpath = new Path2D()
    for (const [a, b] of dendrites) {
      dpath.moveTo(a.sx, a.sy)
      dpath.lineTo(b.sx, b.sy)
    }
    ctx.strokeStyle = `rgba(150, 140, 255, ${dimAll ? 0.03 : 0.07})`
    ctx.lineWidth = 1
    ctx.stroke(dpath)
    for (const s of synapses) {
      const on = hover && (s.a === hover || s.b === hover)
      const depthA = fade((s.a.depth + s.b.depth) / 2)
      const a = (on ? 0.8 : dimAll ? 0.04 : 0.1 + s.s * 0.22) * (on ? 1 : depthA)
      ctx.strokeStyle = on ? kindColor(hover!.kind, a) : `rgba(175, 165, 255, ${a})`
      ctx.lineWidth = on ? 1.6 : 1
      ctx.beginPath()
      ctx.moveTo(s.a.sx, s.a.sy)
      ctx.lineTo(s.b.sx, s.b.sy)
      ctx.stroke()
    }
    // Signals.
    for (const p of pulses) {
      const x = p.from.sx + (p.to.sx - p.from.sx) * p.t
      const y = p.from.sy + (p.to.sy - p.from.sy) * p.t
      const bt = Math.max(0, p.t - 0.14)
      const bx = p.from.sx + (p.to.sx - p.from.sx) * bt
      const by = p.from.sy + (p.to.sy - p.from.sy) * bt
      const depthA = fade(p.from.depth + (p.to.depth - p.from.depth) * p.t)
      const g = ctx.createLinearGradient(bx, by, x, y)
      g.addColorStop(0, 'rgba(255,255,255,0)')
      g.addColorStop(1, kindColor(p.from.kind, (p.synapse ? 0.85 : 0.45) * depthA))
      ctx.strokeStyle = g
      ctx.lineWidth = p.synapse ? 2 : 1.2
      ctx.beginPath()
      ctx.moveTo(bx, by)
      ctx.lineTo(x, y)
      ctx.stroke()
      const sz = (p.synapse ? 9 : 6) * clamp(view.zoom, 0.7, 1.6)
      ctx.globalAlpha = depthA
      ctx.drawImage(sprite(kindColor(p.from.kind, 0.9)), x - sz / 2, y - sz / 2, sz, sz)
      ctx.globalAlpha = 1
    }
    // Neurons, far ones first.
    const sorted = [...neurons].sort((a, b) => b.depth - a.depth)
    for (const n of sorted) {
      const on = lit.has(n) || (query_ && match(n))
      const dim = dimAll && !on
      const rad = (n.r * 3.3 + n.flash * 6) * n.scale * clamp(view.zoom, 0.7, 1.8) * (n === hover ? 1.5 : 1)
      ctx.globalAlpha = (dim ? 0.18 : 0.6 + n.flash * 0.4) * (on ? 1 : fade(n.depth))
      ctx.drawImage(sprite(kindColor(n.kind, 0.95)), n.sx - rad, n.sy - rad, rad * 2, rad * 2)
      if (n.new && !dim) {
        ctx.globalAlpha = 0.6 * fade(n.depth)
        ctx.strokeStyle = kindColor(n.kind, 0.8)
        ctx.lineWidth = 1
        ctx.setLineDash([2, 3])
        ctx.beginPath()
        ctx.arc(n.sx, n.sy, rad * 0.55 + 3, 0, TAU)
        ctx.stroke()
        ctx.setLineDash([])
      }
    }
    ctx.globalAlpha = 1
    ctx.globalCompositeOperation = 'source-over'

    // Lobe names, on the side facing you.
    ctx.textAlign = 'center'
    ctx.font = '600 11px Inter, system-ui, sans-serif'
    ctx.lineJoin = 'round'
    for (const g of regions) {
      const sides = g.name === 'Me' ? [0] : [1, -1]
      const best = sides.map((s) => project([g.c[0], g.c[1] + 0.2, s * g.c[2]])).sort((a, b) => a.depth - b.depth)[0]
      const label = (g.name === 'Me' ? 'ME · EVERYWHERE' : g.name.toUpperCase()).split('').join(' ')
      ctx.globalAlpha = clamp(fade(best.depth) * 1.1, 0.25, 0.9)
      ctx.lineWidth = 4
      ctx.strokeStyle = 'rgba(4,3,14,0.85)'
      ctx.strokeText(label, best.x, best.y)
      ctx.fillStyle = `hsl(${g.hue} 70% 82%)`
      ctx.fillText(label, best.x, best.y)
    }
    ctx.globalAlpha = 1
    // Hover / search labels.
    ctx.font = '500 12px Inter, system-ui, sans-serif'
    const labelled = neurons.filter((n) => n === hover || (hover && lit.has(n)) || (query_ && match(n) && neurons.length < 400))
    for (const n of labelled) {
      const text = n.hook.length > 70 ? n.hook.slice(0, 69) + '…' : n.hook
      const y = n.sy - n.r * 3 * n.scale - 10
      ctx.lineWidth = 4
      ctx.strokeStyle = 'rgba(4,3,14,0.9)'
      ctx.strokeText(text, n.sx, y)
      ctx.fillStyle = n === hover ? '#ffffff' : 'rgba(225,222,250,0.85)'
      ctx.fillText(text, n.sx, y)
    }
  }

  // ---- input: drag or scroll to turn, Ctrl + scroll / pinch to zoom ----
  let bg: CanvasGradient | string = '#05040d'
  const pos = (e: MouseEvent) => {
    const r = el.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }
  const at = (p: { x: number; y: number }) => {
    let best: Neuron | null = null
    let bd = Infinity
    for (const n of neurons) {
      const d = Math.hypot(n.sx - p.x, n.sy - p.y) + n.depth * 6 // prefer the front
      if (Math.hypot(n.sx - p.x, n.sy - p.y) < Math.max(10, n.r * 2.5 * n.scale) && d < bd) {
        best = n
        bd = d
      }
    }
    return best
  }
  const turn = (dx: number, dy: number) => {
    goal.yaw += dx * 0.008
    goal.pitch = clamp(goal.pitch + dy * 0.006, -1.2, 1.2)
    idleSince = performance.now()
  }
  let drag: { x: number; y: number; moved: boolean } | null = null
  const onDown = (e: MouseEvent) => {
    if (e.button !== 0) return
    const p = pos(e)
    drag = { x: p.x, y: p.y, moved: false }
    idleSince = performance.now()
    el.classList.add('dragging')
  }
  const onMove = (e: MouseEvent) => {
    const p = pos(e)
    if (drag) {
      const dx = p.x - drag.x
      const dy = p.y - drag.y
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true
      drag.x = p.x
      drag.y = p.y
      turn(dx, dy)
      // Turn directly while dragging (no lag).
      view.yaw = goal.yaw
      view.pitch = goal.pitch
      return
    }
    if (e.target !== el) return
    const n = at(p)
    if (n !== hover) {
      hover = n
      if (n) n.flash = 1
      el.style.cursor = n ? 'pointer' : ''
    }
  }
  const onUp = (e: MouseEvent) => {
    if (drag && !drag.moved) {
      const n = at(pos(e))
      if (n) onOpen(n.id)
    }
    drag = null
    el.classList.remove('dragging')
  }
  const onWheel = (e: WheelEvent) => {
    e.preventDefault()
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1
    if (e.ctrlKey || e.metaKey) {
      goal.zoom = clamp(goal.zoom * Math.exp(-clamp(e.deltaY * unit, -32, 32) * 0.0045), 0.5, 4)
      idleSince = performance.now()
      return
    }
    turn(-e.deltaX * unit * 0.6, -e.deltaY * unit * 0.6)
  }
  let gestureScale = 1
  const onGestureStart = (e: Event) => {
    e.preventDefault()
    gestureScale = 1
  }
  const onGestureChange = (e: Event) => {
    e.preventDefault()
    const g = e as Event & { scale: number }
    goal.zoom = clamp(goal.zoom * (g.scale / gestureScale), 0.5, 4)
    gestureScale = g.scale
  }

  const resize = () => {
    dpr = Math.min(window.devicePixelRatio || 1, 1.5)
    w = el.clientWidth
    h = el.clientHeight
    el.width = Math.round(w * dpr)
    el.height = Math.round(h * dpr)
    const g = ctx.createRadialGradient(w / 2, h / 2, 0, w / 2, h / 2, Math.max(w, h) * 0.75)
    g.addColorStop(0, '#0d0a22')
    g.addColorStop(1, '#03020a')
    bg = g
  }
  const ro = new ResizeObserver(resize)
  ro.observe(el)
  resize()
  el.addEventListener('mousedown', onDown)
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
  el.addEventListener('wheel', onWheel, { passive: false })
  el.addEventListener('gesturestart', onGestureStart)
  el.addEventListener('gesturechange', onGestureChange)
  raf = requestAnimationFrame(frame)

  return {
    destroy() {
      cancelAnimationFrame(raf)
      ro.disconnect()
      el.removeEventListener('mousedown', onDown)
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      el.removeEventListener('wheel', onWheel)
      el.removeEventListener('gesturestart', onGestureStart)
      el.removeEventListener('gesturechange', onGestureChange)
    },
    setQuery(q) {
      query = q
    },
    setSurface(on) {
      showSurface = on
    },
    fit() {
      Object.assign(goal, { yaw: -0.65, pitch: 0.28, zoom: 1 })
      idleSince = performance.now()
    },
  }
}
