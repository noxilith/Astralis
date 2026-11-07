import { useEffect, useRef, useState, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { useApp } from '../context'

export function Modal(props: {
  title: string
  onClose: () => void
  children: ReactNode
  footer?: ReactNode
  wide?: boolean
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && props.onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [props])
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && props.onClose()}>
      <div className={'modal' + (props.wide ? ' wide' : '')} role="dialog" aria-label={props.title}>
        <div className="modal-header">
          <h2>{props.title}</h2>
          <button className="icon-btn" onClick={props.onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <div className="modal-body">{props.children}</div>
        {props.footer && <div className="modal-footer">{props.footer}</div>}
      </div>
    </div>
  )
}

export function Empty({ icon, title, children }: { icon?: ReactNode; title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      {icon}
      <div style={{ fontWeight: 600, color: 'var(--text-muted)' }}>{title}</div>
      {children}
    </div>
  )
}

export const Spinner = () => <span className="spinner" aria-label="Loading" />

export function Toasts() {
  const { toasts, dismiss } = useApp()
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <div key={t.id} className={'toast' + (t.kind === 'error' ? ' error' : '')}>
          <div className="row">
            <div className="grow">{t.message}</div>
            <button className="icon-btn" onClick={() => dismiss(t.id)} aria-label="Dismiss">
              <X size={14} />
            </button>
          </div>
          {t.actions && (
            <div className="actions">
              {t.actions.map((a) => (
                <button
                  key={a.label}
                  className="btn sm"
                  onClick={() => {
                    a.run()
                    dismiss(t.id)
                  }}
                >
                  {a.label}
                </button>
              ))}
            </div>
          )}
        </div>
      ))}
    </div>
  )
}

/** Asks for confirmation with a modal; returns a trigger and the modal element. */
export function useConfirm() {
  const [state, setState] = useState<{ message: string; confirm: string; resolve: (ok: boolean) => void } | null>(null)
  const ask = (message: string, confirm = 'Confirm') =>
    new Promise<boolean>((resolve) => setState({ message, confirm, resolve }))
  const close = (ok: boolean) => {
    state?.resolve(ok)
    setState(null)
  }
  const element = state && (
    <Modal
      title="Are you sure?"
      onClose={() => close(false)}
      footer={
        <>
          <button className="btn" onClick={() => close(false)}>Cancel</button>
          <button className="btn primary" onClick={() => close(true)} autoFocus>{state.confirm}</button>
        </>
      }
    >
      {state.message}
    </Modal>
  )
  return { ask, element }
}

/** Asks for a line of text with a modal. */
export function usePrompt() {
  const [state, setState] = useState<{
    title: string
    label: string
    value: string
    password?: boolean
    resolve: (v: string | null) => void
  } | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const ask = (title: string, label: string, initial = '', password = false) =>
    new Promise<string | null>((resolve) => setState({ title, label, value: initial, password, resolve }))
  const close = (v: string | null) => {
    state?.resolve(v)
    setState(null)
  }
  const element = state && (
    <Modal
      title={state.title}
      onClose={() => close(null)}
      footer={
        <>
          <button className="btn" onClick={() => close(null)}>Cancel</button>
          <button className="btn primary" onClick={() => close(inputRef.current?.value ?? '')}>OK</button>
        </>
      }
    >
      <div className="field">
        <label>{state.label}</label>
        <input
          ref={inputRef}
          className="input"
          type={state.password ? 'password' : 'text'}
          defaultValue={state.value}
          autoFocus
          onKeyDown={(e) => e.key === 'Enter' && close(e.currentTarget.value)}
        />
      </div>
    </Modal>
  )
  return { ask, element }
}

/** Debounces a callback; `flush` runs any pending call immediately. */
export function useDebounced<A extends unknown[]>(fn: (...args: A) => void, ms: number) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pending = useRef<A | null>(null)
  const fnRef = useRef(fn)
  fnRef.current = fn
  const flush = () => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
    if (pending.current) {
      const args = pending.current
      pending.current = null
      fnRef.current(...args)
    }
  }
  const call = (...args: A) => {
    pending.current = args
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(flush, ms)
  }
  return { call, flush }
}

export const PRIORITY_LABELS = ['None', 'Low', 'Medium', 'High', 'Urgent']
export const PRIORITY_COLORS = ['var(--text-faint)', 'var(--p1)', 'var(--p2)', 'var(--p3)', 'var(--p4)']
