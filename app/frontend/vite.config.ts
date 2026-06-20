import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Tauri v2 dev convention:
// - Vite listens on port 1420 (matches `app.build.devUrl` in tauri.conf.json).
// - `strictPort: true` so Vite fails fast if 1420 is taken rather than
//   silently picking another port that Tauri doesn't know about.
// - `clearScreen: false` so Tauri's stdout (Rust compile progress) stays
//   visible during `cargo tauri dev`.
// - Ignore `src-tauri/**` so Rust edits don't trigger a frontend HMR reload.
// https://vite.dev/config/ + https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  // Production build output — `tauri.conf.json`'s `frontendDist` points to
  // `../frontend/dist`, which resolves to this directory.
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  server: {
    port: 1420,
    strictPort: true,
    host: '127.0.0.1',
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  clearScreen: false,
  // `tauri` env var is set by Tauri CLI during `cargo tauri dev`. We don't
  // gate behavior on it yet, but keep it referenced so future code can use
  // `import.meta.env.TAURI_ENV_*` without additional setup.
  envPrefix: ['VITE_', 'TAURI_ENV_'],
})
