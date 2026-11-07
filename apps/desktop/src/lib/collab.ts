// Live collaboration state shared across the UI: connected devices and where each one is.
import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { p2p, type P2pEvent, type PeerStatus } from '../api'

export interface Presence {
  peer: string
  name: string
  color: string
  noteId: string | null
  anchor: number | null
  head: number | null
  pointer: [number, number] | null
}

type Listener = () => void
let peers: PeerStatus[] = []
let presence = new Map<string, Presence>()
const listeners = new Set<Listener>()
const notify = () => listeners.forEach((l) => l())

export const connectedCount = () => peers.filter((p) => p.connected).length
export const getPeers = () => peers

/** Events the editor and whiteboard listen for. */
export const REMOTE_TEXT = 'astralis-remote-text'
export const REMOTE_SCENE = 'astralis-remote-scene'
export const SYNCED = 'astralis-synced'
export const PAIRED = 'astralis-paired'

let started = false
/** Subscribes to P2P events once; `onSynced` refreshes views when remote changes land. */
export function startCollab(onSynced: () => void) {
  if (started) return
  started = true
  const refresh = () => p2p.status().then((s) => ((peers = s.peers), notify())).catch(() => {})
  refresh()
  listen('p2p-ready', refresh)
  listen<P2pEvent>('p2p', ({ payload: e }) => {
    switch (e.type) {
      case 'peers':
        peers = e.peers
        // Forget presence of devices that left.
        presence = new Map([...presence].filter(([id]) => peers.some((p) => p.id === id && p.connected)))
        notify()
        break
      case 'paired':
        window.dispatchEvent(new CustomEvent(PAIRED, { detail: e.name }))
        refresh()
        break
      case 'synced':
        onSynced()
        window.dispatchEvent(new CustomEvent(SYNCED, { detail: e.notes }))
        break
      case 'remoteText':
        window.dispatchEvent(new CustomEvent(REMOTE_TEXT, { detail: { noteId: e.noteId, body: e.body } }))
        break
      case 'live':
        if (e.live.kind === 'presence') {
          const p = e.live
          presence = new Map(presence)
          if (p.noteId) presence.set(e.peer, { peer: e.peer, name: e.name, color: e.color, noteId: p.noteId, anchor: p.anchor, head: p.head, pointer: p.pointer })
          else presence.delete(e.peer)
          notify()
        } else if (e.live.kind === 'scene') {
          window.dispatchEvent(new CustomEvent(REMOTE_SCENE, { detail: { noteId: e.live.noteId, elements: e.live.elements, peer: e.peer } }))
        }
        break
    }
  })
}

/** Re-renders when devices or their positions change. */
export function useCollab() {
  const [, force] = useState(0)
  useEffect(() => {
    const l = () => force((n) => n + 1)
    listeners.add(l)
    return () => void listeners.delete(l)
  }, [])
  return { peers, presence: [...presence.values()], connected: connectedCount() }
}

/** Calls `fn` at most every `ms`, always delivering the latest call (leading + trailing). */
export function throttle<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let last = 0
  let timer: ReturnType<typeof setTimeout> | null = null
  let pending: A | null = null
  return (...args: A) => {
    const now = Date.now()
    if (now - last >= ms) {
      last = now
      fn(...args)
      return
    }
    pending = args
    if (!timer)
      timer = setTimeout(() => {
        timer = null
        last = Date.now()
        if (pending) fn(...pending)
        pending = null
      }, ms - (now - last))
  }
}
