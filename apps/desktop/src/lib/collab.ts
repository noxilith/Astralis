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
/** This device's name and color as collaborators see them. */
let me = { name: 'Me', color: '#8b7bff' }
export const getMe = () => me
/** Fired (on window) when this device's name or color changed. */
export const ME_CHANGED = 'astralis-me-changed'
const setMe = (name: string, color: string) => {
  if (!name || !color || (name === me.name && color === me.color)) return
  me = { name, color }
  window.dispatchEvent(new Event(ME_CHANGED))
}
/** Re-reads this device's name/color (after changing them in Settings). */
export const refreshMe = () =>
  p2p.status().then((s) => (setMe(s.name, s.color), notify()))

/** Events the editor and whiteboard listen for. */
/** A collaborator's text edit / caret update for a note: detail { noteId, update: Uint8Array }. */
export const REMOTE_UPDATE = 'astralis-remote-update'
export const REMOTE_AWARENESS = 'astralis-remote-awareness'
/** A collaborator opened a note: detail = noteId (open editors send them what they have). */
export const PEER_JOINED = 'astralis-peer-joined'
export const REMOTE_SCENE = 'astralis-remote-scene'
export const SYNCED = 'astralis-synced'
export const PAIRED = 'astralis-paired'

let started = false
/** Subscribes to P2P events once; `onSynced` refreshes views when remote changes land. */
export function startCollab(onSynced: () => void) {
  if (started) return
  started = true
  const refresh = () => p2p.status().then((s) => ((peers = s.peers), setMe(s.name, s.color), notify())).catch(() => {})
  refresh()
  listen('p2p-ready', refresh)
  // Notes an AI agent changed through the Brain's MCP server: refresh like a sync.
  listen<string[]>('outside-edits', ({ payload: ids }) => {
    onSynced()
    window.dispatchEvent(new CustomEvent(SYNCED, { detail: ids }))
  })
  listen<P2pEvent>('p2p', ({ payload: e }) => {
    switch (e.type) {
      case 'peers':
        peers = e.peers
        // Forget presence of devices that left; pick up new names/colors of those still here.
        presence = new Map(
          [...presence].flatMap(([id, pr]) => {
            const peer = peers.find((p) => p.id === id && p.connected)
            return peer ? [[id, { ...pr, name: peer.name, color: peer.color }] as const] : []
          }),
        )
        notify()
        break
      case 'paired':
        window.dispatchEvent(new CustomEvent(PAIRED, { detail: e.name }))
        refresh()
        break
      case 'vaultsChanged':
        onSynced()
        break
      case 'synced':
        onSynced()
        window.dispatchEvent(new CustomEvent(SYNCED, { detail: e.notes }))
        break
      case 'live':
        if (e.live.kind === 'presence') {
          const p = e.live
          if (p.noteId && presence.get(e.peer)?.noteId !== p.noteId) window.dispatchEvent(new CustomEvent(PEER_JOINED, { detail: p.noteId }))
          presence = new Map(presence)
          if (p.noteId) presence.set(e.peer, { peer: e.peer, name: e.name, color: e.color, noteId: p.noteId, anchor: p.anchor, head: p.head, pointer: p.pointer })
          else presence.delete(e.peer)
          notify()
        } else if (e.live.kind === 'update' || e.live.kind === 'awareness') {
          const type = e.live.kind === 'update' ? REMOTE_UPDATE : REMOTE_AWARENESS
          window.dispatchEvent(new CustomEvent(type, { detail: { noteId: e.live.noteId, update: Uint8Array.from(e.live.update) } }))
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
