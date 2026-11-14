import { useEffect, useState } from 'react'

const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent
const search = typeof location === 'undefined' ? '' : location.search

/**
 * A phone or tablet operating system (Android, iOS, iPadOS). Things that can't work there (the
 * local AI model, the Brain for coding agents, installing updates by hand) are left out. `?phone=1`
 * pretends to be one, for trying it in a desktop browser.
 */
export const PHONE =
  /[?&]phone=1/.test(search) ||
  /Android|iPhone|iPad|iPod/i.test(ua) ||
  // iPadOS says it's a Mac; a Mac with a touch screen is an iPad.
  (/Macintosh/.test(ua) && typeof navigator !== 'undefined' && navigator.maxTouchPoints > 1)

/** Narrower than this, the layout is the phone one (one screen at a time, tabs at the bottom). */
export const COMPACT_WIDTH = 720

/** Whether to use the phone layout: on a phone, or in any window that narrow. */
export function useCompact(): boolean {
  const [width, setWidth] = useState(() => (typeof window === 'undefined' ? 1200 : window.innerWidth))
  useEffect(() => {
    const on = () => setWidth(window.innerWidth)
    window.addEventListener('resize', on)
    return () => window.removeEventListener('resize', on)
  }, [])
  return (PHONE && width <= 900) || width <= COMPACT_WIDTH
}
