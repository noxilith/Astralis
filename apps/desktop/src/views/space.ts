// The graph as a view into space: every vault is a galaxy made of its notes (stars laid out
// along spiral arms, so a big vault looks like a galaxy and a small one like a few stars), links
// are constellation lines. Plain canvas 2D: the heavy parts (nebulae, starfield, star glows) are
// painted once into offscreen sprites, then drawn each frame.

import { forceCollide, forceSimulation, forceX, forceY, type SimulationNodeDatum } from 'd3-force'
import type { GraphNode } from '../api'

export interface GalaxyInput {
  id: string
  name: string
  /** Locked encrypted vault: shown cloaked, without its stars. */
  locked: boolean
  encrypted: boolean
  current: boolean
  nodes: GraphNode[]
  edges: { source: string; target: string }[]
}

export interface SpaceHandle {
  destroy(): void
  /** Glides the camera to a galaxy (or to see all of them). */
  flyTo(galaxyId: string | null): void
  setQuery(q: string): void
  /** Average milliseconds to draw one frame (for checking performance). */
  benchmark(frames?: number, panning?: boolean): number
}

interface Star extends SimulationNodeDatum, GraphNode {
  galaxy: Galaxy
  /** Position in the galaxy's own frame (before its slow spin). */
  bx: number
  by: number
  /** On-screen position this frame (for hover). */
  sx: number
  sy: number
  r: number
  color: string
  phase: number
  twinkle: number
}

/** Half the on-screen height of a tilted disc (world units). */
const halfHeight = (g: { R: number; angle: number; tilt: number }) => g.R * (Math.abs(Math.sin(g.angle)) + g.tilt * Math.abs(Math.cos(g.angle)))

interface Galaxy {
  id: string
  name: string
  locked: boolean
  encrypted: boolean
  x: number
  y: number
  R: number
  hue: number
  sat: number
  /** Disc orientation and inclination (it's drawn as a tilted disc). */
  angle: number
  tilt: number
  spin: number
  stars: Star[]
  links: { s: Star; t: Star }[]
  count: number
}

// ---- small helpers ----------------------------------------------------------------------

const TAU = Math.PI * 2
const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v))
const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2)

/** Deterministic randomness per seed, so a vault always looks the same. */
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
const gauss = (r: () => number) => {
  let u = 0
  while (u === 0) u = r()
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(TAU * r())
}
const hashHue = (s: string) => {
  let h = 0
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360
  return h
}

/** Hues that read well against deep space. */
const GALAXY_HUES: [number, number][] = [
  [262, 85], // violet (Astralis)
  [190, 90], // cyan
  [318, 80], // magenta
  [32, 95], // amber
  [162, 70], // teal
  [222, 90], // blue
  [350, 85], // rose
  [285, 75], // purple
]

function canvas(w: number, h: number) {
  const c = document.createElement('canvas')
  c.width = Math.max(1, Math.ceil(w))
  c.height = Math.max(1, Math.ceil(h))
  return c
}

// ---- sprites ----------------------------------------------------------------------------

/** Soft coloured clouds behind everything. */
function paintNebula(seed: string) {
  const r = rng(seed)
  const S = 1024
  const c = canvas(S, S)
  const x = c.getContext('2d')!
  x.globalCompositeOperation = 'lighter'
  const hues = [262, 300, 200, 330, 230, 180]
  for (let i = 0; i < 26; i++) {
    const px = r() * S
    const py = r() * S
    const rad = 90 + r() * 320
    const h = hues[Math.floor(r() * hues.length)] + (r() - 0.5) * 20
    const a = 0.025 + r() * 0.05
    // Draw wrapped copies so the texture tiles seamlessly.
    for (const ox of [-S, 0, S]) {
      for (const oy of [-S, 0, S]) {
        const g = x.createRadialGradient(px + ox, py + oy, 0, px + ox, py + oy, rad)
        g.addColorStop(0, `hsla(${h}, 80%, 55%, ${a})`)
        g.addColorStop(0.55, `hsla(${h + 20}, 70%, 40%, ${a * 0.45})`)
        g.addColorStop(1, 'hsla(0,0%,0%,0)')
        x.fillStyle = g
        x.fillRect(px + ox - rad, py + oy - rad, rad * 2, rad * 2)
      }
    }
  }
  return c
}

const hazeCache = new Map<string, HTMLCanvasElement>()
/** A soft glow in a colour, with no white centre (for light pooling from afar). */
function haze(color: string) {
  let c = hazeCache.get(color)
  if (c) return c
  const S = 48
  c = canvas(S, S)
  const x = c.getContext('2d')!
  const g = x.createRadialGradient(S / 2, S / 2, 0, S / 2, S / 2, S / 2)
  g.addColorStop(0, color.replace(/[\d.]+\)$/, '0.55)'))
  g.addColorStop(0.45, color.replace(/[\d.]+\)$/, '0.18)'))
  g.addColorStop(1, 'rgba(0,0,0,0)')
  x.fillStyle = g
  x.fillRect(0, 0, S, S)
  hazeCache.set(color, c)
  return c
}

const glowCache = new Map<string, HTMLCanvasElement>()
/** A star's glow in a colour (white-hot centre, coloured halo). */
function glow(color: string) {
  let c = glowCache.get(color)
  if (c) return c
  const S = 64
  c = canvas(S, S)
  const x = c.getContext('2d')!
  const g = x.createRadialGradient(S / 2, S / 2, 0, S / 2, S / 2, S / 2)
  g.addColorStop(0, 'rgba(255,255,255,1)')
  g.addColorStop(0.12, 'rgba(255,255,255,0.95)')
  g.addColorStop(0.22, color)
  g.addColorStop(0.5, color.replace(/[\d.]+\)$/, '0.18)'))
  g.addColorStop(1, 'rgba(0,0,0,0)')
  x.fillStyle = g
  x.fillRect(0, 0, S, S)
  glowCache.set(color, c)
  return c
}

/** Centred text with extra space between letters, drawn letter by letter (no special space
 *  characters, which some fonts render as boxes). */
function spacedText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, spacing: number) {
  const chars = [...text]
  const widths = chars.map((ch) => ctx.measureText(ch).width)
  const total = widths.reduce((a, b) => a + b, 0) + spacing * Math.max(0, chars.length - 1)
  const align = ctx.textAlign
  ctx.textAlign = 'left'
  let cx = x - total / 2
  chars.forEach((ch, i) => {
    ctx.fillText(ch, cx, y)
    cx += widths[i] + spacing
  })
  ctx.textAlign = align
}

// ---- the scene --------------------------------------------------------------------------

export function createSpace(el: HTMLCanvasElement, inputs: GalaxyInput[], onOpen: (noteId: string) => void, onGalaxy?: (id: string | null) => void): SpaceHandle {
  const ctx = el.getContext('2d')!
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  let query = ''
  let w = 0
  let h = 0
  let dpr = 1

  // Galaxies on a golden-angle spiral, the current vault in the middle.
  const ordered = [...inputs].sort((a, b) => Number(b.current) - Number(a.current))
  const galaxies: Galaxy[] = []
  let placedR = 0
  ordered.forEach((g, i) => {
    const r = rng(g.id)
    const R = clamp(110 + Math.sqrt(g.nodes.length) * 58, 120, 1800)
    const [hue, sat] = GALAXY_HUES[Math.floor(r() * GALAXY_HUES.length)]
    const dist = i === 0 ? 0 : placedR + R + 360 * Math.sqrt(i)
    const a = i * 2.39996 + r() * 0.4
    const gal: Galaxy = {
      id: g.id,
      name: g.name,
      locked: g.locked,
      encrypted: g.encrypted,
      x: Math.cos(a) * dist,
      y: Math.sin(a) * dist * 0.8,
      R,
      hue,
      sat,
      // Close to edge-on horizontal: wide discs leave room for names.
      angle: (r() - 0.5) * 0.7,
      tilt: 0.52 + r() * 0.3,
      spin: (r() < 0.5 ? -1 : 1) * (0.012 + r() * 0.01),
      stars: [],
      links: [],
      count: g.nodes.length,
    }
    if (i === 0) placedR = R
    galaxies.push(gal)

    // Notes are the galaxy: hubs in the core, the rest along spiral arms, linked notes next to
    // each other on the same arm. Hundreds of notes trace a spiral; a few just look like stars.
    if (!g.locked) {
      const n = g.nodes.length
      const sizeScale = clamp(1.2 - n / 500, 0.55, 1.1)
      const stars: Star[] = g.nodes.map((nd) => {
        const tagHue = nd.tags.length ? hashHue(nd.tags[0]) : hue + (r() - 0.5) * 50
        return {
          ...nd,
          galaxy: gal,
          bx: 0,
          by: 0,
          sx: 0,
          sy: 0,
          r: (2.2 + Math.sqrt(nd.degree) * 1.6 + (nd.pinned ? 1.5 : 0)) * sizeScale,
          color: `hsla(${tagHue}, 90%, 72%, 0.9)`,
          phase: r() * TAU,
          twinkle: 0.6 + r() * 1.8,
        }
      })
      const byId = new Map(stars.map((st) => [st.id, st]))
      const links = g.edges.filter((e) => byId.has(e.source) && byId.has(e.target))
      const adj = new Map<string, string[]>()
      for (const e of links) {
        adj.set(e.source, [...(adj.get(e.source) ?? []), e.target])
        adj.set(e.target, [...(adj.get(e.target) ?? []), e.source])
      }
      // Hubs first, then breadth-first through links so neighbours stay close in the order.
      const order: Star[] = []
      const seen = new Set<string>()
      const byImportance = [...stars].sort((a, b) => b.degree - a.degree || Number(b.pinned) - Number(a.pinned) || a.title.localeCompare(b.title))
      for (const start of byImportance) {
        if (seen.has(start.id)) continue
        const queue = [start]
        seen.add(start.id)
        while (queue.length) {
          const cur = queue.shift()!
          order.push(cur)
          for (const next of adj.get(cur.id) ?? []) {
            if (!seen.has(next)) {
              seen.add(next)
              queue.push(byId.get(next)!)
            }
          }
        }
      }
      const arms = n < 40 ? 2 : 2 + Math.floor(r() * 3)
      const wind = 2.4 + r() * 1.8
      // Few notes: loosely scattered (no visible pattern). Many: tight, clear arms.
      const scatter = clamp(1.5 - n / 70, 0.45, 1.5)
      const core = n < 10 ? 0 : Math.max(1, Math.round(n * 0.18))
      const perArm = Math.max(1, Math.ceil((n - core) / arms))
      const target = new Map<Star, [number, number]>()
      order.forEach((st, i) => {
        let x: number, y: number
        if (i < core) {
          const d = R * (0.01 + Math.abs(gauss(r)) * 0.09)
          const a = r() * TAU
          x = Math.cos(a) * d
          y = Math.sin(a) * d
        } else {
          const j = i - core
          const arm = Math.floor(j / perArm)
          const t = ((j % perArm) + 0.5) / perArm
          // Denser toward the centre, thinning out along the arms.
          const d = R * (0.12 + 0.84 * Math.pow(t, 1.05)) * (1 + gauss(r) * 0.04 * scatter)
          const a = (arm * TAU) / arms + wind * (d / R) + gauss(r) * (0.2 + 0.14 * (1 - t)) * scatter
          x = Math.cos(a) * d + gauss(r) * R * 0.04
          y = Math.sin(a) * d + gauss(r) * R * 0.04
        }
        target.set(st, [x, y])
        // Untagged stars take the galaxy's colours: warm gold-white in the core, the galaxy's
        // own hue along the arms, bluer at the edges. (Tags keep their own colour.)
        if (!st.tags.length) {
          const f = clamp(Math.hypot(x, y) / R, 0, 1)
          const h = f < 0.18 ? 42 : f < 0.6 ? hue + (r() - 0.5) * 24 : (hue * 0.4 + 215 * 0.6) + (r() - 0.5) * 20
          const sat2 = f < 0.18 ? 85 : 80
          const l = f < 0.18 ? 84 : 74
          st.color = `hsla(${h}, ${sat2}%, ${l}%, 0.9)`
        }
        st.x = x
        st.y = y
      })
      // Nudge apart only what overlaps (names need room in smaller galaxies).
      const pad = n < 60 ? 14 : n < 200 ? 7 : 3
      const sim = forceSimulation(stars)
        .force('x', forceX<Star>((d) => target.get(d)![0]).strength(0.4))
        .force('y', forceY<Star>((d) => target.get(d)![1]).strength(0.4))
        .force('collide', forceCollide<Star>().radius((d) => d.r * 1.3 + pad).strength(0.8))
        .stop()
      for (let t = 0; t < 140; t++) sim.tick()
      for (const st of stars) {
        st.bx = st.x!
        st.by = st.y!
      }
      gal.stars = stars
      gal.links = links.map((e) => ({ s: byId.get(e.source)!, t: byId.get(e.target)! }))
    }
  })

  const nebula = paintNebula('astralis')
  // Three parallax layers of background stars, in a tile that repeats.
  const TILE = 1600
  const field = [
    { depth: 0.04, n: 520, size: 0.7, alpha: 0.55 },
    { depth: 0.1, n: 260, size: 1.0, alpha: 0.7 },
    { depth: 0.2, n: 110, size: 1.5, alpha: 0.9 },
  ].map((layer, li) => {
    const r = rng('field' + li)
    const stars = Array.from({ length: layer.n }, () => ({
      x: r() * TILE,
      y: r() * TILE,
      s: layer.size * (0.5 + r()),
      hue: r() < 0.2 ? 30 + r() * 30 : r() < 0.4 ? 200 + r() * 30 : 0,
      sat: r() < 0.4 ? 70 : 0,
      ph: r() * TAU,
      tw: 0.3 + r() * 2,
    }))
    // Most stars are static dots in the cached background; a few in front twinkle live.
    const twinkle = li === 0 ? [] : stars.filter((_, i) => i % (li === 1 ? 9 : 4) === 0)
    const dots = stars
      .filter((st) => !twinkle.includes(st))
      .map((st) => ({ x: st.x, y: st.y, s: st.s, style: st.sat ? `hsla(${st.hue}, ${st.sat}%, 85%, ${layer.alpha})` : `rgba(255,255,255,${layer.alpha * 0.85})` }))
    return { ...layer, dots, twinkle }
  })

  // Camera (world point at the centre of the screen, and zoom). Scrolling moves `target`; the
  // camera eases toward it each frame, so big scroll steps glide instead of jump.
  const cam = { x: 0, y: 0, k: 0.6 }
  const target = { x: 0, y: 0, k: 0.6 }
  let flight: { from: typeof cam; to: typeof cam; t0: number; dur: number } | null = null
  const fitAll = () => {
    if (!galaxies.length || !w) return { x: 0, y: 0, k: 0.5 }
    const minX = Math.min(...galaxies.map((g) => g.x - g.R))
    const maxX = Math.max(...galaxies.map((g) => g.x + g.R))
    const minY = Math.min(...galaxies.map((g) => g.y - halfHeight(g)))
    const maxY = Math.max(...galaxies.map((g) => g.y + halfHeight(g) + 60))
    const k = clamp(Math.min(w / (maxX - minX), h / (maxY - minY)) * 0.82, 0.05, 1.2)
    return { x: (minX + maxX) / 2, y: (minY + maxY) / 2, k }
  }
  // The whole tilted disc with its name below, clear of the top and bottom bars.
  const fitGalaxy = (g: Galaxy) => {
    const k = clamp(Math.min(w / (g.R * 2.4), (h - 170) / (halfHeight(g) * 2.3)), 0.15, 2.2)
    // Nudge up a little so the galaxy's name clears the galaxy chips at the bottom.
    return { x: g.x, y: g.y + 22 / k, k }
  }
  const fly = (to: typeof cam, dur = 1400) => {
    if (reduced) {
      Object.assign(cam, to)
      Object.assign(target, to)
      return
    }
    flight = { from: { ...cam }, to, t0: performance.now(), dur }
  }

  let hover: Star | null = null
  let hoverGalaxy: Galaxy | null = null
  const shooting: { x: number; y: number; vx: number; vy: number; t0: number }[] = []
  let nextShot = performance.now() + 3000
  let raf = 0
  let last = performance.now()
  let time = 0

  const toScreen = (x: number, y: number) => ({ x: (x - cam.x) * cam.k + w / 2, y: (y - cam.y) * cam.k + h / 2 })

  function frame(now: number) {
    raf = requestAnimationFrame(frame)
    if (document.hidden) return
    const dt = Math.min(0.05, (now - last) / 1000)
    last = now
    if (!reduced) time += dt
    if (flight) {
      const t = clamp((now - flight.t0) / flight.dur, 0, 1)
      const e = ease(t)
      // Zoom out a little mid-flight for a sense of travel.
      const arc = Math.sin(Math.PI * t) * 0.35
      cam.x = flight.from.x + (flight.to.x - flight.from.x) * e
      cam.y = flight.from.y + (flight.to.y - flight.from.y) * e
      cam.k = Math.exp(Math.log(flight.from.k) + (Math.log(flight.to.k) - Math.log(flight.from.k)) * e) * (1 - arc * 0.5)
      if (t >= 1) flight = null
      Object.assign(target, cam)
    } else if (!drag) {
      const f = reduced ? 1 : 1 - Math.exp(-dt * 16)
      cam.x += (target.x - cam.x) * f
      cam.y += (target.y - cam.y) * f
      cam.k = Math.exp(Math.log(cam.k) + (Math.log(target.k) - Math.log(cam.k)) * f)
    }
    paint(now)
  }

  function paint(now: number) {
    // Deep space, nebulae, the starfield and the vignette: one cached layer (redrawn only when
    // the view moves), so a still frame copies it once instead of repainting a dozen layers.
    const key = `${w}x${h}:${cam.x.toFixed(2)},${cam.y.toFixed(2)},${cam.k.toFixed(5)}`
    if (key !== bgKey) {
      composeBackground()
      bgKey = key
    }
    ctx.setTransform(1, 0, 0, 1, 0, 0)
    ctx.globalCompositeOperation = 'copy'
    ctx.drawImage(bgLayer, 0, 0, el.width, el.height)
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.globalCompositeOperation = 'lighter'

    // A few background stars twinkle live.
    for (const layer of field) {
      const ox = (((-cam.x * layer.depth * cam.k) % TILE) + TILE) % TILE
      const oy = (((-cam.y * layer.depth * cam.k) % TILE) + TILE) % TILE
      for (const s of layer.twinkle) {
        const tw = reduced ? 1 : 0.55 + 0.45 * Math.sin(time * s.tw + s.ph)
        ctx.fillStyle = s.sat ? `hsla(${s.hue}, ${s.sat}%, 85%, ${layer.alpha * tw})` : `rgba(255,255,255,${layer.alpha * tw})`
        for (let x = s.x + ox - TILE; x < w; x += TILE) {
          if (x < -2) continue
          for (let y = s.y + oy - TILE; y < h; y += TILE) if (y >= -2) ctx.fillRect(x, y, s.s, s.s)
        }
      }
    }

    // Shooting stars.
    if (!reduced && now > nextShot) {
      const fromLeft = Math.random() < 0.5
      shooting.push({ x: fromLeft ? Math.random() * w * 0.5 : w * (0.5 + Math.random() * 0.5), y: Math.random() * h * 0.4, vx: (fromLeft ? 1 : -1) * (500 + Math.random() * 400), vy: 220 + Math.random() * 200, t0: now })
      nextShot = now + 5000 + Math.random() * 9000
    }
    for (let i = shooting.length - 1; i >= 0; i--) {
      const s = shooting[i]
      const t = (now - s.t0) / 1000
      if (t > 1.1) {
        shooting.splice(i, 1)
        continue
      }
      const x = s.x + s.vx * t
      const y = s.y + s.vy * t
      const a = Math.sin(Math.PI * (t / 1.1))
      const g = ctx.createLinearGradient(x, y, x - s.vx * 0.18, y - s.vy * 0.18)
      g.addColorStop(0, `rgba(255,255,255,${0.9 * a})`)
      g.addColorStop(1, 'rgba(160,170,255,0)')
      ctx.strokeStyle = g
      ctx.lineWidth = 1.6
      ctx.beginPath()
      ctx.moveTo(x, y)
      ctx.lineTo(x - s.vx * 0.18, y - s.vy * 0.18)
      ctx.stroke()
    }

    const q = query.trim().toLowerCase()
    const focus = hover ? new Set(hover.galaxy.links.flatMap((l) => (l.s === hover ? [l.t] : l.t === hover ? [l.s] : []))) : null

    for (const g of galaxies) {
      const c = toScreen(g.x, g.y)
      const R = g.R * cam.k
      if (c.x + R * 1.4 < 0 || c.x - R * 1.4 > w || c.y + R * 1.4 < 0 || c.y - R * 1.4 > h) {
        for (const s of g.stars) s.sx = s.sy = -1e6
        continue
      }
      const spin = time * g.spin
      if (g.locked) {
        // Locked: its stars are hidden — just a faint, cloaked haze.
        const haze = ctx.createRadialGradient(c.x, c.y, 0, c.x, c.y, R * 0.7)
        haze.addColorStop(0, `hsla(${g.hue}, 40%, 70%, 0.14)`)
        haze.addColorStop(1, 'hsla(0,0%,0%,0)')
        ctx.fillStyle = haze
        ctx.beginPath()
        ctx.arc(c.x, c.y, R * 0.7, 0, TAU)
        ctx.fill()
      }
      // Stars orbit with the disc (in its tilted plane).
      const cs = Math.cos(spin * 0.6)
      const sn = Math.sin(spin * 0.6)
      const ca = Math.cos(g.angle)
      const sa = Math.sin(g.angle)
      // Stars sit in a thick halo around the disc (a thin plane would crowd their names).
      const flat = 0.25 + g.tilt * 0.75
      for (const s of g.stars) {
        const rx = s.bx * cs - s.by * sn
        const ry = (s.bx * sn + s.by * cs) * flat
        s.sx = c.x + (rx * ca - ry * sa) * cam.k
        s.sy = c.y + (rx * sa + ry * ca) * cam.k
      }

      // Big galaxies from afar: one layer instead of a thousand tiny stars (crossfades in).
      const layerMix = g.stars.length > 150 ? clamp((0.42 - cam.k) / 0.12, 0, 1) : 0
      const highlighting = !!(focus || q)
      if (layerMix > 0) {
        const L = galaxyLayer(g)
        ctx.save()
        ctx.translate(c.x, c.y)
        ctx.rotate(g.angle)
        ctx.scale(cam.k, cam.k * flat)
        ctx.rotate(spin * 0.6)
        ctx.globalAlpha = layerMix * (highlighting ? 0.3 : 1)
        ctx.drawImage(L.c, -L.world / 2, -L.world / 2, L.world, L.world)
        ctx.restore()
        ctx.globalAlpha = 1
      }

      // Constellation lines: faint from afar (the stars carry the shape), and fainter still in
      // galaxies with many links, so hubs don't smear webs across the view.
      const lineFade = clamp((cam.k - 0.12) / 0.6, 0.03, 1) * (cam.k > 0.9 ? 1 : clamp(Math.sqrt(160 / Math.max(1, g.links.length)), 0.3, 1))
      // Constellation lines: ordinary and dimmed ones in one stroke each; active ones on top.
      ctx.lineWidth = 1
      const normal = new Path2D()
      const faded = new Path2D()
      const activeLinks: typeof g.links = []
      for (const l of g.links) {
        const active = hover && (l.s === hover || l.t === hover)
        if (active) {
          activeLinks.push(l)
          continue
        }
        if (layerMix >= 1 && !focus && !q) continue
        if ((l.s.sx < -50 && l.t.sx < -50) || (l.s.sx > w + 50 && l.t.sx > w + 50) || (l.s.sy < -50 && l.t.sy < -50) || (l.s.sy > h + 50 && l.t.sy > h + 50)) continue
        const dimmed = focus || (q && !l.s.title.toLowerCase().includes(q) && !l.t.title.toLowerCase().includes(q))
        const path = dimmed ? faded : normal
        path.moveTo(l.s.sx, l.s.sy)
        path.lineTo(l.t.sx, l.t.sy)
      }
      if (0.32 * lineFade * 0.75 > 0.012) {
        ctx.strokeStyle = `hsla(${g.hue}, 90%, 80%, ${0.32 * lineFade * 0.75})`
        ctx.stroke(normal)
      }
      ctx.strokeStyle = `hsla(${g.hue}, 90%, 80%, ${0.05 * 0.75})`
      ctx.stroke(faded)
      for (const l of activeLinks) {
        const active = true
        const a = 0.9
        {
          const grad = ctx.createLinearGradient(l.s.sx, l.s.sy, l.t.sx, l.t.sy)
          grad.addColorStop(0, `hsla(${g.hue}, 90%, 82%, ${a})`)
          grad.addColorStop(0.5, `hsla(${g.hue}, 90%, 88%, ${a * 0.6})`)
          grad.addColorStop(1, `hsla(${g.hue}, 90%, 82%, ${a})`)
          ctx.strokeStyle = grad
        }
        ctx.lineWidth = active ? 1.6 : 1
        ctx.beginPath()
        ctx.moveTo(l.s.sx, l.s.sy)
        ctx.lineTo(l.t.sx, l.t.sy)
        ctx.stroke()
        if (active && !reduced) {
          // Light travelling along the link.
          for (let k = 0; k < 2; k++) {
            const t = (time * 0.7 + k * 0.5) % 1
            const from = l.s === hover ? l.s : l.t
            const to = from === l.s ? l.t : l.s
            const px = from.sx + (to.sx - from.sx) * t
            const py = from.sy + (to.sy - from.sy) * t
            ctx.drawImage(glow(`hsla(${g.hue}, 100%, 80%, 0.9)`), px - 7, py - 7, 14, 14)
          }
        }
      }

      // From afar each star also sheds a wide, faint glow: where stars crowd (core, arms) the
      // light pools into the galaxy's shape. Only the notes make it; nothing is painted behind.
      const far = clamp((0.75 - cam.k) / 0.55, 0, 1)
      if (far > 0 && g.stars.length > 12 && layerMix < 1) {
        // Big galaxies have many overlapping glows: soften each so the core doesn't blow out.
        // Sampled in big galaxies (the glow is soft; a few hundred carry the shape).
        const stride = Math.max(1, Math.ceil(g.stars.length / 300))
        ctx.globalAlpha = Math.min(1, 0.07 * far * clamp(Math.sqrt(30 / g.stars.length), 0.18, 1) * stride) * (1 - layerMix)
        for (let i = 0; i < g.stars.length; i += stride) {
          const s = g.stars[i]
          if (s.sx < -60 || s.sx > w + 60 || s.sy < -60 || s.sy > h + 60) continue
          const gr = 16 + s.r * 2
          ctx.drawImage(haze(s.color), s.sx - gr, s.sy - gr, gr * 2, gr * 2)
        }
        ctx.globalAlpha = 1
      }

      // Far away, a dense galaxy's stars overlap; dim each so they don't add up to white.
      const crowd = 1 - far * (1 - clamp(Math.sqrt(90 / Math.max(1, g.stars.length)), 0.4, 1))
      // The stars.
      const zoomScale = clamp(Math.pow(cam.k, 0.7), 0.16, 1.8)
      for (const s of g.stars) {
        if (s.sx < -40 || s.sx > w + 40 || s.sy < -40 || s.sy > h + 40) continue
        const lit = s === hover || (focus && focus.has(s)) || (q && s.title.toLowerCase().includes(q))
        if (layerMix >= 1 && !lit) continue
        const match = q && s.title.toLowerCase().includes(q)
        const dim = (focus && s !== hover && !focus.has(s)) || (q && !match)
        const tw = reduced ? 1 : 0.78 + 0.22 * Math.sin(time * s.twinkle + s.phase)
        const big = s === hover ? 1.7 : match ? 1.6 : 1
        const rad = Math.max(s.r * zoomScale * 3.2, 2.2) * big * (0.9 + 0.1 * tw)
        ctx.globalAlpha = (dim ? 0.18 : 0.75 + 0.25 * tw) * crowd * (lit ? 1 : 1 - layerMix)
        ctx.drawImage(glow(s.color), s.sx - rad, s.sy - rad, rad * 2, rad * 2)
        if (s.kind === 'whiteboard' && !dim) {
          // Whiteboards wear a ring, like a planet.
          ctx.strokeStyle = s.color.replace(/[\d.]+\)$/, '0.6)')
          ctx.lineWidth = 1
          ctx.beginPath()
          ctx.ellipse(s.sx, s.sy, rad * 0.75, rad * 0.25, -0.4, 0, TAU)
          ctx.stroke()
        }
        if ((s.pinned || s === hover || match) && !dim) {
          // The Astralis star: four-point diffraction spikes.
          const L = rad * (s === hover ? 2.6 : 1.9)
          for (const [dx, dy] of [
            [1, 0],
            [0, 1],
          ]) {
            const gr = ctx.createLinearGradient(s.sx - dx * L, s.sy - dy * L, s.sx + dx * L, s.sy + dy * L)
            gr.addColorStop(0, 'rgba(255,255,255,0)')
            gr.addColorStop(0.5, 'rgba(255,255,255,0.85)')
            gr.addColorStop(1, 'rgba(255,255,255,0)')
            ctx.strokeStyle = gr
            ctx.lineWidth = 1.1
            ctx.beginPath()
            ctx.moveTo(s.sx - dx * L, s.sy - dy * L)
            ctx.lineTo(s.sx + dx * L, s.sy + dy * L)
            ctx.stroke()
          }
        }
      }
      ctx.globalAlpha = 1

      // Note names when close enough (or hovered/related/matching).
      ctx.globalCompositeOperation = 'source-over'
      ctx.textAlign = 'center'
      ctx.font = '500 12px Inter, system-ui, sans-serif'
      const labelsAll = clamp((cam.k - 0.75) / 0.35, 0, 1)
      ctx.lineJoin = 'round'
      ctx.lineWidth = 3
      ctx.strokeStyle = 'rgba(2,2,10,0.85)'
      for (const s of g.stars) {
        if (s.sx < -150 || s.sx > w + 150 || s.sy < -30 || s.sy > h + 30) continue
        const show = s === hover || (focus && focus.has(s)) || (q && s.title.toLowerCase().includes(q)) ? 1 : labelsAll
        if (show <= 0.02 || (focus && s !== hover && !focus.has(s))) continue
        const text = s.title || 'Untitled'
        const y = s.sy + s.r * zoomScale * 3 + 14
        ctx.globalAlpha = show * (s === hover ? 1 : 0.82)
        const label = text.length > 40 ? text.slice(0, 39) + '…' : text
        ctx.strokeText(label, s.sx, y)
        ctx.fillStyle = s === hover ? '#ffffff' : 'rgba(225,225,245,0.92)'
        ctx.fillText(label, s.sx, y)
      }

      // The galaxy's name, readable from afar.
      const nameAlpha = g === hoverGalaxy ? 1 : clamp((1.15 - cam.k) / 0.5, 0.25, 1)
      const ny = c.y + halfHeight(g) * cam.k + 24
      ctx.globalAlpha = nameAlpha
      const tag = nameTag(g)
      ctx.drawImage(tag, c.x - tag.width / 4, ny - 26, tag.width / 2, tag.height / 2)
      ctx.globalAlpha = 1
      ctx.globalCompositeOperation = 'lighter'
    }
    ctx.globalCompositeOperation = 'source-over'
  }

  /** Paints the static background for the current view into `bgLayer`: the soft parts
   *  (backdrop, nebulae, vignette) at quarter resolution, then the background stars as dots. */
  function composeBackground() {
    const Q = 4
    const sw = Math.max(1, Math.ceil(w / Q))
    const shh = Math.max(1, Math.ceil(h / Q))
    if (soft.width !== sw || soft.height !== shh) soft = canvas(sw, shh)
    const sx = soft.getContext('2d')!
    sx.setTransform(1 / Q, 0, 0, 1 / Q, 0, 0)
    sx.globalCompositeOperation = 'source-over'
    sx.globalAlpha = 1
    sx.drawImage(backdrop, 0, 0, w, h)
    sx.globalCompositeOperation = 'lighter'
    const nk = 1.4 + Math.log2(1 + cam.k) * 0.25
    const ns = 1024 * nk
    const nox = (((-cam.x * 0.06 * cam.k) % ns) + ns) % ns
    const noy = (((-cam.y * 0.06 * cam.k) % ns) + ns) % ns
    sx.globalAlpha = 0.95
    for (let x = nox - ns; x < w; x += ns) for (let y = noy - ns; y < h; y += ns) sx.drawImage(nebula, x, y, ns, ns)
    sx.globalAlpha = 1
    sx.globalCompositeOperation = 'source-over'
    sx.drawImage(vignette, 0, 0, w, h)

    if (bgLayer.width !== Math.max(1, Math.round(w)) || bgLayer.height !== Math.max(1, Math.round(h))) bgLayer = canvas(w, h)
    const b = bgLayer.getContext('2d')!
    b.imageSmoothingEnabled = true
    b.globalCompositeOperation = 'copy'
    b.drawImage(soft, 0, 0, w, h)
    b.globalCompositeOperation = 'lighter'
    for (const layer of field) {
      const ox = (((-cam.x * layer.depth * cam.k) % TILE) + TILE) % TILE
      const oy = (((-cam.y * layer.depth * cam.k) % TILE) + TILE) % TILE
      for (const st of layer.dots) {
        b.fillStyle = st.style
        for (let x = st.x + ox - TILE; x < w; x += TILE) {
          if (x < -2) continue
          for (let y = st.y + oy - TILE; y < h; y += TILE) if (y >= -2) b.fillRect(x, y, st.s, st.s)
        }
      }
    }
  }

  // ---- input ----
  const pos = (e: MouseEvent) => {
    const r = el.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }
  const starAt = (p: { x: number; y: number }) => {
    let best: Star | null = null
    let bestD = Infinity
    for (const g of galaxies)
      for (const s of g.stars) {
        const d = Math.hypot(s.sx - p.x, s.sy - p.y)
        const reach = Math.max(9, s.r * clamp(Math.pow(cam.k, 0.7), 0.16, 1.8) * 2.2)
        if (d < reach && d < bestD) {
          best = s
          bestD = d
        }
      }
    return best
  }
  const galaxyAt = (p: { x: number; y: number }) =>
    galaxies.find((g) => {
      const c = toScreen(g.x, g.y)
      return Math.hypot(c.x - p.x, c.y - p.y) < Math.max(40, g.R * cam.k * 0.6)
    }) ?? null

  let drag: { x: number; y: number; cx: number; cy: number; moved: boolean } | null = null
  const onDown = (e: MouseEvent) => {
    if (e.button !== 0) return
    const p = pos(e)
    drag = { x: p.x, y: p.y, cx: cam.x, cy: cam.y, moved: false }
    flight = null
    el.classList.add('dragging')
  }
  const onMove = (e: MouseEvent) => {
    const p = pos(e)
    if (drag) {
      const dx = p.x - drag.x
      const dy = p.y - drag.y
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true
      cam.x = drag.cx - dx / cam.k
      cam.y = drag.cy - dy / cam.k
      Object.assign(target, cam)
      return
    }
    if (e.target !== el) return
    hover = starAt(p)
    hoverGalaxy = hover ? null : galaxyAt(p)
    el.style.cursor = hover || hoverGalaxy ? 'pointer' : ''
    el.title = hover ? hover.title || 'Untitled' : hoverGalaxy ? `${hoverGalaxy.name} — click to fly here` : ''
  }
  const onUp = (e: MouseEvent) => {
    if (drag && !drag.moved) {
      const p = pos(e)
      const s = starAt(p)
      if (s) onOpen(s.id)
      else {
        const g = galaxyAt(p)
        if (g) {
          fly(fitGalaxy(g))
          onGalaxy?.(g.id)
        }
      }
    }
    drag = null
    el.classList.remove('dragging')
  }
  const zoomAt = (factor: number, px: number, py: number) => {
    const k = clamp(target.k * factor, 0.04, 4)
    const wx = (px - w / 2) / target.k + target.x
    const wy = (py - h / 2) / target.k + target.y
    target.k = k
    target.x = wx - (px - w / 2) / k
    target.y = wy - (py - h / 2) / k
  }
  // Like the whiteboard: two-finger scroll pans in any direction (a mouse wheel pans up/down,
  // Shift + wheel sideways); Ctrl/⌘ + scroll — or a trackpad pinch — zooms at the pointer.
  const onWheel = (e: WheelEvent) => {
    e.preventDefault()
    flight = null
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1
    const dx = e.deltaX * unit
    const dy = e.deltaY * unit
    if (e.ctrlKey || e.metaKey) {
      const p = pos(e)
      // Each event zooms at most ~11% (a mouse-wheel notch); small trackpad steps much less.
      const step = clamp(dy, -32, 32)
      zoomAt(Math.exp(-step * 0.0036), p.x, p.y)
      return
    }
    // Trackpad scroll distances, at a comfortable speed.
    const speed = 0.55 / target.k
    if (e.shiftKey && dx === 0) {
      target.x += dy * speed
      return
    }
    target.x += dx * speed
    target.y += dy * speed
  }
  let gestureScale = 1
  const onGestureStart = (e: Event) => {
    e.preventDefault()
    gestureScale = 1
  }
  const onGestureChange = (e: Event) => {
    e.preventDefault()
    const g = e as Event & { scale: number; clientX: number; clientY: number }
    const r = el.getBoundingClientRect()
    zoomAt(g.scale / gestureScale, g.clientX - r.left, g.clientY - r.top)
    gestureScale = g.scale
  }

  let backdrop = canvas(1, 1)
  let vignette = canvas(1, 1)
  let bgLayer = canvas(1, 1)
  let soft = canvas(1, 1)
  let bgKey = ''
  /** Gradients that only change with the size of the view. */
  const paintFrame = () => {
    backdrop = canvas(w, h)
    let x = backdrop.getContext('2d')!
    const bg = x.createRadialGradient(w / 2, h / 2, 0, w / 2, h / 2, Math.max(w, h) * 0.8)
    bg.addColorStop(0, '#0b0a1d')
    bg.addColorStop(1, '#020208')
    x.fillStyle = bg
    x.fillRect(0, 0, w, h)
    vignette = canvas(w, h)
    x = vignette.getContext('2d')!
    const v = x.createRadialGradient(w / 2, h / 2, Math.min(w, h) * 0.45, w / 2, h / 2, Math.max(w, h) * 0.78)
    v.addColorStop(0, 'rgba(0,0,0,0)')
    v.addColorStop(1, 'rgba(0,0,6,0.55)')
    x.fillStyle = v
    x.fillRect(0, 0, w, h)
  }

  const nameTags = new Map<Galaxy, HTMLCanvasElement>()
  /** A galaxy's glowing name and star count, rendered once (blurred glows are slow to redraw). */
  const nameTag = (g: Galaxy) => {
    let t = nameTags.get(g)
    if (t) return t
    const W = 640
    const H = 96
    t = canvas(W, H) // 2x for sharpness
    const x = t.getContext('2d')!
    x.scale(2, 2)
    x.textAlign = 'center'
    x.textBaseline = 'alphabetic'
    x.font = '600 13px Inter, system-ui, sans-serif'
    x.shadowColor = `hsla(${g.hue}, 90%, 60%, 0.9)`
    x.shadowBlur = 14
    x.fillStyle = '#f2f0ff'
    spacedText(x, g.name.toUpperCase(), W / 4, 26, 2.2)
    x.shadowBlur = 0
    x.font = '11px Inter, system-ui, sans-serif'
    x.fillStyle = `hsla(${g.hue}, 60%, 80%, 0.75)`
    const sub = g.locked ? 'Locked · encrypted' : g.encrypted ? 'Encrypted · notes hidden' : `${g.count} ${g.count === 1 ? 'star' : 'stars'}`
    x.fillText(sub, W / 4, 43)
    nameTags.set(g, t)
    return t
  }

  /** A big galaxy's stars and their pooled glow, painted once in the galaxy's own frame. Drawn as
   *  one turning image when zoomed out (where each star is a tiny dot anyway). */
  const layers = new Map<Galaxy, { c: HTMLCanvasElement; world: number }>()
  const galaxyLayer = (g: Galaxy) => {
    let l = layers.get(g)
    if (l) return l
    const L = 1600
    const world = g.R * 2.5 // world units across the layer
    const sc = L / world
    const c = canvas(L, L)
    const x = c.getContext('2d')!
    x.globalCompositeOperation = 'lighter'
    const crowd = clamp(Math.sqrt(90 / g.stars.length), 0.4, 1)
    x.globalAlpha = 0.07 * clamp(Math.sqrt(30 / g.stars.length), 0.18, 1) * 1.6
    for (const st of g.stars) {
      const gr = 28 + st.r * 3
      x.drawImage(haze(st.color), L / 2 + st.bx * sc - gr, L / 2 + st.by * sc - gr, gr * 2, gr * 2)
    }
    x.globalAlpha = 0.9 * crowd
    for (const st of g.stars) {
      const rad = 4 + st.r * 1.6
      x.drawImage(glow(st.color), L / 2 + st.bx * sc - rad, L / 2 + st.by * sc - rad, rad * 2, rad * 2)
    }
    l = { c, world }
    layers.set(g, l)
    return l
  }

  let started = false
  const resize = () => {
    // Sharp on HiDPI without painting 4x the pixels.
    dpr = Math.min(window.devicePixelRatio || 1, 1.5)
    w = el.clientWidth
    h = el.clientHeight
    el.width = Math.round(w * dpr)
    el.height = Math.round(h * dpr)
    if (w > 0 && h > 0) paintFrame()
    bgKey = ''
    if (!started && w > 0) {
      started = true
      // Arrive from the wide view, gliding into the current vault's galaxy.
      Object.assign(cam, fitAll())
      Object.assign(target, cam)
      if (galaxies[0]) fly(fitGalaxy(galaxies[0]), 2200)
    }
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
    flyTo(id) {
      const g = id ? galaxies.find((x) => x.id === id) : null
      fly(g ? fitGalaxy(g) : fitAll())
    },
    setQuery(q) {
      query = q
    },
    benchmark(frames = 60, panning = false) {
      const t0 = performance.now()
      for (let i = 0; i < frames; i++) {
        time += 1 / 60
        if (panning) {
          cam.x += 4 / cam.k
          Object.assign(target, cam)
        }
        paint(performance.now())
      }
      return (performance.now() - t0) / frames
    },
  }
}
