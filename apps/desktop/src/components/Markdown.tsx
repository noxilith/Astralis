import { useEffect, useMemo, useRef } from 'react'
import { Marked, type Tokens } from 'marked'
import DOMPurify from 'dompurify'
import { ATTACHMENT_RE, attachmentUrl, splitAlt } from '../lib/attachments'
import { highlightElement } from "../lib/highlight"
import { SYNCED } from "../lib/collab"

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')

// Counts repeated uses of the same image so a resize can find the right one in the source.
let occurrences = new Map<string, number>()

const md = new Marked({ gfm: true, breaks: true, async: false }).use({
  renderer: {
    code({ text, lang }: Tokens.Code) {
      const language = (lang ?? "").trim().split(/\s+/)[0]
      const label = language ? `<span class="code-lang">${escapeHtml(language)}</span>` : ""
      return `<div class="code-block">${label}<pre><code data-lang="${escapeHtml(language)}">${escapeHtml(text)}</code></pre></div>`
    },
    image({ href, text }: Tokens.Image) {
      const { alt, width } = splitAlt(text)
      const occ = occurrences.get(href) ?? 0
      occurrences.set(href, occ + 1)
      const attachment = ATTACHMENT_RE.exec(href)?.[1]
      const style = width ? ` style="width:${width}px"` : ''
      const img = attachment
        ? `<img alt="${escapeHtml(alt)}" data-attachment="${attachment}">`
        : `<img alt="${escapeHtml(alt)}" src="${escapeHtml(href)}">`
      return `<span class="md-img" data-src="${escapeHtml(href)}" data-occ="${occ}"${style}>${img}<span class="md-img-handle" title="Drag to resize"></span></span>`
    },
  },
})

/**
 * Renders Markdown safely (sanitized). [[wikilinks]] become clickable; vault images load from
 * the database and can be resized by dragging their bottom-right corner (when `onResizeImage`
 * is given).
 */
export function Markdown({
  source,
  onOpenLink,
  onResizeImage,
}: {
  source: string
  onOpenLink?: (title: string) => void
  onResizeImage?: (src: string, occurrence: number, width: number) => void
}) {
  const root = useRef<HTMLDivElement>(null)
  const html = useMemo(() => {
    const withLinks = source.replace(/\[\[([^\]\n]+?)\]\]/g, (_, inner: string) => {
      const [targetPart, alias] = inner.split('|')
      const target = targetPart.split('#')[0].trim()
      const label = (alias ?? targetPart).trim()
      return `<a class="wikilink" data-wikilink="${escapeHtml(target)}">${escapeHtml(label)}</a>`
    })
    occurrences = new Map()
    const raw = md.parse(withLinks) as string
    return DOMPurify.sanitize(raw, { ADD_ATTR: ["data-wikilink", "data-attachment", "data-src", "data-occ", "data-lang"] })
  }, [source])

  // Color code blocks by their language.
  useEffect(() => {
    root.current?.querySelectorAll<HTMLElement>("code[data-lang]").forEach((el) => {
      if (el.dataset.lang) highlightElement(el, el.dataset.lang).catch(() => {})
    })
  }, [html])

  // Load vault images. Someone else's image can reach this device a moment after the note that
  // shows it (the text streams live; the image comes with the next sync), so images that aren't
  // here yet are tried again whenever a sync lands, and a few more times after that.
  useEffect(() => {
    let alive = true
    const load = () =>
      root.current?.querySelectorAll<HTMLImageElement>('img[data-attachment]').forEach((img) => {
        if (img.dataset.loaded) return
        attachmentUrl(img.dataset.attachment!)
          .then((url) => {
            if (!alive) return
            img.src = url
            img.dataset.loaded = '1'
            img.closest('.md-img')?.classList.remove('missing')
          })
          .catch(() => img.closest('.md-img')?.classList.add('missing'))
      })
    load()
    window.addEventListener(SYNCED, load)
    const timers = [2000, 5000, 12000, 30000].map((ms) => setTimeout(load, ms))
    return () => {
      alive = false
      window.removeEventListener(SYNCED, load)
      timers.forEach(clearTimeout)
    }
  }, [html])

  return (
    <div
      ref={root}
      className={'preview' + (onResizeImage ? ' resizable' : '')}
      dangerouslySetInnerHTML={{ __html: html }}
      onPointerDown={(e) => {
        const handle = e.target as HTMLElement
        if (!onResizeImage || !handle.classList.contains('md-img-handle')) return
        const wrap = handle.parentElement as HTMLElement
        const maxWidth = root.current?.clientWidth ?? 2000
        const startX = e.clientX
        const startW = wrap.getBoundingClientRect().width
        e.preventDefault()
        try {
          handle.setPointerCapture(e.pointerId)
        } catch {
          /* capture unavailable (e.g. synthetic pointer): moves still reach the handle */
        }
        wrap.classList.add('resizing')
        const move = (ev: PointerEvent) => {
          wrap.style.width = `${Math.max(60, Math.min(maxWidth, startW + ev.clientX - startX))}px`
        }
        const up = () => {
          handle.removeEventListener('pointermove', move)
          handle.removeEventListener('pointerup', up)
          wrap.classList.remove('resizing')
          const width = Math.round(wrap.getBoundingClientRect().width)
          if (width !== Math.round(startW)) onResizeImage(wrap.dataset.src!, Number(wrap.dataset.occ), width)
        }
        handle.addEventListener('pointermove', move)
        handle.addEventListener('pointerup', up)
      }}
      onClick={(e) => {
        const a = (e.target as HTMLElement).closest('a')
        if (!a) return
        const target = a.getAttribute('data-wikilink')
        e.preventDefault()
        if (target && onOpenLink) onOpenLink(target)
      }}
    />
  )
}
