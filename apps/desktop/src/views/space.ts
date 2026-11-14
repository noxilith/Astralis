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
  /** Flies to the system with the most planets (for checking performance). */
  focusBiggestSystem(): void
  /** What the view is doing right now (for checking it works). */
  debugState(): { hover: string | null; motion: number; time: number }
  /** Moves the camera closer (k < 1) or farther, at once (for checking performance). */
  zoomBy(k: number): void
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
  /** The plane it orbits in (planets and moons each have their own tilt). */
  ou: V3
  ov: V3
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
const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
/** Rotates `x` about the unit axis `k` by `t` radians (Rodrigues). */
const rotate = (x: V3, k: V3, t: number): V3 => {
  const c = Math.cos(t)
  const s = Math.sin(t)
  return add(add(scale(x, c), scale(cross(k, x), s)), scale(k, dot(k, x) * (1 - c)))
}
/** A plane (u, v) tipped by `inclination` about a line through it at angle `node`. */
const tilt = (u: V3, v: V3, inclination: number, node: number): [V3, V3] => {
  const k = norm(add(scale(u, Math.cos(node)), scale(v, Math.sin(node))))
  return [rotate(u, k, inclination), rotate(v, k, inclination)]
}
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
function glow(rawColor: string, core = 0.22) {
  const color = quant(rawColor)
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


// ---- bodies drawn as what they are: suns, planets, moons and four-point stars ----

const spriteCache = new Map<string, HTMLCanvasElement>()
function sprite(key: string, size: number, draw: (x: CanvasRenderingContext2D, size: number) => void) {
  let c = spriteCache.get(key)
  if (c) return c
  if (spriteCache.size > 500) spriteCache.clear()
  c = document.createElement('canvas')
  c.width = c.height = size
  draw(c.getContext('2d')!, size)
  spriteCache.set(key, c)
  return c
}

/** [hue, saturation, lightness] of an "hsla(…)" colour. */
function hslOf(color: string): [number, number, number] {
  const m = /hsla?\(\s*([\d.]+)\s*,\s*([\d.]+)%\s*,\s*([\d.]+)%/.exec(color)
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : [220, 50, 70]
}
const hsl = (h: number, s: number, l: number, a = 1) => `hsla(${h}, ${clamp(s, 0, 100)}%, ${clamp(l, 0, 100)}%, ${a})`

/** A colour rounded to a few steps of hue, saturation and lightness, so bodies whose colours differ
 *  by a hair share one cached picture. (Every lone star has its own tiny variation; without this
 *  each one made its own picture, thousands of them, over and over.) */
function quant(color: string): string {
  const [h, s, l] = hslOf(color)
  const a = /,\s*([\d.]+)\)\s*$/.exec(color)?.[1] ?? '1'
  return `hsla(${Math.round(h / 8) * 8}, ${Math.round(s / 6) * 6}%, ${Math.round(l / 6) * 6}%, ${a})`
}

/** A small deterministic random sequence, so a planet looks the same every time. */
function seeded(seed: number) {
  let t = (seed * 2654435761) >>> 0
  return () => {
    t = (t + 0x6d2b79f5) >>> 0
    let r = Math.imul(t ^ (t >>> 15), 1 | t)
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296
  }
}

/** The four-point star from the logo (and the assistant's icon): thin points, curved sides. */
const STAR_PATH = 'M627 545 C634 612 646 646 729 653 C646 660 634 694 627 765 C620 694 608 660 525 653 C608 646 620 612 627 545 Z'

/** A glowing four-point star in `color`, the logo's star: a white-hot heart, the star itself with
 *  thin curved points, faint diffraction lines running through it, a small sparkle turned 45°, and
 *  a soft bloom in the star's own colour. The points reach the sprite's edge. */
function starSprite(rawColor: string, size = 192) {
  const color = quant(rawColor)
  return sprite(`star|${size}|${color}`, size, (x, S) => {
    const u = S / 192
    const [h, sat] = hslOf(color)
    const c = S / 2
    x.globalCompositeOperation = 'lighter'
    // Bloom: tight and bright in the middle, tinted, fading well before the edge.
    const bloom = x.createRadialGradient(c, c, 0, c, c, c * 0.62)
    bloom.addColorStop(0, hsl(h, sat, 92, 0.75))
    bloom.addColorStop(0.1, hsl(h, sat, 82, 0.4))
    bloom.addColorStop(0.35, hsl(h, sat, 68, 0.12))
    bloom.addColorStop(1, hsl(h, sat, 60, 0))
    x.fillStyle = bloom
    x.fillRect(0, 0, S, S)
    // Diffraction lines: very thin, long, fading to nothing.
    for (const rot of [0, Math.PI / 2]) {
      x.save()
      x.translate(c, c)
      x.rotate(rot)
      const line = x.createLinearGradient(-c, 0, c, 0)
      line.addColorStop(0, hsl(h, sat, 85, 0))
      line.addColorStop(0.5, hsl(h, sat, 92, 0.75))
      line.addColorStop(1, hsl(h, sat, 85, 0))
      x.fillStyle = line
      x.fillRect(-c, -0.9 * Math.max(u, 0.6), S, 1.8 * Math.max(u, 0.6))
      x.restore()
    }
    const shape = (scale: number, rot: number, fill: string | CanvasGradient, blur: number) => {
      x.save()
      x.translate(c, c)
      x.rotate(rot)
      const k = (c * 0.98 * scale) / 112
      x.scale(k, k)
      x.translate(-627, -653)
      x.shadowColor = hsl(h, 100, 80, 0.95)
      x.shadowBlur = (blur * u) / k
      x.fillStyle = fill
      x.fill(new Path2D(STAR_PATH))
      x.restore()
    }
    // The sparkle between the points.
    shape(0.5, Math.PI / 4, hsl(h, sat, 90, 0.6), 8)
    // The star: white at the heart, tinted toward the tips.
    const body = x.createRadialGradient(c, c, 0, c, c, c * 0.7)
    body.addColorStop(0, 'rgba(255,255,255,1)')
    body.addColorStop(0.28, hsl(h, sat, 88, 1))
    body.addColorStop(1, hsl(h, sat, 68, 0.95))
    shape(0.74, 0, body, 14)
    // A white-hot heart.
    const heart = x.createRadialGradient(c, c, 0, c, c, c * 0.15)
    heart.addColorStop(0, 'rgba(255,255,255,1)')
    heart.addColorStop(0.5, 'rgba(255,255,255,0.7)')
    heart.addColorStop(1, 'rgba(255,255,255,0)')
    x.fillStyle = heart
    x.fillRect(0, 0, S, S)
  })
}

/** The glow around a sun (the sun itself is drawn as a lit-up 3D sphere). `SUN_DISC` is the disc's
 *  radius as a fraction of the sprite's half-width, so the corona has room around it. */
const SUN_DISC = 36 / 128
function coronaSprite(rawColor: string) {
  const color = quant(rawColor)
  return sprite('corona|' + color, 256, (x, S) => {
    const [h] = hslOf(color)
    const c = S / 2
    const R = (S * SUN_DISC) / 2
    const g = x.createRadialGradient(c, c, R * 0.7, c, c, c)
    g.addColorStop(0, hsl(h, 100, 72, 0.62))
    g.addColorStop(0.16, hsl(h, 100, 64, 0.32))
    g.addColorStop(0.5, hsl(h - 6, 100, 55, 0.09))
    g.addColorStop(1, hsl(h, 100, 50, 0))
    x.fillStyle = g
    x.fillRect(0, 0, S, S)
  })
}

// ---- planets, moons and suns as real spheres ----
// Every body has a surface texture wrapped around it that turns with the body (slowly) and stays
// put when the camera moves, so orbiting the view shows different sides. The night side follows
// the sun. The texture is made once per look; each frame only the sphere is shaded.

const hash3 = (ix: number, iy: number, iz: number, seed: number) => {
  let h = (Math.imul(ix, 374761393) + Math.imul(iy, 668265263) + Math.imul(iz, 2147483647) + Math.imul(seed, 1274126177)) | 0
  h = Math.imul(h ^ (h >>> 13), 1274126177)
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295
}
function noise3(x: number, y: number, z: number, seed: number) {
  const ix = Math.floor(x), iy = Math.floor(y), iz = Math.floor(z)
  const fx = x - ix, fy = y - iy, fz = z - iz
  const u = fx * fx * (3 - 2 * fx), v = fy * fy * (3 - 2 * fy), w = fz * fz * (3 - 2 * fz)
  const l = (a: number, b: number, t: number) => a + (b - a) * t
  const c = (dx: number, dy: number, dz: number) => hash3(ix + dx, iy + dy, iz + dz, seed)
  return l(l(l(c(0, 0, 0), c(1, 0, 0), u), l(c(0, 1, 0), c(1, 1, 0), u), v), l(l(c(0, 0, 1), c(1, 0, 1), u), l(c(0, 1, 1), c(1, 1, 1), u), v), w)
}
function fbm3(x: number, y: number, z: number, seed: number, octaves = 4) {
  let amp = 0.5, f = 1, sum = 0, norm = 0
  for (let i = 0; i < octaves; i++) {
    sum += amp * noise3(x * f, y * f, z * f, seed + i * 31)
    norm += amp
    f *= 2
    amp *= 0.5
  }
  return sum / norm
}
const smooth = (a: number, b: number, x: number) => {
  const t = clamp((x - a) / (b - a), 0, 1)
  return t * t * (3 - 2 * t)
}
function hslToRgb(h: number, s: number, l: number): V3 {
  h = ((h % 360) + 360) % 360
  s = clamp(s, 0, 100) / 100
  l = clamp(l, 0, 100) / 100
  const k = (n: number) => (n + h / 30) % 12
  const a = s * Math.min(l, 1 - l)
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)))
  return [f(0) * 255, f(8) * 255, f(4) * 255]
}

type Kind = 'rocky' | 'banded' | 'cloudy' | 'moon' | 'sun'
const KINDS: Kind[] = ['rocky', 'banded', 'cloudy', 'moon', 'sun']
const SEEDS = 12
/** A surface pattern in three channels (main, detail, extra), the same for every colour: a body's
 *  own colours are applied when it's shaded. So there are only a few dozen of these, made once. */
interface Texture {
  w: number
  h: number
  data: Uint8Array
}
const TEX_W = 384
const TEX_H = 192
const textures = new Map<string, Texture>()

function makeTexture(kind: Kind, seed: number): Texture {
  const key = `${kind}|${seed}`
  const hit = textures.get(key)
  if (hit) return hit
  const data = new Uint8Array(TEX_W * TEX_H * 3)
  const craters: [V3, number][] = []
  if (kind === 'moon') {
    const r = seeded(seed * 17 + 3)
    for (let i = 0; i < 38; i++) {
      const z = r() * 2 - 1
      const a = r() * TAU
      const q = Math.sqrt(1 - z * z)
      craters.push([[q * Math.cos(a), z, q * Math.sin(a)], 0.05 + r() * r() * 0.2])
    }
  }
  const stormLat = -0.35 + (hash3(seed, 1, 2, 3) - 0.5) * 0.4
  const bands = 6 + (seed % 5)
  for (let j = 0; j < TEX_H; j++) {
    const lat = (0.5 - (j + 0.5) / TEX_H) * Math.PI
    const cl = Math.cos(lat)
    const py = Math.sin(lat)
    for (let i = 0; i < TEX_W; i++) {
      const lon = ((i + 0.5) / TEX_W - 0.5) * TAU
      const px = cl * Math.cos(lon)
      const pz = cl * Math.sin(lon)
      let t = 0
      let d = 0
      let e = 0
      if (kind === 'rocky') {
        t = smooth(0.47, 0.53, fbm3(px * 2.3 + 5, py * 2.3, pz * 2.3, seed))
        d = fbm3(px * 9, py * 9, pz * 9, seed + 7, 3)
        e = smooth(1.12, 1.35, Math.abs(lat) + (fbm3(px * 4, py * 4, pz * 4, seed + 2) - 0.5) * 0.5)
      } else if (kind === 'banded') {
        const warp = (fbm3(px * 3, py * 3, pz * 3, seed) - 0.5) * 2.2
        t = Math.sin((lat + warp * 0.12) * bands * 1.4 + warp) * 0.5 + 0.5
        d = fbm3(px * 14, py * 40, pz * 14, seed + 5, 3)
        const dl = Math.atan2(Math.sin(lon - 1), Math.cos(lon - 1))
        const q = (dl / 0.35) ** 2 + ((lat - stormLat) / 0.14) ** 2
        e = q < 1 ? 1 - q : 0
      } else if (kind === 'cloudy') {
        t = smooth(0.46, 0.72, fbm3(px * 3.4 + 2, py * 5.4, pz * 3.4, seed, 5))
        d = fbm3(px * 3, py * 3, pz * 3, seed + 9)
      } else if (kind === 'moon') {
        let v = 0.45 + (fbm3(px * 4, py * 4, pz * 4, seed, 5) - 0.5) * 0.7
        for (const [c, rad] of craters) {
          const dd = Math.acos(clamp(px * c[0] + py * c[1] + pz * c[2], -1, 1)) / rad
          if (dd < 1) v -= (1 - dd * dd) * 0.3
          else if (dd < 1.25) v += (1.25 - dd) * 0.4
        }
        t = clamp(v, 0, 1)
      } else {
        t = fbm3(px * 9, py * 9, pz * 9, seed, 4)
        e = smooth(0.6, 0.78, fbm3(px * 2.4 + 9, py * 2.4, pz * 2.4, seed + 4, 3))
      }
      const o = (j * TEX_W + i) * 3
      data[o] = t * 255
      data[o + 1] = d * 255
      data[o + 2] = e * 255
    }
  }
  const tex = { w: TEX_W, h: TEX_H, data }
  textures.set(key, tex)
  return tex
}

/** Makes the textures a little at a time while the browser is idle, so none is made in the middle
 *  of a frame (which shows as a stutter). */
let warming = false
function warmTextures() {
  if (warming) return
  warming = true
  const todo = KINDS.flatMap((k) => Array.from({ length: SEEDS }, (_, i) => [k, i] as const)).filter(([k, i]) => !textures.has(`${k}|${i}`))
  const step = () => {
    const next = todo.shift()
    if (!next) return
    makeTexture(next[0], next[1])
    setTimeout(step, 40)
  }
  setTimeout(step, 400)
}

/** A body's colours, from its own colour. */
interface Palette {
  a: V3
  b: V3
  c: V3
  d: V3
}
const palettes = new Map<string, Palette>()
function paletteOf(kind: Kind, rawColor: string): Palette {
  const color = quant(rawColor)
  const key = kind + '|' + color
  let p = palettes.get(key)
  if (p) return p
  if (palettes.size > 600) palettes.clear()
  const [h, s, l] = hslOf(color)
  if (kind === 'rocky') p = { a: hslToRgb(h, s * 0.9, l - 20), b: hslToRgb(h + 28, s * 0.55, l - 6), c: [236, 242, 250], d: [0, 0, 0] }
  else if (kind === 'banded') p = { a: hslToRgb(h + 14, s, l - 15), b: hslToRgb(h - 10, s * 0.8, l + 14), c: hslToRgb(h - 20, s, l + 4), d: [0, 0, 0] }
  else if (kind === 'cloudy') p = { a: hslToRgb(h, s * 0.9, l - 8), b: [250, 252, 255], c: [0, 0, 0], d: [0, 0, 0] }
  else if (kind === 'moon') p = { a: hslToRgb(h, 9, l - 32), b: hslToRgb(h, 9, l + 16), c: [0, 0, 0], d: [0, 0, 0] }
  else p = { a: hslToRgb(h + 4, 100, 70), b: hslToRgb(h - 10, 100, 90), c: hslToRgb(h - 12, 95, 58), d: [0, 0, 0] }
  palettes.set(key, p)
  return p
}

/** For one size of sphere: where each pixel sits on it (and how near its edge), worked out once. */
interface Grid {
  x: Float32Array
  y: Float32Array
  z: Float32Array
  fres: Float32Array
  alpha: Uint8Array
}
const grids = new Map<number, Grid>()
function gridOf(n: number): Grid {
  let g = grids.get(n)
  if (g) return g
  g = { x: new Float32Array(n * n), y: new Float32Array(n * n), z: new Float32Array(n * n), fres: new Float32Array(n * n), alpha: new Uint8Array(n * n) }
  const half = n / 2
  for (let j = 0; j < n; j++) {
    for (let i = 0; i < n; i++) {
      const u = (i + 0.5 - half) / half
      const v = (j + 0.5 - half) / half
      const r2 = u * u + v * v
      const k = j * n + i
      const z = Math.sqrt(Math.max(0, 1 - r2))
      g.x[k] = u
      g.y[k] = -v
      g.z[k] = z
      g.fres[k] = Math.pow(1 - z, 3)
      g.alpha[k] = r2 >= 1.04 ? 0 : r2 <= 1 ? 255 : ((1.04 - r2) / 0.04) * 255
    }
  }
  grids.set(n, g)
  return g
}

/** The canvas each body is shaded into: one per body, never shared. (A canvas drawn from more than
 *  once in a frame can be read after it's been overwritten, which showed every body of the same
 *  size with the last one's surface, so textures seemed to change and swap places.) */
const sphereCanvases = new Map<string, { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D; image: ImageData; n: number; used: number; shaded: number }>()
let sphereFrame = 0
function sphereTarget(key: string, n: number) {
  let t = sphereCanvases.get(key)
  if (!t || t.n !== n) {
    const canvas = t?.canvas ?? document.createElement('canvas')
    canvas.width = canvas.height = n
    const ctx = canvas.getContext('2d')!
    t = { canvas, ctx, image: ctx.createImageData(n, n), n, used: sphereFrame, shaded: -999 }
    sphereCanvases.set(key, t)
  }
  t.used = sphereFrame
  return t
}
/** Called once a frame: forgets the canvases of bodies that haven't been drawn for a while. */
function sweepSpheres() {
  sphereFrame++
  if (sphereCanvases.size < 200 || sphereFrame % 120) return
  for (const [k, t] of sphereCanvases) if (sphereFrame - t.used > 240) sphereCanvases.delete(k)
}

const KIND_CODE: Record<Kind, number> = { rocky: 0, banded: 1, cloudy: 2, moon: 3, sun: 4 }

/** Shades a textured sphere of `n` x `n` pixels. `B` turns a view-space direction into the body's
 *  own texture frame; `L` points toward the sun, in view space; `rim` is the colour of the thin
 *  atmosphere (or the sun's glowing edge). */
function shadeSphere(id: string, n: number, kind: Kind, tex: Texture, pal: Palette, B: number[], L: V3, lit: boolean, rim: V3) {
  const t = sphereTarget(id, n)
  const out = t.image.data
  const g = gridOf(n)
  const { w: tw, h: th, data: td } = tex
  const code = KIND_CODE[kind]
  const [aR, aG, aB] = pal.a
  const [bR, bG, bB] = pal.b
  const [cR, cG, cB] = pal.c
  const b0 = B[0], b1 = B[1], b2 = B[2], b3 = B[3], b4 = B[4], b5 = B[5], b6 = B[6], b7 = B[7], b8 = B[8]
  const total = n * n
  for (let k = 0; k < total; k++) {
    const o = k * 4
    const al = g.alpha[k]
    if (al === 0) {
      out[o + 3] = 0
      continue
    }
    const X = g.x[k], Y = g.y[k], Z = g.z[k]
    const lx = b0 * X + b3 * Y + b6 * Z
    const ly = b1 * X + b4 * Y + b7 * Z
    const lz = b2 * X + b5 * Y + b8 * Z
    const fu = (Math.atan2(lz, lx) / TAU + 0.5) * tw - 0.5
    const fv = (0.5 - Math.asin(ly < -1 ? -1 : ly > 1 ? 1 : ly) / Math.PI) * th - 0.5
    const x0 = Math.floor(fu)
    const y0 = Math.floor(fv)
    const ax = fu - x0
    const ay = fv - y0 < 0 ? 0 : fv - y0
    const xa = ((x0 % tw) + tw) % tw
    const xb = xa + 1 === tw ? 0 : xa + 1
    const ya = y0 < 0 ? 0 : y0 > th - 1 ? th - 1 : y0
    const yb = y0 + 1 > th - 1 ? th - 1 : y0 + 1
    const i00 = (ya * tw + xa) * 3, i10 = (ya * tw + xb) * 3, i01 = (yb * tw + xa) * 3, i11 = (yb * tw + xb) * 3
    const w00 = (1 - ax) * (1 - ay), w10 = ax * (1 - ay), w01 = (1 - ax) * ay, w11 = ax * ay
    const T = (td[i00] * w00 + td[i10] * w10 + td[i01] * w01 + td[i11] * w11) / 255
    const D = (td[i00 + 1] * w00 + td[i10 + 1] * w10 + td[i01 + 1] * w01 + td[i11 + 1] * w11) / 255
    const E = (td[i00 + 2] * w00 + td[i10 + 2] * w10 + td[i01 + 2] * w01 + td[i11 + 2] * w11) / 255
    let R: number, G: number, Bc: number
    if (code === 0) {
      // Rocky: sea and land, with ice at the poles.
      const f = 0.88 + 0.3 * D
      R = aR + (bR * f - aR) * T
      G = aG + (bG * f - aG) * T
      Bc = aB + (bB * f - aB) * T
      R += (cR - R) * E
      G += (cG - G) * E
      Bc += (cB - Bc) * E
    } else if (code === 1) {
      // Banded, with a storm.
      let u = T * 0.8 + D * 0.3
      u = u < 0 ? 0 : u > 1 ? 1 : u
      R = aR + (bR - aR) * u
      G = aG + (bG - aG) * u
      Bc = aB + (bB - aB) * u
      const s = E * 0.8
      R += (cR - R) * s
      G += (cG - G) * s
      Bc += (cB - Bc) * s
    } else if (code === 2) {
      // Clouds over a calm surface.
      const f = 0.9 + 0.2 * D
      const c = T * 0.85
      R = aR * f + (bR - aR * f) * c
      G = aG * f + (bG - aG * f) * c
      Bc = aB * f + (bB - aB * f) * c
    } else if (code === 3) {
      R = aR + (bR - aR) * T
      G = aG + (bG - aG) * T
      Bc = aB + (bB - aB) * T
    } else {
      // A sun: granulation, with cooler spots.
      let u = T * 1.5 - 0.2
      u = u < 0 ? 0 : u > 1 ? 1 : u
      R = aR + (bR - aR) * u
      G = aG + (bG - aG) * u
      Bc = aB + (bB - aB) * u
      const s = E * 0.65
      R += (cR - R) * s
      G += (cG - G) * s
      Bc += (cB - Bc) * s
    }
    const fres = g.fres[k]
    if (lit) {
      const nl = X * L[0] + Y * L[1] + Z * L[2]
      const day = smooth(-0.12, 0.55, nl)
      const shade = 0.13 + 0.87 * day
      const glow = fres * (0.25 + 0.75 * smooth(-0.35, 0.5, nl)) * 0.9
      R = R * shade + rim[0] * glow
      G = G * shade + rim[1] * glow
      Bc = Bc * shade + rim[2] * glow
    } else {
      // A sun: darker toward its edge, which glows.
      const limb = 0.6 + 0.4 * Math.sqrt(Math.sqrt(Z))
      R = R * limb + rim[0] * fres * 0.5
      G = G * limb + rim[1] * fres * 0.5
      Bc = Bc * limb + rim[2] * fres * 0.5
    }
    out[o] = R
    out[o + 1] = G
    out[o + 2] = Bc
    out[o + 3] = al
  }
  t.ctx.putImageData(t.image, 0, 0)
  t.shaded = sphereFrame
  return t.canvas
}

/** A small lit ball for planets and moons too far away to show a surface (lit from the upper left). */
const DISC_RADIUS = 0.8
function discSprite(rawColor: string, moon: boolean) {
  const color = quant(rawColor)
  return sprite(`disc|${color}|${moon}`, 48, (x, S) => {
    const [h, sat, lit] = hslOf(color)
    const c = S / 2
    const R = c * DISC_RADIUS
    const g = x.createRadialGradient(c - R * 0.4, c - R * 0.4, R * 0.05, c, c, R * 1.1)
    g.addColorStop(0, hsl(h, moon ? 10 : sat, lit + 22))
    g.addColorStop(0.55, hsl(h, moon ? 10 : sat, lit - 6))
    g.addColorStop(1, hsl(h, moon ? 10 : sat, lit - 34))
    x.fillStyle = g
    x.beginPath()
    x.arc(c, c, R, 0, TAU)
    x.fill()
  })
}

/** Which look a planet has: rocky, banded, ringed, or cloudy. */
const planetVariant = (b: { id: string }) => Math.floor(rng(b.id + ':look')() * 6)
const PLANET_KIND: Kind[] = ['rocky', 'banded', 'banded', 'banded', 'cloudy', 'cloudy']
const PLANET_RINGED = [false, false, false, true, true, false]

const sphereSteps = new WeakMap<object, number>()

/** A system with more planets than this shows no orbit lines. */
const CROWDED = 8
const planetCounts = new WeakMap<object, number>()
const planetCount = (s: { bodies: { role: Role }[] }) => {
  let n = planetCounts.get(s)
  if (n === undefined) {
    n = s.bodies.filter((b) => b.role === 'planet').length
    planetCounts.set(s, n)
  }
  return n
}

/** Each body's slow spin and the tilt of its axis (the same every time). */
const spins = new Map<string, { tilt: number; rate: number; seed: number; offset: number }>()
function spinOf(b: { id: string; role: Role }) {
  let sp = spins.get(b.id)
  if (!sp) {
    const r = rng(b.id + ':spin')
    sp = { tilt: (r() - 0.5) * (b.role === 'sun' ? 0.3 : 0.9), rate: (b.role === 'sun' ? 0.05 : 0.12) + r() * 0.16, seed: Math.floor(r() * 12), offset: r() * TAU }
    spins.set(b.id, sp)
  }
  return sp
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
    // Everything about a note's look comes from its own id, never from the order things are made
    // in, so a planet stays the same planet when other notes change.
    phase: rng(nd.id + ':phase')() * TAU,
    speed: 0,
    home: [0, 0, 0],
    ou: [1, 0, 0],
    ov: [0, 0, 1],
    twinkle: 0.6 + rng(nd.id + ':twinkle')() * 1.8,
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
    sun.color = `hsla(${28 + rng(sun.id + ':sun')() * 26}, 100%, 72%, 0.95)`
    sun.corona = th0 === null ? sun.color : `hsla(${th0}, 85%, 70%, 0.9)`
    // Moons orbit their planet; a planet's moon cloud decides how much room it needs.
    const moonsOf = new Map<Body, Body[]>()
    for (const m of moons) moonsOf.set(m.parent!, [...(moonsOf.get(m.parent!) ?? []), m])
    const reach = new Map<Body, number>()
    for (const p of planets) {
      p.r = 1.3 + Math.sqrt(p.degree) * 0.38 + (p.pinned ? 0.5 : 0)
      const h = tagHue(p) ?? hue + (rng(p.id + ':hue')() - 0.5) * 70
      p.color = `hsla(${h}, 85%, 70%, 0.95)`
      const ms = moonsOf.get(p) ?? []
      ms.forEach((m, i) => {
        m.r = 0.55 + Math.sqrt(m.degree) * 0.12
        const mh = tagHue(m) ?? h
        m.color = `hsla(${mh}, 45%, 82%, 0.9)`
        // Moons on a few close orbits.
        m.orbit = p.r + 1.6 + (i % 4) * 1.1 + Math.floor(i / 4) * 0.5
        m.speed = (1.4 / Math.sqrt(m.orbit)) * (rng(m.id + ':dir')() < 0.15 ? -1 : 1)
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
      // Spread around the ring, not evenly: each planet gets its own slot (in an order fixed by its
      // id) and sits a little off its centre, so the spacing looks natural but never bunches up.
      const base = rng(`ring:${sun.id}:${ring}`)() * TAU
      const slot = TAU / onRing.length
      const slots = [...onRing].sort((x, y) => rng(x.id + ':slot')() - rng(y.id + ':slot')())
      slots.forEach((p, k) => {
        p.phase = base + k * slot + (rng(p.id + ':jitter')() - 0.5) * slot * 0.55
      })
      onRing.forEach((p) => {
        p.orbit = radius
        p.speed = 1.1 / Math.pow(radius, 0.75)
      })
      rings.push(radius)
      radius += room + 3
      i += cap
      ring++
    }
    const ext = rings.length ? rings[rings.length - 1] + Math.max(...planets.map((p) => reach.get(p)!)) : sun.r * 3
    // Each system has its own plane, tipped well away from the disc's; within it every planet
    // orbits at its own small tilt, and moons at bigger ones, so a system has depth.
    const rs = rng('plane:' + sun.id)
    const n = norm([gauss(rs) * 0.6, 1, gauss(rs) * 0.6])
    const u = norm(cross(n, [0, 0, 1]))
    const v = cross(n, u)
    for (const p of planets) {
      {
      const rp = rng(p.id + ':orbit')
      ;[p.ou, p.ov] = tilt(u, v, clamp(gauss(rp) * 0.2, -0.55, 0.55), rp() * TAU)
    }
      for (const m of moonsOf.get(p) ?? []) {
        const rm = rng(m.id + ':orbit')
        ;[m.ou, m.ov] = tilt(p.ou, p.ov, clamp(gauss(rm) * 0.5, -1.2, 1.2), rm() * TAU)
      }
    }
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
    const lift = R * (0.025 + 0.08 * Math.exp(-((d / R) ** 2) * 5))
    s.center = [Math.cos(a) * d, k === 0 ? 0 : gauss(r) * lift, Math.sin(a) * d]
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
    const thick = g.R * (0.025 + 0.09 * Math.exp(-((d / g.R) ** 2) * 6))
    s.home = [Math.cos(a) * d + gauss(r) * g.R * 0.03, gauss(r) * thick, Math.sin(a) * d + gauss(r) * g.R * 0.03]
    s.r = 0.9 + Math.sqrt(s.degree) * 0.3 + (s.pinned ? 0.6 : 0)
    // Untagged stars take the galaxy's colours: gold-white near the core, its own hue along
    // the arms, bluer at the edges.
    const th = tagHue(s)
    if (th !== null) s.color = `hsla(${th}, 90%, 74%, 0.9)`
    else {
      const f = d / g.R
      const jitter = rng(s.id + ':hue')() - 0.5
      const h = f < 0.18 ? 42 : f < 0.6 ? hue + jitter * 24 : hue * 0.4 + 215 * 0.6 + jitter * 20
      s.color = `hsla(${h}, ${f < 0.18 ? 85 : 78}%, ${f < 0.18 ? 84 : 76}%, 0.9)`
    }
    g.stars.push(s)
  })
  g.bodies = [...g.systems.flatMap((s) => s.bodies), ...g.stars]
  return g
}

// ---- the view -------------------------------------------------------------------------------

let lastSpaceTime = 0

export function createSpace(el: HTMLCanvasElement, inputs: GalaxyInput[], onOpen: (noteId: string) => void, onGalaxy?: (id: string | null) => void): SpaceHandle {
  warmTextures()
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
  // Time carries on across rebuilds (a note changing rebuilds the view), so planets don't jump back to the start of their orbits.
  let time = lastSpaceTime
  /** 1 while things move, easing to 0 while a body is hovered. */
  let motion = 1
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
            b.local = [host[0] + b.ou[0] * c + b.ov[0] * sn, host[1] + b.ou[1] * c + b.ov[1] * sn, host[2] + b.ou[2] * c + b.ov[2] * sn]
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
    // Pointing at a planet, star, sun or moon eases everything to a stop (orbits, spins, the
    // galaxy's turning, the slow drift of the view) so it holds still to be looked at and clicked.
    motion += ((hover ? 0 : 1) - motion) * (1 - Math.exp(-dt * 9))
    if (!reduced) {
      const m = dt * motion
      time += m
      lastSpaceTime = time
      for (const g of galaxies) g.angle += m * g.spin
      // Drift slowly around on its own when left alone.
      if (!drag && now - idleSince > 4000) goal.yaw += m * 0.035
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
      // Each planet's own tilted orbit (and its moons', once you're right up close).
      // With lots of planets the orbit lines are just clutter (and cost a lot to draw), so a busy
      // system shows none.
      const crowded = planetCount(s) > CROWDED
      const steps = Math.round(clamp(s.extPx / 4, 24, 72))
      const path = new Path2D()
      const ring = (center: V3, ou: V3, ov: V3, R: number) => {
        for (let k = 0; k <= steps; k++) {
          const a = (k / steps) * TAU
          const ca = Math.cos(a) * R
          const sa = Math.sin(a) * R
          const q = project(toWorld(g, [center[0] + ou[0] * ca + ov[0] * sa, center[1] + ou[1] * ca + ov[1] * sa, center[2] + ou[2] * ca + ov[2] * sa]))
          if (k === 0) path.moveTo(q.x, q.y)
          else path.lineTo(q.x, q.y)
        }
      }
      if (!crowded) {
        for (const b of s.bodies) {
          if (b.role === 'planet') ring(c, b.ou, b.ov, b.orbit)
          else if (b.role === 'moon' && s.extPx > 260) ring(b.parent!.local, b.ou, b.ov, b.orbit)
        }
        ctx.strokeStyle = `hsla(${g.hue}, 60%, 75%, ${(dimAll ? 0.04 : 0.09) * strength})`
        ctx.stroke(path)
      }
      // Links between its notes.
      // All the ordinary links in one stroke (thousands of separate strokes are slow); the ones
      // touching the hovered note are drawn brighter on top.
      const baseAlpha = (dimAll ? 0.04 : 0.14) * strength
      if (baseAlpha >= 0.015) {
        const plain = new Path2D()
        for (const [a, b] of s.links) {
          if (hover && (a === hover || b === hover)) continue
          plain.moveTo(a.sx, a.sy)
          plain.lineTo(b.sx, b.sy)
        }
        ctx.strokeStyle = `rgba(190, 185, 255, ${baseAlpha})`
        ctx.lineWidth = 1
        ctx.stroke(plain)
      }
      if (hover) {
        ctx.strokeStyle = hover.color.replace(/[\d.]+\)$/, '0.75)')
        ctx.lineWidth = 1.4
        ctx.beginPath()
        for (const [a, b] of s.links) {
          if (a !== hover && b !== hover) continue
          ctx.moveTo(a.sx, a.sy)
          ctx.lineTo(b.sx, b.sy)
        }
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
    sweepSpheres()
    // With many spheres on screen each is re-shaded every other (or third) frame: they turn slowly,
    // and the picture from a frame ago is close enough.
    const spheresShown = visible.filter((v) => v.role !== 'star' && v.px >= 6).length
    const period = spheresShown > 40 ? 3 : spheresShown > 8 ? 2 : 1
    // How many pixels each sphere is shaded at: the nearest get the most, within a budget.
    const sphereN = new Map<Body, number>()
    let budget = 50000
    for (const b of [...visible].sort((a, c) => c.px - a.px)) {
      if (b.role === 'star' || b.px < (b.role === 'sun' ? 3 : 6)) continue
      // Sizes come in steps of 8 and only change when they've moved a good way, so a body's detail
      // doesn't flicker back and forth.
      const want = clamp(Math.round((b.px * 2.4) / 8) * 8, 16, 144)
      const had = sphereSteps.get(b)
      const n = had !== undefined && Math.abs(had - want) < 16 ? had : want
      sphereSteps.set(b, n)
      const use = budget > n * n ? n : Math.max(16, Math.round(n / 16) * 8)
      budget -= use * use
      sphereN.set(b, use)
    }
    // How the camera turns directions: world → (right, up, toward the camera).
    const Rv = [cy, 0, sy, sy * sp, cp, -cy * sp, sy * cp, -sp, -cy * cp]
    const viewDir = (d: V3): V3 => [Rv[0] * d[0] + Rv[1] * d[1] + Rv[2] * d[2], Rv[3] * d[0] + Rv[4] * d[1] + Rv[5] * d[2], Rv[6] * d[0] + Rv[7] * d[1] + Rv[8] * d[2]]
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
      ctx.globalAlpha = alpha * (b.role === 'planet' || b.role === 'moon' ? 1 - near * 0.75 : b.role === 'star' ? 0.4 : 1)
      if (b.role !== 'star' || on) ctx.drawImage(glow(b.role === 'sun' ? b.corona : b.color, b.role === 'sun' ? 0.3 : 0.22), b.sx - rad, b.sy - rad, rad * 2, rad * 2)
      ctx.globalCompositeOperation = 'source-over'
      if (b.role === 'star') {
        // A glowing four-point star, like the one in the logo.
        const size = clamp(px * 9, 12.75, 142) * 1.25 * (0.88 + 0.12 * tw)
        ctx.globalAlpha = alpha
        ctx.drawImage(starSprite(b.color, size < 44 ? 64 : 192), b.sx - size / 2, b.sy - size / 2, size, size)
      } else if (sphereN.has(b)) {
        // A sun, planet or moon you can see: a textured sphere that turns on its own axis.
        const n = sphereN.get(b)!
        const show = clamp((px - 1.6) / 2.4, 0, 1)
        const spin = spinOf(b)
        const th = (reduced ? 0 : time * spin.rate) + spin.offset
        const cs = Math.cos(th)
        const sn = Math.sin(th)
        const ct = Math.cos(spin.tilt)
        const st = Math.sin(spin.tilt)
        // Body-to-world turn (tilt of the axis after the spin), then world-to-view.
        const A = [ct * cs, -st, ct * sn, st * cs, ct, st * sn, -sn, 0, cs]
        const B: number[] = []
        for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) B.push(Rv[r * 3] * A[c] + Rv[r * 3 + 1] * A[3 + c] + Rv[r * 3 + 2] * A[6 + c])
        const [hue, sat, lig] = hslOf(b.color)
        if (b.role === 'sun') {
          // Corona behind it, then the disc.
          const reach = px / SUN_DISC
          ctx.globalCompositeOperation = 'lighter'
          ctx.globalAlpha = alpha * show * 0.9
          ctx.drawImage(coronaSprite(b.color), b.sx - reach, b.sy - reach, reach * 2, reach * 2)
          ctx.globalCompositeOperation = 'source-over'
          ctx.globalAlpha = alpha * show
          const cachedSun = sphereCanvases.get(b.id)
          const freshSun = cachedSun && cachedSun.n === n && sphereFrame - cachedSun.shaded < period && (sphereFrame + spin.seed) % period !== 0
          const sunSurface = freshSun ? cachedSun.canvas : shadeSphere(b.id, n, 'sun', makeTexture('sun', spin.seed), paletteOf('sun', b.color), B, [0, 0, 1], false, hslToRgb(hue - 8, 100, 62))
          ctx.drawImage(sunSurface, b.sx - px, b.sy - px, px * 2, px * 2)
        } else {
          const sun = b.system?.sun
          const toSun: V3 = sun ? [sun.world[0] - b.world[0], sun.world[1] - b.world[1], sun.world[2] - b.world[2]] : [-1, 1, 1]
          const ll = len(toSun) || 1
          const L = viewDir([toSun[0] / ll, toSun[1] / ll, toSun[2] / ll])
          const variant = planetVariant(b)
          const ringed = b.role === 'planet' && PLANET_RINGED[variant]
          const kind: Kind = b.role === 'moon' ? 'moon' : PLANET_KIND[variant]
          const rim = b.role === 'moon' ? ([150, 155, 175] as V3) : hslToRgb(hue, sat, 78)
          // The ring is a circle around the equator; the half nearer the camera is drawn over the planet.
          const ring = (front: boolean) => {
            for (const [rad, wd, a] of [[1.55, 0.2, 0.55], [1.92, 0.085, 0.34]] as const) {
              ctx.strokeStyle = hsl(hue + 18, sat * 0.6, lig + 8, a)
              ctx.lineWidth = Math.max(1, px * wd)
              ctx.beginPath()
              let pen = false
              for (let k = 0; k <= 72; k++) {
                const t = (k / 72) * TAU
                const X = Math.cos(t) * rad
                const Z = Math.sin(t) * rad
                const vx = B[0] * X + B[2] * Z
                const vy = B[3] * X + B[5] * Z
                const vz = B[6] * X + B[8] * Z
                if (vz >= 0 === front) {
                  if (pen) ctx.lineTo(b.sx + vx * px, b.sy - vy * px)
                  else (ctx.moveTo(b.sx + vx * px, b.sy - vy * px), (pen = true))
                } else pen = false
              }
              ctx.stroke()
            }
          }
          ctx.globalAlpha = alpha * show
          if (ringed) ring(false)
          const cached = sphereCanvases.get(b.id)
          const fresh = cached && cached.n === n && sphereFrame - cached.shaded < period && (sphereFrame + spin.seed) % period !== 0
          const surface = fresh ? cached.canvas : shadeSphere(b.id, n, kind, makeTexture(kind, spin.seed), paletteOf(kind, b.color), B, L, true, rim)
          ctx.drawImage(surface, b.sx - px, b.sy - px, px * 2, px * 2)
          if (ringed) ring(true)
        }
      } else if (px > 1.6) {
        // Too small for a textured sphere: a cheap lit disc (planets and moons; far suns are just glows).
        if (b.role !== 'sun') {
          const show = clamp((px - 1.6) / 2.4, 0, 1)
          const reach = px / DISC_RADIUS
          ctx.globalAlpha = alpha * show
          ctx.drawImage(discSprite(b.color, b.role === 'moon'), b.sx - reach, b.sy - reach, reach * 2, reach * 2)
        }
      }
      ctx.globalCompositeOperation = 'lighter'
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
        if (s.sun === hover) continue // the hover label below names it
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
    // The planets of a system you are up close to (moons are named only when pointed at); hover and search results.
    ctx.font = '500 11px Inter, system-ui, sans-serif'
    const close = focus.kind === 'system' && focus.s.extPx > 160 ? focus.s.bodies.filter((b) => b.role === 'planet' && b.shown) : []
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
    zoomBy(k: number) {
      view.dist *= k
      goal.dist = view.dist
    },
    debugState: () => ({ hover: hover?.title ?? null, motion, time }), // dev only
    focusBiggestSystem() {
      const s = [...systems].sort((a, b) => b.bodies.length - a.bodies.length)[0]
      if (s) setFocus({ kind: 'system', s })
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
