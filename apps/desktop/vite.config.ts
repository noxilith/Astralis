import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Tauri expects a fixed dev port and shouldn't have its Rust sources watched.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Polling: the event-based watcher sometimes missed quick successive saves, serving stale files.
    watch: { ignored: ['**/src-tauri/**', '**/public/excalidraw/**'], usePolling: true, interval: 200 },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    target: 'es2022',
    chunkSizeWarningLimit: 4000,
  },
})
