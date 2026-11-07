export const MINUTE = 60_000
export const HOUR = 60 * MINUTE
export const DAY = 24 * HOUR

export const startOfDay = (t: number | Date = Date.now()) => {
  const d = new Date(t)
  d.setHours(0, 0, 0, 0)
  return d.getTime()
}
export const endOfDay = (t: number | Date = Date.now()) => startOfDay(t) + DAY
export const addDays = (t: number, n: number) => {
  const d = new Date(t)
  d.setDate(d.getDate() + n)
  return d.getTime()
}

const pad = (n: number) => String(n).padStart(2, '0')

/** For <input type="datetime-local">. */
export const toLocalInput = (ms: number | null | undefined) => {
  if (ms == null) return ''
  const d = new Date(ms)
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}
/** For <input type="date">. */
export const toDateInput = (ms: number | null | undefined) => toLocalInput(ms).slice(0, 10)
export const fromLocalInput = (v: string): number | null => (v ? new Date(v).getTime() : null)

export const relativeTime = (ms: number) => {
  const diff = Date.now() - ms
  if (diff < MINUTE) return 'just now'
  if (diff < HOUR) return `${Math.floor(diff / MINUTE)}m ago`
  if (diff < DAY) return `${Math.floor(diff / HOUR)}h ago`
  if (diff < 7 * DAY) return `${Math.floor(diff / DAY)}d ago`
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
}

export const formatDue = (ms: number, allDay: boolean) => {
  const today = startOfDay()
  const day = startOfDay(ms)
  const time = allDay ? '' : ' ' + new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
  if (day === today) return 'Today' + time
  if (day === today + DAY) return 'Tomorrow' + time
  if (day === today - DAY) return 'Yesterday' + time
  const sameYear = new Date(ms).getFullYear() === new Date().getFullYear()
  return (
    new Date(ms).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: sameYear ? undefined : 'numeric' }) + time
  )
}

export const formatDateTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
