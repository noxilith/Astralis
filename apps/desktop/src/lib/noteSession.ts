// A note open in the editor, as a shared Yjs document (the same CRDT the backend stores with yrs).
//
// Every keystroke becomes a small Yjs update: it's stored (so sync carries it) and sent to
// connected devices, which merge it — nothing ever replaces the text wholesale, so concurrent
// edits can't clobber each other. Carets travel as Yjs awareness, anchored to the text itself,
// so they move with edits.
import * as Y from 'yjs'
import { Awareness, applyAwarenessUpdate, encodeAwarenessUpdate, removeAwarenessStates } from 'y-protocols/awareness'
import { p2p, type Id } from '../api'
import { ME_CHANGED, PEER_JOINED, REMOTE_AWARENESS, REMOTE_UPDATE, SYNCED, getMe, throttle } from './collab'

/** Origin for changes that came from elsewhere (not to be stored or sent again). */
const REMOTE = 'remote'
/** Local edits are batched this long before being stored and sent. */
const FLUSH_MS = 40

export interface NoteSession {
  noteId: Id
  ytext: Y.Text
  awareness: Awareness
  undoManager: Y.UndoManager
  close: () => void
}

type Detail = { noteId: string; update: Uint8Array }

/** Loads a note's text state and starts collaborating on it. `onBody` gets the stored text after each edit. */
export async function openNoteSession(noteId: Id, onBody: (body: string) => void): Promise<NoteSession> {
  const doc = new Y.Doc()
  Y.applyUpdate(doc, Uint8Array.from(await p2p.noteCrdt(noteId)), REMOTE)
  const ytext = doc.getText('body')
  const awareness = new Awareness(doc)
  const setUser = () => {
    const me = getMe()
    awareness.setLocalStateField('user', { name: me.name, color: me.color, colorLight: me.color + '33' })
  }
  setUser()

  // ---- local edits → store + send ----
  let pending: Uint8Array[] = []
  let timer: ReturnType<typeof setTimeout> | null = null
  const flush = () => {
    if (timer) clearTimeout(timer)
    timer = null
    if (!pending.length) return
    const update = pending.length === 1 ? pending[0] : Y.mergeUpdates(pending)
    pending = []
    p2p
      .noteEdit(noteId, update)
      .then((body) => body != null && onBody(body))
      .catch((e) => console.error('saving edit failed', e))
  }
  const onUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin === REMOTE) return
    pending.push(update)
    if (!timer) timer = setTimeout(flush, FLUSH_MS)
  }
  doc.on('update', onUpdate)

  // ---- local caret → send ----
  const sendAwareness = throttle(() => {
    p2p.liveAwareness(noteId, encodeAwarenessUpdate(awareness, [doc.clientID])).catch(() => {})
  }, 50)
  const onAwareness = (_: unknown, origin: unknown) => {
    if (origin !== REMOTE) sendAwareness()
  }
  awareness.on('update', onAwareness)

  // ---- from collaborators ----
  const onRemoteUpdate = (e: Event) => {
    const d = (e as CustomEvent<Detail>).detail
    if (d.noteId === noteId) Y.applyUpdate(doc, d.update, REMOTE)
  }
  const onRemoteAwareness = (e: Event) => {
    const d = (e as CustomEvent<Detail>).detail
    if (d.noteId === noteId) applyAwarenessUpdate(awareness, d.update, REMOTE)
  }
  // Background sync merged changes into the stored note (e.g. edits made while apart): fold them in.
  const onSynced = (e: Event) => {
    if (!(e as CustomEvent<string[]>).detail.includes(noteId)) return
    p2p
      .noteCrdt(noteId)
      .then((s) => Y.applyUpdate(doc, Uint8Array.from(s), REMOTE))
      .catch(() => {})
  }
  // Someone opened this note: send them everything we have, and where we are.
  const onPeerJoined = (e: Event) => {
    if ((e as CustomEvent<string>).detail !== noteId) return
    flush()
    p2p.noteEdit(noteId, Y.encodeStateAsUpdate(doc)).catch(() => {})
    sendAwareness()
  }
  window.addEventListener(REMOTE_UPDATE, onRemoteUpdate)
  window.addEventListener(REMOTE_AWARENESS, onRemoteAwareness)
  window.addEventListener(SYNCED, onSynced)
  window.addEventListener(PEER_JOINED, onPeerJoined)
  window.addEventListener(ME_CHANGED, setUser)
  sendAwareness()

  return {
    noteId,
    ytext,
    awareness,
    undoManager: new Y.UndoManager(ytext),
    close: () => {
      flush()
      window.removeEventListener(REMOTE_UPDATE, onRemoteUpdate)
      window.removeEventListener(REMOTE_AWARENESS, onRemoteAwareness)
      window.removeEventListener(SYNCED, onSynced)
      window.removeEventListener(PEER_JOINED, onPeerJoined)
      window.removeEventListener(ME_CHANGED, setUser)
      awareness.off('update', onAwareness)
      // Tell collaborators our caret is gone.
      removeAwarenessStates(awareness, [doc.clientID], 'local')
      p2p.liveAwareness(noteId, encodeAwarenessUpdate(awareness, [doc.clientID])).catch(() => {})
      awareness.destroy()
      doc.off('update', onUpdate)
      doc.destroy()
    },
  }
}
