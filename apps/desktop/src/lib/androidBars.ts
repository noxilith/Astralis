/**
 * On Android the system draws the status and navigation bars and the app is padded around them
 * (see MainActivity in the Android build), so the page adds no space of its own for them. The
 * activity also needs to know the page's colour, to paint behind the bars and to choose light or
 * dark bar icons. In a browser there's no bridge and this does nothing.
 */
export function installAndroidBars() {
  const bridge = (window as unknown as { AstralisBars?: { setBars(color: string, light: boolean): void } }).AstralisBars
  if (!bridge) return
  document.documentElement.classList.add('android')
  const apply = () => {
    const m = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(getComputedStyle(document.body).backgroundColor)
    if (!m) return
    const [r, g, b] = [Number(m[1]), Number(m[2]), Number(m[3])]
    const hex = '#' + [r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')
    try {
      bridge.setBars(hex, 0.299 * r + 0.587 * g + 0.114 * b > 153)
    } catch {
      /* the bridge is gone: nothing to paint */
    }
  }
  apply()
  new MutationObserver(apply).observe(document.documentElement, { attributes: true })
  window.addEventListener('focus', apply)
  setTimeout(apply, 400)
}
