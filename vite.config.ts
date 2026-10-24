import { defineConfig } from 'vite'

export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
    // The main window's page and Auto-Type's chooser (#307).
    rollupOptions: { input: { main: 'index.html', chooser: 'chooser.html' } },
  },
})
