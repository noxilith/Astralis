import { api } from '../api'

/**
 * Words and sentences the user told the grammar & spelling check to leave alone. Kept in the
 * app's settings (so they survive leaving a note and restarting), matched without regard to case
 * anywhere in any note.
 */
const KEY = 'grammar.ignored'
const MAX_ENTRIES = 1000
const MAX_LENGTH = 500

let cache: string[] | null = null
const listeners = new Set<() => void>()

const norm = (s: string) => s.replace(/\s+/g, ' ').trim()

export async function loadIgnored(): Promise<string[]> {
  if (cache) return cache
  try {
    const raw = await api.getSetting(KEY)
    const list = raw ? JSON.parse(raw) : []
    cache = Array.isArray(list) ? list.filter((x): x is string => typeof x === 'string') : []
  } catch {
    cache = []
  }
  return cache
}

async function save(list: string[]) {
  cache = list
  await api.setSetting(KEY, JSON.stringify(list))
  listeners.forEach((f) => f())
}

export async function addIgnored(text: string) {
  const t = norm(text)
  if (!t || t.length > MAX_LENGTH) return
  const list = await loadIgnored()
  if (list.some((x) => x.toLowerCase() === t.toLowerCase())) return
  await save([...list, t].slice(-MAX_ENTRIES))
}

export async function removeIgnored(text: string) {
  await save((await loadIgnored()).filter((x) => x !== text))
}

export const onIgnoredChange = (f: () => void) => (listeners.add(f), () => void listeners.delete(f))

/** Where the ignored texts occur in `doc` (spans issues must not touch to be shown). */
export function ignoredSpans(doc: string, list: string[]): [number, number][] {
  const lower = doc.toLowerCase()
  const spans: [number, number][] = []
  for (const entry of list) {
    const needle = entry.toLowerCase()
    if (!needle) continue
    // Whitespace in the entry matches any whitespace in the note (line breaks, double spaces).
    const re = needle.includes(' ') ? new RegExp(needle.split(' ').map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('\\s+'), 'gi') : null
    if (re) {
      for (const m of doc.matchAll(re)) spans.push([m.index!, m.index! + m[0].length])
    } else {
      for (let i = lower.indexOf(needle); i !== -1; i = lower.indexOf(needle, i + needle.length)) {
        // A single word only counts as a whole word (ignoring "teh" shouldn't hide "tehran").
        const before = lower[i - 1]
        const after = lower[i + needle.length]
        const word = /[\p{L}\p{N}']/u
        if ((before && word.test(before) && word.test(needle[0])) || (after && word.test(after) && word.test(needle[needle.length - 1]))) continue
        spans.push([i, i + needle.length])
      }
    }
  }
  return spans
}
