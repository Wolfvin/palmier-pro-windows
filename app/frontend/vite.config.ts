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
    proxy: {
      // Dev-only same-origin proxy for the MCP HTTP server.
      //
      // The MCP server (app/src-tauri/src/mcp/server.rs) listens on
      // http://127.0.0.1:19789 — a different origin from the Vite dev server
      // (http://127.0.0.1:1420). The MCP server contract is locked to match
      // the macOS Swift build and intentionally does NOT send CORS headers
      // (it is meant for CLI clients like Claude / Cursor / Codex, not
      // browsers). Without this proxy, a browser-loaded dev UI would have to
      // fetch cross-origin and the browser would block the request, leaving
      // the MCP status card stuck on "Failed" even when the server is healthy.
      //
      // Vite's dev-server proxy performs the fetch server-side and returns
      // the response to the browser as same-origin, sidestepping CORS
      // entirely without weakening the MCP server's security posture.
      //
      // Prefix `/mcp-api` is stripped before forwarding (`rewrite`), so the
      // MCP server sees the exact paths it expects (`/mcp`,
      // `/.well-known/oauth-protected-resource`, …). `changeOrigin: true`
      // rewrites the `Host` header to `127.0.0.1:19789` so any host-based
      // routing on the MCP side keeps working.
      //
      // NOTE: This proxy is ONLY active under `npm run dev` / `cargo tauri dev`.
      // Production Tauri builds serve the frontend from the bundled webview
      // origin (tauri://localhost on macOS, https://tauri.localhost on
      // Windows, http://tauri.localhost on Linux) — there is no Vite dev
      // server in that path. See app/README.md "MCP fetch in production"
      // for the production behaviour and follow-up plan.
      '/mcp-api': {
        target: 'http://127.0.0.1:19789',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/mcp-api/, ''),
      },
    },
  },
  clearScreen: false,
  // `tauri` env var is set by Tauri CLI during `cargo tauri dev`. We don't
  // gate behavior on it yet, but keep it referenced so future code can use
  // `import.meta.env.TAURI_ENV_*` without additional setup.
  envPrefix: ['VITE_', 'TAURI_ENV_'],
})
