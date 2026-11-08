// The graph as a view into 3D space you can fly around. Every vault is a galaxy. Notes that link
// together form solar systems: the most-linked note is the sun, the notes it links to are planets
// orbiting it, and notes reached through a planet are that planet's moons. Unlinked notes are lone
// stars strewn along the galaxy's spiral arms, so a big vault looks like a galaxy and a small one
// like a few stars (there's no galaxy picture behind the notes). Plain canvas 2D with a small
// hand-rolled 3D projection: no WebGL needed.

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

type V3 = [number, number, number]
type Role = 'sun' | 'planet' | 'moon' | 'star'

interface Body extends GraphNode {
  galaxy: Galaxy
  system: System | null
  role: Role
  /** Radius in world units. */
  r: number
  color: string
  /** Suns: the colour of the glow around them. */
  corona: string
  /** What it orbits (planets: the sun; moons: their planet), how far out, and how fast. */
  parent: Body | null
  orbit: number
  phase: number
  speed: number
  /** Lone stars: a fixed place in the galaxy's own frame. */
  home: V3
  twinkle: number
  // Each frame:
  local: V3
  world: V3
  sx: number
  sy: number
  /** Distance from the camera along the view, and on-screen radius in pixels. */
  d: number
  px: number
  shown: boolean
}

interface System {
  galaxy: Galaxy
  sun: Body
  bodies: Body[]
  links: [Body, Body][]
  /** Centre in the galaxy's frame, and the plane its planets orbit in. */
  center: V3
  u: V3
  v: V3
  /** How far its outermost moon reaches. */
  ext: number
  rings: number[]
  world: V3
  sx: number
  sy: number
  extPx: number
  shown: boolean
}

interface Galaxy {
  id: string
  name: string
  locked: boolean
  encrypted: boolean
  hue: number
  pos: V3
  R: number
  /** Orientation: the galaxy's x, y (up, out of the disc) and z axes in world space. */
  ex: V3
  ey: V3
  ez: V3
  spin: number
  angle: number
  systems: System[]
  stars: Body[]
  bodies: Body[]
  count: number
  // Each frame:
  sx: number
  sy: number
  Rpx: number
  d: number
  shown: boolean
}

type Focus = { kind: 'all' } | { kind: 'galaxy'; g: Galaxy } | { kind: 'system'; s: System }

// ---- small helpers ----------------------------------------------------------------------

const TAU = Math.PI * 2
const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v))
const add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
const scale = (a: V3, k: number): V3 => [a[0] * k, a[1] * k, a[2] * k]
const len = (a: V3) => Math.hypot(a[0], a[1], a[2])
const norm = (a: V3): V3 => scale(a, 1 / (len(a) || 1))
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]

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
const GALAXY_HUES = [262, 190, 318, 32, 162, 222, 350, 285]

/** A soft round glow, cached per colour. */
const glowCache = new Map<string, HTMLCanvasElement>()
function glow(color: string, core = 0.22) {
  const key = `${color}|${core}`
  let c = glowCache.get(key)
  if (c) return c
  c = document.createElement('canvas')
  c.width = c.height = 64
  const x = c.getContext('2d')!
  const g = x.createRadialGradient(32, 32, 0, 32, 32, 32)
  g.addColorStop(0, 'rgba(255,255,255,1)')
  g.addColorStop(core * 0.5, 'rgba(255,255,255,0.95)')
  g.addColorStop(core, color)
  g.addColorStop(0.55, color.replace(/[\d.]+\)$/, '0.16)'))
  g.addColorStop(1, 'rgba(0,0,0,0)')
  x.fillStyle = g
  x.fillRect(0, 0, 64, 64)
  glowCache.set(key, c)
  return c
}

/** Text with letter spacing, drawn a letter at a time (spacing characters show as boxes in some
 *  webviews). */
function spacedText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, spacing: number) {
  const widths = [...text].map((ch) => ctx.measureText(ch).width)
  const total = widths.reduce((a, b) => a + b, 0) + spacing * Math.max(0, widths.length - 1)
  let cx = x - total / 2
  const align = ctx.textAlign
  ctx.textAlign = 'left'
  ;[...text].forEach((ch, i) => {
    ctx.strokeText(ch, cx, y)
    ctx.fillText(ch, cx, y)
    cx += widths[i] + spacing
  })
  ctx.textAlign = align
}

// ---- building the universe ---------------------------------------------------------------

/** Splits a vault's notes into solar systems (linked groups) and lone stars. */
function buildGalaxy(input: GalaxyInput, index: number): Galaxy {
  const r = rng(input.id)
  const hue = GALAXY_HUES[index % GALAXY_HUES.length]
  const tilt1 = gauss(r) * 0.22
  const tilt2 = gauss(r) * 0.22
  // Orientation: tip the disc a little so galaxies don't all lie in one plane.
  const ey = norm([Math.sin(tilt1), 1, Math.sin(tilt2)])
  const ex = norm(cross(ey, [0, 0, 1]))
  const ez = cross(ex, ey)
  const g: Galaxy = {
    id: input.id,
    name: input.name,
    locked: input.locked,
    encrypted: input.encrypted,
    hue,
    pos: [0, 0, 0],
    R: 60,
    ex,
    ey,
    ez,
    spin: 0.004 + r() * 0.004,
    angle: r() * TAU,
    systems: [],
    stars: [],
    bodies: [],
    count: input.nodes.length,
    sx: 0,
    sy: 0,
    Rpx: 0,
    d: 0,
    shown: false,
  }
  if (input.locked) return g

  const make = (nd: GraphNode, role: Role): Body => ({
    ...nd,
    galaxy: g,
    system: null,
    role,
    r: 1,
    color: '',
    corona: '',
    parent: null,
    orbit: 0,
    phase: r() * TAU,
    speed: 0,
    home: [0, 0, 0],
    twinkle: 0.6 + r() * 1.8,
    local: [0, 0, 0],
    world: [0, 0, 0],
    sx: 0,
    sy: 0,
    d: 0,
    px: 0,
    shown: false,
  })
  const tagHue = (nd: GraphNode) => (nd.tags.length ? hashHue(nd.tags[0]) : null)

  const byId = new Map(input.nodes.map((n) => [n.id, n]))
  const adj = new Map<string, string[]>()
  for (const e of input.edges) {
    if (!byId.has(e.source) || !byId.has(e.target) || e.source === e.target) continue
    adj.set(e.source, [...(adj.get(e.source) ?? []), e.target])
    adj.set(e.target, [...(adj.get(e.target) ?? []), e.source])
  }
  const important = (a: GraphNode, b: GraphNode) => b.degree - a.degree || Number(b.pinned) - Number(a.pinned) || a.title.localeCompare(b.title)

  // Linked groups: each becomes a solar system around its most-linked note.
  const seen = new Set<string>()
  const lone: GraphNode[] = []
  for (const start of [...input.nodes].sort(important)) {
    if (seen.has(start.id)) continue
    if (!(adj.get(start.id)?.length)) {
      seen.add(start.id)
      lone.push(start)
      continue
    }
    // Breadth-first from the sun: distance 1 = planets; further = moons of the planet they're
    // reached through.
    const sun = make(start, 'sun')
    const bodies: Body[] = [sun]
    const via = new Map<string, Body>() // note → the planet it hangs off
    const queue: [string, number][] = [[start.id, 0]]
    seen.add(start.id)
    const planets: Body[] = []
    const moons: Body[] = []
    while (queue.length) {
      const [id, depth] = queue.shift()!
      const neighbours = (adj.get(id) ?? []).map((n) => byId.get(n)!).sort(important)
      for (const nd of neighbours) {
        if (seen.has(nd.id)) continue
        seen.add(nd.id)
        if (depth === 0) {
          const p = make(nd, 'planet')
          p.parent = sun
          planets.push(p)
          via.set(nd.id, p)
        } else {
          const host = via.get(id)!
          const m = make(nd, 'moon')
          m.parent = host
          moons.push(m)
          via.set(nd.id, host)
        }
        queue.push([nd.id, depth + 1])
      }
    }
    bodies.push(...planets, ...moons)

    // Sizes and colours: the sun warm and bright, planets in their tag's colour.
    // Suns look like suns (warm, a little different each); a tag tints the corona.
    const th0 = tagHue(sun)
    sun.r = 2.6 + Math.sqrt(sun.degree) * 0.9 + (sun.pinned ? 0.8 : 0)
    sun.color = `hsla(${28 + r() * 26}, 100%, 72%, 0.95)`
    sun.corona = th0 === null ? sun.color : `hsla(${th0}, 85%, 70%, 0.9)`
    // Moons orbit their planet; a planet's moon cloud decides how much room it needs.
    const moonsOf = new Map<Body, Body[]>()
    for (const m of moons) moonsOf.set(m.parent!, [...(moonsOf.get(m.parent!) ?? []), m])
    const reach = new Map<Body, number>()
    for (const p of planets) {
      p.r = 1.3 + Math.sqrt(p.degree) * 0.38 + (p.pinned ? 0.5 : 0)
      const h = tagHue(p) ?? hue + (r() - 0.5) * 70
      p.color = `hsla(${h}, 85%, 70%, 0.95)`
      const ms = moonsOf.get(p) ?? []
      ms.forEach((m, i) => {
        m.r = 0.55 + Math.sqrt(m.degree) * 0.12
        const mh = tagHue(m) ?? h
        m.color = `hsla(${mh}, 45%, 82%, 0.9)`
        // Moons on a few close orbits.
        m.orbit = p.r + 1.6 + (i % 4) * 1.1 + Math.floor(i / 4) * 0.5
        m.speed = (1.4 / Math.sqrt(m.orbit)) * (r() < 0.15 ? -1 : 1)
      })
      reach.set(p, ms.length ? Math.max(...ms.map((m) => m.orbit + m.r)) : p.r)
    }
    // Planets on rings: a few on the inner ring, more on each one further out.
    const rings: number[] = []
    let radius = sun.r * 2.6 + 4
    let i = 0
    let ring = 0
    while (i < planets.length) {
      const cap = 5 + ring * 4
      const onRing = planets.slice(i, i + cap)
      const room = Math.max(...onRing.map((p) => reach.get(p)!))
      radius += room
      const base = r() * TAU
      onRing.forEach((p, k) => {
        p.orbit = radius
        p.phase = base + (k / onRing.length) * TAU + (r() - 0.5) * 0.3
        p.speed = 1.1 / Math.pow(radius, 0.75)
      })
      rings.push(radius)
      radius += room + 3
      i += cap
      ring++
    }
    const ext = rings.length ? rings[rings.length - 1] + Math.max(...planets.map((p) => reach.get(p)!)) : sun.r * 3
    // Each system orbits in its own plane, roughly level with the disc.
    const n = norm([gauss(r) * 0.3, 1, gauss(r) * 0.3])
    const u = norm(cross(n, [0, 0, 1]))
    const v = cross(n, u)
    const sys: System = {
      galaxy: g,
      sun,
      bodies,
      links: [],
      center: [0, 0, 0],
      u,
      v,
      ext,
      rings,
      world: [0, 0, 0],
      sx: 0,
      sy: 0,
      extPx: 0,
      shown: false,
    }
    for (const b of bodies) b.system = sys
    const inSys = new Map(bodies.map((b) => [b.id, b]))
    const done = new Set<string>()
    for (const b of bodies)
      for (const o of adj.get(b.id) ?? []) {
        const key = b.id < o ? `${b.id}|${o}` : `${o}|${b.id}`
        if (done.has(key) || !inSys.has(o)) continue
        done.add(key)
        sys.links.push([b, inSys.get(o)!])
      }
    g.systems.push(sys)
  }

  // How big the galaxy is: room for every system, plus the lone stars.
  const area = g.systems.reduce((a, s) => a + Math.PI * (s.ext + 14) ** 2 * 2.4, 0) + lone.length * 140
  g.R = Math.max(70, Math.sqrt(area / Math.PI) + 30)
  const R = g.R

  // Systems: the biggest near the core, the rest out along the arms, then pushed apart until
  // none overlap.
  const arms = 2 + Math.floor(r() * 3)
  const wind = 2.2 + r() * 1.6
  const sysCount = g.systems.length
  g.systems.forEach((s, k) => {
    const t = (k + 0.5) / Math.max(1, sysCount)
    const d = R * (k === 0 ? 0 : 0.14 + 0.72 * Math.sqrt(t))
    const a = ((k % arms) * TAU) / arms + wind * (d / R) + gauss(r) * 0.25
    s.center = [Math.cos(a) * d, gauss(r) * R * 0.015, Math.sin(a) * d]
  })
  for (let pass = 0; pass < 40; pass++) {
    let moved = false
    for (let a = 0; a < sysCount; a++)
      for (let b = a + 1; b < sysCount; b++) {
        const A = g.systems[a]
        const B = g.systems[b]
        const dx = B.center[0] - A.center[0]
        const dz = B.center[2] - A.center[2]
        const dist = Math.hypot(dx, dz) || 0.01
        const need = A.ext + B.ext + 8
        if (dist >= need) continue
        const push = (need - dist) / 2
        const ux = dx / dist
        const uz = dz / dist
        if (a !== 0) {
          A.center[0] -= ux * push
          A.center[2] -= uz * push
        }
        B.center[0] += ux * push * (a === 0 ? 2 : 1)
        B.center[2] += uz * push * (a === 0 ? 2 : 1)
        moved = true
      }
    if (!moved) break
  }
  const farthest = Math.max(R, ...g.systems.map((s) => Math.hypot(s.center[0], s.center[2]) + s.ext))
  g.R = farthest

  // Lone stars along the spiral arms, denser and thicker toward the core.
  const n = lone.length
  const scatter = clamp(1.5 - n / 70, 0.45, 1.5)
  const perArm = Math.max(1, Math.ceil(n / arms))
  lone.forEach((nd, k) => {
    const s = make(nd, 'star')
    const arm = Math.floor(k / perArm)
    const t = ((k % perArm) + 0.5) / perArm
    const d = g.R * (0.1 + 0.88 * Math.pow(t, 0.9)) * (1 + gauss(r) * 0.05 * scatter)
    const a = (arm * TAU) / arms + wind * (d / g.R) + gauss(r) * (0.18 + 0.12 * (1 - t)) * scatter
    const thick = g.R * (0.012 + 0.05 * Math.exp(-((d / g.R) ** 2) * 6))
    s.home = [Math.cos(a) * d + gauss(r) * g.R * 0.03, gauss(r) * thick, Math.sin(a) * d + gauss(r) * g.R * 0.03]
    s.r = 0.9 + Math.sqrt(s.degree) * 0.3 + (s.pinned ? 0.6 : 0)
    // Untagged stars take the galaxy's colours: gold-white near the core, its own hue along
    // the arms, bluer at the edges.
    const th = tagHue(s)
    if (th !== null) s.color = `hsla(${th}, 90%, 74%, 0.9)`
    else {
      const f = d / g.R
      const h = f < 0.18 ? 42 : f < 0.6 ? hue + (r() - 0.5) * 24 : hue * 0.4 + 215 * 0.6 + (r() - 0.5) * 20
      s.color = `hsla(${h}, ${f < 0.18 ? 85 : 78}%, ${f < 0.18 ? 84 : 76}%, 0.9)`
    }
    g.stars.push(s)
  })
  g.bodies = [...g.systems.flatMap((s) => s.bodies), ...g.stars]
  return g
}

// ---- the view -------------------------------------------------------------------------------

export function createSpace(el: HTMLCanvasElement, inputs: GalaxyInput[], onOpen: (noteId: string) => void, onGalaxy?: (id: string | null) => void): SpaceHandle {
  const ctx = el.getContext('2d')!
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches

  // Galaxies, the current vault's in the middle and the rest around it.
  const ordered = [...inputs].sort((a, b) => Number(b.current) - Number(a.current))
  const galaxies = ordered.map((inp) => buildGalaxy(inp, inputs.indexOf(inp)))
  {
    const r = rng('universe')
    let ringR = 0
    galaxies.forEach((g, i) => {
      if (i === 0) {
        ringR = g.R
        return
      }
      const a = i * 2.39996 // golden angle
      ringR += g.R * 1.1 + 60
      const d = ringR + galaxies[0].R * 0.8
      g.pos = [Math.cos(a) * d, gauss(r) * d * 0.18, Math.sin(a) * d]
    })
  }
  const bodies = galaxies.flatMap((g) => g.bodies)
  const systems = galaxies.flatMap((g) => g.systems)
  const allCenter: V3 = scale(
    galaxies.reduce((s, g) => add(s, g.pos), [0, 0, 0] as V3),
    1 / Math.max(1, galaxies.length),
  )
  const allRadius = Math.max(...galaxies.map((g) => len(add(g.pos, scale(allCenter, -1))) + g.R), 100)

  // The far-off sky: a fixed sphere of faint stars that turns with the camera (no parallax).
  const sky: { p: V3; b: number }[] = []
  {
    const r = rng('sky')
    for (let i = 0; i < 1100; i++) {
      const p = norm([gauss(r), gauss(r) * 0.75, gauss(r)])
      sky.push({ p, b: Math.pow(r(), 3) })
    }
  }

  // ---- camera: orbit around a point ----
  let w = 0
  let h = 0
  let dpr = 1
  let focal = 1
  const view = { yaw: -0.5, pitch: 0.42, dist: allRadius * 2.6, target: [...allCenter] as V3 }
  const goal = { yaw: -0.5, pitch: 0.42, dist: allRadius * 2.6, target: [...allCenter] as V3 }
  let focus: Focus = { kind: 'all' }
  let follow = true // keep a moving system centred
  let idleSince = performance.now()
  const minDist = 5
  const maxDist = allRadius * 4

  let cy = 1
  let sy = 0
  let cp = 1
  let sp = 0
  /** World → screen. `d` is the distance in front of the camera (≤ 0: behind it). */
  const project = (p: V3) => {
    const x0 = p[0] - view.target[0]
    const y0 = p[1] - view.target[1]
    const z0 = p[2] - view.target[2]
    const x1 = x0 * cy + z0 * sy
    const z1 = -x0 * sy + z0 * cy
    const y2 = y0 * cp - z1 * sp
    const z2 = y0 * sp + z1 * cp
    const d = view.dist + z2
    const f = d > 0.5 ? focal / d : 0
    return { x: w / 2 + x1 * f, y: h / 2 + 18 - y2 * f, d, f }
  }
  /** Camera-facing right and up directions (for sliding the view). */
  const right = (): V3 => [cy, 0, sy]
  const up = (): V3 => [sy * sp, cp, -cy * sp]

  const setFocus = (f: Focus, announce = false) => {
    focus = f
    follow = true
    if (f.kind === 'all') {
      goal.target = [...allCenter]
      goal.dist = allRadius * 2.4
      goal.pitch = 0.5
    } else if (f.kind === 'galaxy') {
      goal.target = [...f.g.pos]
      goal.dist = Math.max(f.g.R * 2.4, 60)
      goal.pitch = clamp(goal.pitch, 0.35, 0.8)
    } else {
      goal.target = [...f.s.world]
      goal.dist = Math.max(f.s.ext * 3.2, 24)
    }
    idleSince = performance.now()
    if (announce) onGalaxy?.(f.kind === 'all' ? null : f.kind === 'galaxy' ? f.g.id : f.s.galaxy.id)
  }
  const backOut = () => {
    if (focus.kind === 'system') setFocus({ kind: 'galaxy', g: focus.s.galaxy }, true)
    else if (focus.kind === 'galaxy' && galaxies.length > 1) setFocus({ kind: 'all' }, true)
    else if (focus.kind === 'galaxy') setFocus(focus)
  }

  // ---- motion ----
  let time = 0
  let query = ''
  let hover: Body | null = null
  let hoverSystem: System | null = null
  let raf = 0
  let last = performance.now()

  /** Galaxy frame → world. */
  const toWorld = (g: Galaxy, p: V3): V3 => {
    const ca = Math.cos(g.angle)
    const sa = Math.sin(g.angle)
    const x = p[0] * ca - p[2] * sa
    const z = p[0] * sa + p[2] * ca
    const y = p[1]
    return [
      g.pos[0] + g.ex[0] * x + g.ey[0] * y + g.ez[0] * z,
      g.pos[1] + g.ex[1] * x + g.ey[1] * y + g.ez[1] * z,
      g.pos[2] + g.ex[2] * x + g.ey[2] * y + g.ez[2] * z,
    ]
  }
  const place = () => {
    for (const g of galaxies) {
      for (const s of g.systems) {
        for (const b of s.bodies) {
          if (b.role === 'sun') b.local = s.center
          else {
            const host = b.role === 'planet' ? s.center : b.parent!.local
            const a = b.phase + time * b.speed
            const c = Math.cos(a) * b.orbit
            const sn = Math.sin(a) * b.orbit
            b.local = [host[0] + s.u[0] * c + s.v[0] * sn, host[1] + s.u[1] * c + s.v[1] * sn, host[2] + s.u[2] * c + s.v[2] * sn]
          }
        }
        s.world = toWorld(g, s.center)
      }
      for (const st of g.stars) st.local = st.home
      for (const b of g.bodies) b.world = toWorld(g, b.local)
    }
  }

  function frame(now: number) {
    raf = requestAnimationFrame(frame)
    if (document.hidden) return
    const dt = Math.min(0.05, (now - last) / 1000)
    last = now
    if (!reduced) {
      time += dt
      for (const g of galaxies) g.angle += dt * g.spin
      // Drift slowly around on its own when left alone.
      if (!drag && now - idleSince > 4000) goal.yaw += dt * 0.035
    }
    place()
    if (focus.kind === 'system' && follow) goal.target = [...focus.s.world]
    const f = reduced ? 1 : 1 - Math.exp(-dt * 5)
    const fr = reduced ? 1 : 1 - Math.exp(-dt * 12)
    view.yaw += (goal.yaw - view.yaw) * fr
    view.pitch += (goal.pitch - view.pitch) * fr
    view.dist = Math.exp(Math.log(view.dist) + (Math.log(goal.dist) - Math.log(view.dist)) * f)
    for (let i = 0; i < 3; i++) view.target[i] += (goal.target[i] - view.target[i]) * f
    paint()
  }

  // Light fades with distance beyond what you're looking at, like haze.
  const fog = (d: number) => clamp(1.2 - (d - view.dist) / (view.dist * 1.6), 0.1, 1)

  function paint() {
    cy = Math.cos(view.yaw)
    sy = Math.sin(view.yaw)
    cp = Math.cos(view.pitch)
    sp = Math.sin(view.pitch)
    focal = Math.min(w, h) * 1.15
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.globalCompositeOperation = 'source-over'
    ctx.globalAlpha = 1
    ctx.fillStyle = bg
    ctx.fillRect(0, 0, w, h)

    // The far-off sky (directions only).
    const skyPaths = [new Path2D(), new Path2D(), new Path2D()]
    for (const s of sky) {
      const [x0, y0, z0] = s.p
      const x1 = x0 * cy + z0 * sy
      const z1 = -x0 * sy + z0 * cy
      const y2 = y0 * cp - z1 * sp
      const z2 = y0 * sp + z1 * cp
      if (z2 < 0.05) continue
      const x = w / 2 + (x1 / z2) * focal * 0.5
      const y = h / 2 - (y2 / z2) * focal * 0.5
      if (x < 0 || y < 0 || x > w || y > h) continue
      const i = s.b > 0.5 ? 2 : s.b > 0.15 ? 1 : 0
      skyPaths[i].rect(x, y, i === 2 ? 1.6 : 1.1, i === 2 ? 1.6 : 1.1)
    }
    ;[0.22, 0.4, 0.75].forEach((a, i) => {
      ctx.fillStyle = `rgba(200, 205, 255, ${a})`
      ctx.fill(skyPaths[i])
    })

    // Where everything is on screen.
    for (const g of galaxies) {
      const q = project(g.pos)
      g.sx = q.x
      g.sy = q.y
      g.d = q.d
      g.Rpx = g.R * q.f
      g.shown = q.f > 0
    }
    for (const s of systems) {
      const q = project(s.world)
      s.sx = q.x
      s.sy = q.y
      s.extPx = s.ext * q.f
      s.shown = q.f > 0 && q.x > -s.extPx && q.x < w + s.extPx && q.y > -s.extPx && q.y < h + s.extPx
    }
    for (const b of bodies) {
      const q = project(b.world)
      b.sx = q.x
      b.sy = q.y
      b.d = q.d
      b.px = b.r * q.f
      b.shown = q.f > 0 && q.x > -40 && q.y > -40 && q.x < w + 40 && q.y < h + 40
    }

    const q_ = query.trim().toLowerCase()
    const match = (b: Body) => !!q_ && b.title.toLowerCase().includes(q_)
    const lit = new Set<Body>()
    if (hover) {
      lit.add(hover)
      for (const [a, b] of hover.system?.links ?? []) {
        if (a === hover) lit.add(b)
        if (b === hover) lit.add(a)
      }
    }
    const dimAll = !!hover || !!q_

    ctx.globalCompositeOperation = 'lighter'
    // Orbits and links of the systems close enough to see.
    for (const s of systems) {
      if (!s.shown || s.extPx < 22) continue
      const strength = clamp((s.extPx - 22) / 80, 0, 1) * fog(len(add(s.world, scale(view.target, -1))) + view.dist)
      if (strength <= 0.02) continue
      const g = s.galaxy
      const c = s.center
      ctx.lineWidth = 1
      for (const R of s.rings) {
        const path = new Path2D()
        for (let k = 0; k <= 72; k++) {
          const a = (k / 72) * TAU
          const p = toWorld(g, [c[0] + (s.u[0] * Math.cos(a) + s.v[0] * Math.sin(a)) * R, c[1] + (s.u[1] * Math.cos(a) + s.v[1] * Math.sin(a)) * R, c[2] + (s.u[2] * Math.cos(a) + s.v[2] * Math.sin(a)) * R])
          const q = project(p)
          if (k === 0) path.moveTo(q.x, q.y)
          else path.lineTo(q.x, q.y)
        }
        ctx.strokeStyle = `hsla(${g.hue}, 60%, 75%, ${(dimAll ? 0.05 : 0.11) * strength})`
        ctx.stroke(path)
      }
      // Links between its notes.
      for (const [a, b] of s.links) {
        const on = hover && (a === hover || b === hover)
        const alpha = on ? 0.75 : (dimAll ? 0.04 : 0.14) * strength
        if (alpha < 0.015) continue
        ctx.strokeStyle = on ? hover!.color.replace(/[\d.]+\)$/, `${alpha})`) : `rgba(190, 185, 255, ${alpha})`
        ctx.lineWidth = on ? 1.4 : 1
        ctx.beginPath()
        ctx.moveTo(a.sx, a.sy)
        ctx.lineTo(b.sx, b.sy)
        ctx.stroke()
      }
    }

    // Locked galaxies: a dim dashed outline instead of stars.
    for (const g of galaxies) {
      if (!g.locked || !g.shown) continue
      const path = new Path2D()
      for (let k = 0; k <= 64; k++) {
        const a = (k / 64) * TAU
        const q = project(toWorld(g, [Math.cos(a) * g.R, 0, Math.sin(a) * g.R]))
        if (k === 0) path.moveTo(q.x, q.y)
        else path.lineTo(q.x, q.y)
      }
      ctx.setLineDash([3, 6])
      ctx.strokeStyle = `hsla(${g.hue}, 50%, 70%, 0.35)`
      ctx.stroke(path)
      ctx.setLineDash([])
    }

    // Suns, planets, moons and stars, far ones first.
    const visible = bodies.filter((b) => b.shown)
    visible.sort((a, b) => b.d - a.d)
    for (const b of visible) {
      const on = lit.has(b) || match(b)
      const dim = dimAll && !on
      const tw = reduced || b.role !== 'star' ? 1 : 0.75 + 0.25 * Math.sin(time * b.twinkle + b.phase)
      const px = Math.max(b.role === 'moon' ? 0.45 : 0.7, b.px) * (b === hover ? 1.3 : 1)
      // Tiny far-off lights are dimmed so a whole galaxy seen from afar isn't a white blob.
      const far = clamp(px / 1.6, b.role === 'star' ? 0.34 : 0.22, 1)
      const alpha = (dim ? 0.16 : on ? 1 : 0.85) * fog(b.d) * far * tw
      // Far away: a point of light. The glow shrinks relative to the body as you get closer, so
      // nothing swamps the screen up close.
      const near = clamp((px - 2) / 6, 0, 1)
      const halo = (b.role === 'sun' ? 4.2 : b.role === 'star' ? 3.4 : 3) * (1 - near) + (b.role === 'sun' ? 2.1 : b.role === 'star' ? 2.4 : 1.35) * near
      const rad = px * halo
      ctx.globalAlpha = alpha * (b.role === 'planet' || b.role === 'moon' ? 1 - near * 0.75 : 1)
      ctx.drawImage(glow(b.role === 'sun' ? b.corona : b.color, b.role === 'sun' ? 0.3 : 0.22), b.sx - rad, b.sy - rad, rad * 2, rad * 2)
      if (near > 0 && b.role !== 'star') {
        // Up close: a body you can see. Suns a bright disc; planets and moons shaded spheres,
        // lit from the upper left.
        ctx.globalCompositeOperation = 'source-over'
        ctx.globalAlpha = alpha * near
        const g = ctx.createRadialGradient(b.sx - px * 0.35, b.sy - px * 0.35, px * 0.1, b.sx, b.sy, px)
        if (b.role === 'sun') {
          g.addColorStop(0, 'rgba(255,255,250,1)')
          g.addColorStop(0.7, b.color.replace(/[\d.]+\)$/, '1)'))
          g.addColorStop(1, b.color.replace(/[\d.]+\)$/, '0.9)'))
        } else {
          g.addColorStop(0, b.color.replace(/(\d+)%, [\d.]+\)$/, '88%, 1)'))
          g.addColorStop(0.55, b.color.replace(/[\d.]+\)$/, '1)'))
          g.addColorStop(1, b.color.replace(/(\d+)%, [\d.]+\)$/, '18%, 1)'))
        }
        ctx.fillStyle = g
        ctx.beginPath()
        ctx.arc(b.sx, b.sy, px, 0, TAU)
        ctx.fill()
        ctx.globalCompositeOperation = 'lighter'
      }
      if (b.kind === 'whiteboard' && !dim && px > 1.5) {
        ctx.globalAlpha = 0.5 * fog(b.d)
        ctx.strokeStyle = b.color
        ctx.lineWidth = 1
        ctx.strokeRect(b.sx - px * 1.6, b.sy - px * 1.6, px * 3.2, px * 3.2)
      }
    }
    ctx.globalAlpha = 1
    ctx.globalCompositeOperation = 'source-over'

    // ---- names (each skipped if it would cover one already drawn) ----
    const taken: [number, number, number, number][] = []
    const room = (x: number, y: number, width: number, height: number, force = false) => {
      const box: [number, number, number, number] = [x - width / 2 - 4, y - height, x + width / 2 + 4, y + 4]
      if (!force && taken.some((t) => box[0] < t[2] && box[2] > t[0] && box[1] < t[3] && box[3] > t[1])) return false
      taken.push(box)
      return true
    }
    ctx.lineJoin = 'round'
    // The hovered note first, so it always gets its place.
    if (hover?.shown) room(hover.sx, hover.sy - Math.max(hover.px * 1.5, 5) - 8, ctx.measureText(hover.title).width + 20, 30, true)
    ctx.textAlign = 'center'
    // Galaxies, when you can see them as a whole.
    ctx.font = '600 11px Inter, system-ui, sans-serif'
    for (const g of galaxies) {
      if (!g.shown || g.Rpx > Math.min(w, h) * 0.55) continue
      const y = g.sy + Math.max(18, g.Rpx * 0.45) + 14
      if (!room(g.sx, y + 16, Math.max(g.name.length * 10, 150), 30)) continue
      ctx.globalAlpha = clamp(fog(g.d) * 1.2, 0.35, 0.95)
      ctx.lineWidth = 4
      ctx.strokeStyle = 'rgba(3,2,10,0.85)'
      ctx.fillStyle = `hsl(${g.hue} 70% 84%)`
      spacedText(ctx, g.name.toUpperCase(), g.sx, y, 2.4)
      ctx.font = '500 10px Inter, system-ui, sans-serif'
      ctx.fillStyle = 'rgba(200,196,235,0.7)'
      const sub = g.locked ? 'Locked · encrypted' : g.encrypted ? 'Encrypted' : `${g.systems.length} ${g.systems.length === 1 ? 'system' : 'systems'} · ${g.stars.length} lone ${g.stars.length === 1 ? 'star' : 'stars'}`
      ctx.strokeText(sub, g.sx, y + 15)
      ctx.fillText(sub, g.sx, y + 15)
      ctx.font = '600 11px Inter, system-ui, sans-serif'
    }
    // Solar systems, named after their sun, once you're in their galaxy.
    if (focus.kind !== 'all' || galaxies.length === 1) {
      const named = systems.filter((s) => s.shown && (s.extPx > 34 || s === hoverSystem)).sort((a, b) => Number(b === hoverSystem) - Number(a === hoverSystem) || b.extPx - a.extPx).slice(0, 14)
      ctx.font = '600 11px Inter, system-ui, sans-serif'
      for (const s of named) {
        const y = s.sy - Math.max(s.sun.px * 1.6, 8) - 10
        ctx.globalAlpha = (s === hoverSystem ? 1 : 0.8) * fog(s.sun.d)
        ctx.lineWidth = 4
        ctx.strokeStyle = 'rgba(3,2,10,0.85)'
        ctx.fillStyle = s === hoverSystem ? '#ffffff' : 'rgba(255, 236, 200, 0.9)'
        const text = s.sun.title.length > 40 ? s.sun.title.slice(0, 39) + '…' : s.sun.title
        if (!room(s.sx, y, ctx.measureText(text).width, 13, s === hoverSystem)) continue
        ctx.strokeText(text, s.sx, y)
        ctx.fillText(text, s.sx, y)
      }
    }
    // Every planet and moon of a system you're up close to; hover and search results.
    ctx.font = '500 11px Inter, system-ui, sans-serif'
    const close = focus.kind === 'system' && focus.s.extPx > 160 ? focus.s.bodies.filter((b) => b.role !== 'sun' && b.shown) : []
    const labelled = new Set<Body>([...close, ...(hover ? [hover] : []), ...(hover ? [...lit] : []), ...(q_ ? visible.filter(match).slice(0, 60) : [])])
    for (const b of labelled) {
      if (!b.shown || (b.role === 'sun' && b !== hover && !match(b))) continue
      const text = b.title.length > 48 ? b.title.slice(0, 47) + '…' : b.title
      const y = b.sy - Math.max(b.px * 1.5, 5) - 8
      if (b !== hover && !room(b.sx, y, ctx.measureText(text).width, 13)) continue
      ctx.globalAlpha = b === hover ? 1 : 0.85 * fog(b.d)
      ctx.lineWidth = 4
      ctx.strokeStyle = 'rgba(3,2,10,0.9)'
      ctx.fillStyle = b === hover ? '#ffffff' : 'rgba(225,222,250,0.9)'
      ctx.strokeText(text, b.sx, y)
      ctx.fillText(text, b.sx, y)
      if (b === hover) {
        const what =
          b.role === 'sun'
            ? `Sun · ${b.system!.bodies.length - 1} linked ${b.system!.bodies.length === 2 ? 'note' : 'notes'}`
            : b.role === 'planet'
              ? `Planet of ${b.parent!.title}`
              : b.role === 'moon'
                ? `Moon of ${b.parent!.title}`
                : 'Lone star · not linked yet'
        ctx.font = '500 10px Inter, system-ui, sans-serif'
        ctx.fillStyle = 'rgba(200,196,235,0.75)'
        ctx.strokeText(what, b.sx, y - 14)
        ctx.fillText(what, b.sx, y - 14)
        ctx.font = '500 11px Inter, system-ui, sans-serif'
      }
    }
    ctx.globalAlpha = 1
  }

  // ---- input: drag or scroll to turn, Ctrl + scroll / pinch to zoom, right-drag to slide ----
  let bg: CanvasGradient | string = '#03020a'
  const pos = (e: MouseEvent) => {
    const r = el.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }
  const bodyAt = (p: { x: number; y: number }) => {
    let best: Body | null = null
    let bd = Infinity
    for (const b of bodies) {
      if (!b.shown) continue
      const dist = Math.hypot(b.sx - p.x, b.sy - p.y)
      if (dist > Math.max(7, b.px * 2.4)) continue
      const score = dist + b.d * 0.0001 // prefer the nearer one
      if (score < bd) {
        best = b
        bd = score
      }
    }
    return best
  }
  const systemAt = (p: { x: number; y: number }) => {
    let best: System | null = null
    let bd = Infinity
    for (const s of systems) {
      if (!s.shown) continue
      const dist = Math.hypot(s.sx - p.x, s.sy - p.y)
      if (dist <= Math.max(16, s.extPx) && dist < bd) {
        best = s
        bd = dist
      }
    }
    return best
  }
  const galaxyAt = (p: { x: number; y: number }) => {
    let best: Galaxy | null = null
    let bd = Infinity
    for (const g of galaxies) {
      if (!g.shown) continue
      const dist = Math.hypot(g.sx - p.x, g.sy - p.y)
      if (dist <= Math.max(30, g.Rpx * 0.8) && dist < bd) {
        best = g
        bd = dist
      }
    }
    return best
  }
  const turn = (dx: number, dy: number) => {
    goal.yaw += dx * 0.007
    goal.pitch = clamp(goal.pitch + dy * 0.005, -1.35, 1.35)
    idleSince = performance.now()
  }
  const slide = (dx: number, dy: number) => {
    const k = view.dist / focal
    const r = right()
    const u = up()
    goal.target = add(goal.target, add(scale(r, -dx * k), scale(u, dy * k)))
    view.target = add(view.target, add(scale(r, -dx * k), scale(u, dy * k)))
    follow = false
    idleSince = performance.now()
  }
  let drag: { x: number; y: number; moved: boolean; slide: boolean } | null = null
  const onDown = (e: MouseEvent) => {
    if (e.button !== 0 && e.button !== 2) return
    const p = pos(e)
    drag = { x: p.x, y: p.y, moved: false, slide: e.button === 2 || e.shiftKey }
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
      if (drag.slide) slide(dx, dy)
      else {
        turn(dx, dy)
        view.yaw = goal.yaw
        view.pitch = goal.pitch
      }
      return
    }
    if (e.target !== el) return
    const b = bodyAt(p)
    hover = b
    hoverSystem = b ? b.system : focus.kind !== 'all' || galaxies.length === 1 ? systemAt(p) : null
    el.style.cursor = b || hoverSystem || (focus.kind === 'all' && galaxyAt(p)) ? 'pointer' : ''
  }
  const onUp = (e: MouseEvent) => {
    if (drag && !drag.moved && e.button === 0) {
      const p = pos(e)
      const b = bodyAt(p)
      // A click inside a system that's still small on screen flies there first (its planets are
      // too close together to pick out); once you're there, clicking a body opens it.
      const inGalaxy = focus.kind !== 'all' || galaxies.length === 1
      const s = inGalaxy ? systemAt(p) : null
      const smallSystem = s && !(focus.kind === 'system' && focus.s === s) && s.extPx < 90
      if (smallSystem) setFocus({ kind: 'system', s }, true)
      else if (b) onOpen(b.id)
      else if (!inGalaxy) {
        const g = galaxyAt(p)
        if (g) setFocus({ kind: 'galaxy', g }, true)
      } else if (s && !(focus.kind === 'system' && focus.s === s)) setFocus({ kind: 'system', s }, true)
    }
    drag = null
    el.classList.remove('dragging')
  }
  const onDouble = (e: MouseEvent) => {
    if (!bodyAt(pos(e))) backOut()
  }
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && !(e.target instanceof HTMLInputElement)) backOut()
  }
  const onContext = (e: Event) => e.preventDefault()
  const zoom = (factor: number) => {
    goal.dist = clamp(goal.dist * factor, minDist, maxDist)
    idleSince = performance.now()
  }
  const onWheel = (e: WheelEvent) => {
    e.preventDefault()
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1
    if (e.ctrlKey || e.metaKey) zoom(Math.exp(clamp(e.deltaY * unit, -40, 40) * 0.006))
    else turn(-e.deltaX * unit * 0.5, -e.deltaY * unit * 0.5)
  }
  let gestureScale = 1
  const onGestureStart = (e: Event) => {
    e.preventDefault()
    gestureScale = 1
  }
  const onGestureChange = (e: Event) => {
    e.preventDefault()
    const g = e as Event & { scale: number }
    zoom(gestureScale / g.scale)
    gestureScale = g.scale
  }

  const resize = () => {
    dpr = Math.min(window.devicePixelRatio || 1, 1.5)
    w = el.clientWidth
    h = el.clientHeight
    el.width = Math.round(w * dpr)
    el.height = Math.round(h * dpr)
    const g = ctx.createRadialGradient(w / 2, h / 2, 0, w / 2, h / 2, Math.max(w, h) * 0.8)
    g.addColorStop(0, '#0a0820')
    g.addColorStop(1, '#020108')
    bg = g
  }
  const ro = new ResizeObserver(resize)
  ro.observe(el)
  resize()
  el.addEventListener('mousedown', onDown)
  el.addEventListener('dblclick', onDouble)
  el.addEventListener('contextmenu', onContext)
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
  window.addEventListener('keydown', onKey)
  el.addEventListener('wheel', onWheel, { passive: false })
  el.addEventListener('gesturestart', onGestureStart)
  el.addEventListener('gesturechange', onGestureChange)

  // Arrive from the wide view, gliding into the current vault's galaxy.
  place()
  if (galaxies.length) setFocus({ kind: 'galaxy', g: galaxies[0] })
  if (reduced) Object.assign(view, { ...goal, target: [...goal.target] })
  raf = requestAnimationFrame(frame)

  return {
    destroy() {
      cancelAnimationFrame(raf)
      ro.disconnect()
      el.removeEventListener('mousedown', onDown)
      el.removeEventListener('dblclick', onDouble)
      el.removeEventListener('contextmenu', onContext)
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      window.removeEventListener('keydown', onKey)
      el.removeEventListener('wheel', onWheel)
      el.removeEventListener('gesturestart', onGestureStart)
      el.removeEventListener('gesturechange', onGestureChange)
    },
    flyTo(galaxyId) {
      const g = galaxies.find((x) => x.id === galaxyId)
      setFocus(g ? { kind: 'galaxy', g } : { kind: 'all' })
    },
    setQuery(q) {
      query = q
    },
    benchmark(frames = 60, panning = false) {
      const start = performance.now()
      for (let i = 0; i < frames; i++) {
        if (panning) view.yaw += 0.01
        place()
        paint()
      }
      return (performance.now() - start) / frames
    },
  }
}
