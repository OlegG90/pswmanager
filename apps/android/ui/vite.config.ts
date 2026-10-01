import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'

// The phone's UI: its own page, built beside the Windows one into dist-android/.
export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  clearScreen: false,
  build: { target: 'es2022', outDir: fileURLToPath(new URL('../../../dist-android', import.meta.url)), emptyOutDir: true },
})
