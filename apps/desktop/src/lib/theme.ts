import { useCallback, useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { api, type MatugenColors } from '../api'

export type Theme = 'dark' | 'light' | 'matugen'

/** The matugen template Astralis reads (matugen fills in the colors for the current wallpaper). */
export const MATUGEN_TEMPLATE = `{
  "mode": "{{mode}}",
  "bg": "{{colors.surface.default.hex}}",
  "elev": "{{colors.surface_container.default.hex}}",
  "elev2": "{{colors.surface_container_high.default.hex}}",
  "hover": "{{colors.surface_container_highest.default.hex}}",
  "active": "{{colors.secondary_container.default.hex}}",
  "border": "{{colors.outline_variant.default.hex}}",
  "borderStrong": "{{colors.outline.default.hex}}",
  "text": "{{colors.on_surface.default.hex}}",
  "muted": "{{colors.on_surface_variant.default.hex}}",
  "faint": "{{colors.outline.default.hex}}",
  "accent": "{{colors.primary.default.hex}}",
  "danger": "{{colors.error.default.hex}}"
}
`

export const MATUGEN_CONFIG = `[templates.astralis]
input_path = '~/.config/matugen/templates/astralis.json'
output_path = '~/.config/astralis/matugen.json'`

export const ACCENT_PRESETS = ['#8b7bff', '#6bb4ff', '#4fd1a5', '#ffb547', '#ff6b81', '#c792ea', '#ff9f5a', '#5ee7df']

const HEX = /^#[0-9a-f]{6}$/i
const VARS = ['--bg', '--bg-elev', '--bg-elev-2', '--bg-hover', '--bg-active', '--border', '--border-strong', '--text', '--text-muted', '--text-faint', '--accent', '--accent-strong', '--accent-soft', '--danger']

const rgb = (hex: string): [number, number, number] => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as [number, number, number]
const toHex = (c: number[]) => '#' + c.map((v) => Math.round(Math.max(0, Math.min(255, v))).toString(16).padStart(2, '0')).join('')
/** Mixes `hex` toward white (t > 0) or black (t < 0). */
const shade = (hex: string, t: number) => toHex(rgb(hex).map((v) => (t >= 0 ? v + (255 - v) * t : v * (1 + t))))
const isDark = (hex: string) => {
  const [r, g, b] = rgb(hex)
  return 0.299 * r + 0.587 * g + 0.114 * b < 140
}

/** Sets (or clears) the accent: the purple everywhere becomes `color`. */
function accentVars(color: string, dark: boolean): Record<string, string> {
  const [r, g, b] = rgb(color)
  return { '--accent': color, '--accent-strong': dark ? shade(color, 0.22) : shade(color, -0.15), '--accent-soft': `rgba(${r}, ${g}, ${b}, ${dark ? 0.16 : 0.12})` }
}

/** Either our template's flat file, or a whole matugen palette (`{ mode, colors: { surface, … } }`). */
function normalize(j: Record<string, unknown>): MatugenColors {
  const c = j.colors as Record<string, string> | undefined
  if (!c) return j as MatugenColors
  return {
    mode: j.mode === 'light' ? 'light' : 'dark',
    bg: c.surface, elev: c.surface_container, elev2: c.surface_container_high, hover: c.surface_container_highest, active: c.secondary_container,
    border: c.outline_variant, borderStrong: c.outline, text: c.on_surface, muted: c.on_surface_variant, faint: c.outline, accent: c.primary, danger: c.error,
  }
}

/** Colors from matugen's output, as CSS variables. Anything missing or malformed is skipped. */
function matugenVars(c: MatugenColors): Record<string, string> {
  const ok = (v?: string) => (v && HEX.test(v) ? v : null)
  const out: Record<string, string> = {}
  const map: [string, string | null][] = [
    ['--bg', ok(c.bg)], ['--bg-elev', ok(c.elev)], ['--bg-elev-2', ok(c.elev2)], ['--bg-hover', ok(c.hover)], ['--bg-active', ok(c.active)],
    ['--border', ok(c.border)], ['--border-strong', ok(c.borderStrong)], ['--text', ok(c.text)], ['--text-muted', ok(c.muted)], ['--text-faint', ok(c.faint)], ['--danger', ok(c.danger)],
  ]
  for (const [k, v] of map) if (v) out[k] = v
  const accent = ok(c.accent)
  if (accent) Object.assign(out, accentVars(accent, c.mode !== 'light'))
  return out
}

function apply(base: 'dark' | 'light', vars: Record<string, string>) {
  const root = document.documentElement
  root.dataset.theme = base
  for (const v of VARS) root.style.removeProperty(v)
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v)
}

/**
 * Theme and accent: Dark, Light, or Matugen (the colors matugen made from your wallpaper, read
 * from a file and followed live). A custom accent replaces the purple in any of them.
 */
export function useAppearance() {
  const [theme, setThemeState] = useState<Theme>('dark')
  const [accent, setAccentState] = useState<string | null>(null)
  const [matugen, setMatugen] = useState<{ colors: MatugenColors | null; path: string }>({ colors: null, path: '' })

  useEffect(() => {
    api.getSetting('theme').then((t) => (t === 'light' || t === 'matugen' || t === 'dark') && setThemeState(t)).catch(() => {})
    api.getSetting('accent').then((a) => a && HEX.test(a) && setAccentState(a)).catch(() => {})
  }, [])

  // Follow matugen's file while that theme is on: the app tells us the moment it's rewritten
  // (a slow poll stays as a safety net, e.g. in a browser).
  useEffect(() => {
    if (theme !== 'matugen') return
    let last: string | null = null
    const read = () =>
      api
        .matugenColors()
        .then((r) => {
          const text = (r.json ?? '') + '|' + r.path
          if (text === last) return
          last = text
          let colors: MatugenColors | null = null
          try {
            colors = r.json ? normalize(JSON.parse(r.json)) : null
          } catch {
            colors = null
          }
          setMatugen({ colors, path: r.path })
        })
        .catch(() => {})
    read()
    const un = listen('matugen-changed', read)
    const t = window.setInterval(read, 5000)
    return () => {
      window.clearInterval(t)
      void un.then((f) => f())
    }
  }, [theme])

  const base: 'dark' | 'light' = theme === 'matugen' ? (matugen.colors?.mode === 'light' ? 'light' : matugen.colors ? 'dark' : 'dark') : theme
  useEffect(() => {
    const vars = theme === 'matugen' && matugen.colors ? matugenVars(matugen.colors) : {}
    if (accent) Object.assign(vars, accentVars(accent, theme === 'matugen' && matugen.colors?.bg ? isDark(matugen.colors.bg) : base === 'dark'))
    apply(base, vars)
  }, [theme, base, accent, matugen])

  const setTheme = useCallback((t: Theme) => (setThemeState(t), void api.setSetting('theme', t).catch(() => {})), [])
  const setAccent = useCallback((a: string | null) => (setAccentState(a), void api.setSetting('accent', a ?? '').catch(() => {})), [])
  return { theme, setTheme, accent, setAccent, base, matugen }
}
