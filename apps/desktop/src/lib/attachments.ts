// Image attachments: uploading files into the vault, loading them for display, and editing the
// `|width` in `![name|width](attachments/<id>.<ext>)` image references.
import { api, type Id } from '../api'

export const ATTACHMENT_RE = /^attachments\/([0-9a-f-]{36})\.[a-z0-9]+$/i

const toBase64 = (file: Blob) =>
  new Promise<string>((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(String(r.result).split(',', 2)[1] ?? '')
    r.onerror = () => reject(r.error)
    r.readAsDataURL(file)
  })

/** Stores an image in the vault and returns the Markdown that shows it. */
export async function uploadImage(file: File, vaultId: Id | null): Promise<string> {
  const a = await api.addAttachment(vaultId, file.name || 'image', file.type, await toBase64(file))
  const alt = (file.name || 'image').replace(/[[\]|]/g, '')
  return `![${alt}](${a.path})`
}

export const imageFiles = (list: FileList | null | undefined) => [...(list ?? [])].filter((f) => f.type.startsWith('image/'))

const urls = new Map<Id, Promise<string>>()

/** Object URL for an attachment (cached for the session). */
export function attachmentUrl(id: Id): Promise<string> {
  let p = urls.get(id)
  if (!p) {
    p = api.getAttachment(id).then(({ mime, data }) => {
      const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0))
      return URL.createObjectURL(new Blob([bytes], { type: mime }))
    })
    p.catch(() => urls.delete(id))
    urls.set(id, p)
  }
  return p
}

const IMAGE_MD = /!\[([^\]]*)\]\(([^)\s]+)((?:\s+"[^"]*")?)\)/g

/** Parses `alt|300` → { alt, width }. */
export function splitAlt(alt: string): { alt: string; width: number | null } {
  const m = /^(.*?)\|(\d{1,5})$/.exec(alt)
  return m ? { alt: m[1], width: Number(m[2]) } : { alt, width: null }
}

/** Sets the width of the `occurrence`-th image with this `src` in a Markdown body. */
export function setImageWidth(body: string, src: string, occurrence: number, width: number): string {
  let seen = -1
  return body.replace(IMAGE_MD, (whole, alt: string, url: string, title: string) => {
    if (url !== src || ++seen !== occurrence) return whole
    return `![${splitAlt(alt).alt}|${Math.round(width)}](${url}${title})`
  })
}

/** Uploads images and inserts their Markdown (each on its own line) at `pos` in the editor. */
export async function insertImages(
  view: import('@codemirror/view').EditorView,
  files: File[],
  vaultId: Id | null,
  pos: number,
): Promise<void> {
  const parts: string[] = []
  for (const f of files) {
    try {
      parts.push(await uploadImage(f, vaultId))
    } catch (e) {
      console.error('image upload failed', e)
      window.dispatchEvent(new CustomEvent('astralis-error', { detail: e }))
    }
  }
  if (!parts.length) return
  const line = view.state.doc.lineAt(Math.min(pos, view.state.doc.length))
  const before = pos > line.from && line.text.trim() ? '\n' : ''
  const insert = before + parts.join('\n') + '\n'
  view.dispatch({ changes: { from: pos, insert }, selection: { anchor: pos + insert.length }, scrollIntoView: true })
  view.focus()
}
