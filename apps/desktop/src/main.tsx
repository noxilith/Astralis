import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'
import App from './App'

// Browser preview (`./run.sh ui`): no Tauri runtime, so use an in-memory mock backend.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
  await import('./dev/mockBackend')
}

// No browser/webview context menu (Reload, Inspect…) — Astralis shows its own menus where they
// make sense. In development, Shift + right-click still opens it for debugging.
document.addEventListener('contextmenu', (e) => {
  if (import.meta.env.DEV && e.shiftKey) return
  e.preventDefault()
})

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
