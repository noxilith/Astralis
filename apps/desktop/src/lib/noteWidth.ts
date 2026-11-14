import { api } from '../api'

export type NoteWidth = 'readable' | 'full'

/** "readable" keeps lines to a comfortable length; "full" lets notes fill the window. */
export function applyNoteWidth(w: NoteWidth) {
  document.documentElement.dataset.noteWidth = w
}

export async function loadNoteWidth(): Promise<NoteWidth> {
  const v = await api.getSetting('noteWidth').catch(() => null)
  return v === 'full' ? 'full' : 'readable'
}

export function saveNoteWidth(w: NoteWidth) {
  applyNoteWidth(w)
  void api.setSetting('noteWidth', w).catch(() => {})
}
