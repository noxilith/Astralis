import { useEffect, useState } from 'react'
import { api } from '../api'

/** The assistant's name and picture, as set in Settings (kept in the app's settings). */
export interface AssistantProfile {
  name: string
  /** A small image as a data URL; null = the default sparkle. */
  avatar: string | null
}

export const DEFAULT_NAME = 'Astralis'
const NAME_KEY = 'assistant.name'
const AVATAR_KEY = 'assistant.avatar'

let profile: AssistantProfile = { name: DEFAULT_NAME, avatar: null }
let loaded: Promise<void> | null = null
const listeners = new Set<() => void>()
const notify = () => listeners.forEach((f) => f())

function load() {
  loaded ??= Promise.all([api.getSetting(NAME_KEY).catch(() => null), api.getSetting(AVATAR_KEY).catch(() => null)]).then(([name, avatar]) => {
    profile = { name: name?.trim() || DEFAULT_NAME, avatar: avatar && avatar.startsWith('data:image/') ? avatar : null }
    notify()
  })
  return loaded
}

export function useAssistantProfile(): AssistantProfile {
  const [, tick] = useState(0)
  useEffect(() => {
    const f = () => tick((n) => n + 1)
    listeners.add(f)
    void load()
    return () => void listeners.delete(f)
  }, [])
  return profile
}

export function setAssistantName(name: string) {
  const n = name.trim().slice(0, 30)
  profile = { ...profile, name: n || DEFAULT_NAME }
  void api.setSetting(NAME_KEY, n).catch(() => {})
  notify()
}

export function setAssistantAvatar(avatar: string | null) {
  profile = { ...profile, avatar }
  void api.setSetting(AVATAR_KEY, avatar ?? '').catch(() => {})
  notify()
}

/** Reads a picked image, crops it to a centered square and shrinks it (so it stays tiny). */
export function avatarFromFile(file: File, size = 160): Promise<string> {
  return new Promise((resolve, reject) => {
    if (!file.type.startsWith('image/')) return reject(new Error('That file isn’t an image.'))
    const url = URL.createObjectURL(file)
    const img = new Image()
    img.onload = () => {
      URL.revokeObjectURL(url)
      const side = Math.min(img.naturalWidth, img.naturalHeight)
      if (!side) return reject(new Error('Couldn’t read that image.'))
      const canvas = document.createElement('canvas')
      canvas.width = canvas.height = size
      const ctx = canvas.getContext('2d')
      if (!ctx) return reject(new Error('Couldn’t read that image.'))
      ctx.drawImage(img, (img.naturalWidth - side) / 2, (img.naturalHeight - side) / 2, side, side, 0, 0, size, size)
      resolve(canvas.toDataURL('image/webp', 0.88))
    }
    img.onerror = () => (URL.revokeObjectURL(url), reject(new Error('Couldn’t read that image.')))
    img.src = url
  })
}
