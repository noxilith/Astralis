import { useEffect, useRef, useState } from 'react'

const MIN_OPEN = 90

/**
 * A scratchpad that lives at the bottom of every screen. Drag the line up to open it, down to
 * close it (or click the line). Plain text only, and never saved — it's gone when the app closes.
 */
export function QuickNote() {
  const [text, setText] = useState('')
  const [height, setHeight] = useState(0)
  const lastOpen = useRef(220)
  const area = useRef<HTMLTextAreaElement>(null)
  const dragging = useRef(false)
  const open = height > 0

  useEffect(() => {
    if (open && !dragging.current) area.current?.focus()
  }, [open])

  const toggle = () => setHeight((h) => (h > 0 ? 0 : lastOpen.current))

  return (
    <div className={'quicknote' + (open ? ' open' : '')} style={{ height: height + 22 }}>
      <div
        className="quicknote-grip"
        role="separator"
        aria-orientation="horizontal"
        aria-label="Quick note — drag up to open"
        title="Quick note — drag up (or click) to open. Not saved."
        tabIndex={0}
        onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), toggle())}
        onPointerDown={(e) => {
          const startY = e.clientY
          const startH = height
          let moved = false
          dragging.current = true
          try {
            ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
          } catch {
            /* fine without capture */
          }
          const maxH = Math.round(window.innerHeight * 0.7)
          const move = (ev: PointerEvent) => {
            const dy = startY - ev.clientY
            if (Math.abs(dy) > 3) moved = true
            setHeight(Math.max(0, Math.min(maxH, startH + dy)))
          }
          const up = (ev: PointerEvent) => {
            window.removeEventListener('pointermove', move)
            window.removeEventListener('pointerup', up)
            dragging.current = false
            if (!moved) return toggle()
            const h = Math.max(0, Math.min(maxH, startH + startY - ev.clientY))
            if (h < MIN_OPEN) setHeight(0)
            else {
              lastOpen.current = h
              setHeight(h)
              area.current?.focus()
            }
          }
          window.addEventListener('pointermove', move)
          window.addEventListener('pointerup', up)
        }}
      >
        <span className="quicknote-hit" />
        <span className="quicknote-pill" />
        {!open && text && <span className="quicknote-dot" title="Has text" />}
      </div>
      {open && (
        <div className="quicknote-body">
          <textarea
            ref={area}
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Quick note — plain text, not saved anywhere."
            spellCheck={false}
          />
          {text && (
            <button className="btn ghost sm quicknote-clear" onClick={() => (setText(''), area.current?.focus())}>
              Clear
            </button>
          )}
        </div>
      )}
    </div>
  )
}
