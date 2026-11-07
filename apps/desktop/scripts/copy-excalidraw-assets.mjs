// Self-host Excalidraw's fonts so whiteboards work offline (Astralis never loads from a CDN).
// The large CJK font (Xiaolai) is skipped to keep the app light; CJK text falls back to system fonts.
import { cpSync, existsSync, mkdirSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const src = 'node_modules/@excalidraw/excalidraw/dist/prod/fonts'
const dest = 'public/excalidraw/fonts'
if (!existsSync(src)) process.exit(0)
mkdirSync(dest, { recursive: true })
for (const family of readdirSync(src)) {
  if (family === 'Xiaolai') continue
  cpSync(join(src, family), join(dest, family), { recursive: true })
}
