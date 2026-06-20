# Palmier Pro — Windows (Frontend)

React + TypeScript + Vite frontend for the Tauri v2 shell.

## Layout

```
app/frontend/
├── index.html               # Root HTML, sets <title>
├── package.json             # npm scripts + deps
├── vite.config.ts           # Vite config — port 1420, watch ignore src-tauri
├── tsconfig*.json           # TS configs (app, node, root)
├── eslint.config.js         # ESLint flat config (Vite default)
└── src/
    ├── main.tsx             # React entry — mounts <App> into #root
    ├── App.tsx              # Single page — title + MCP status card
    ├── App.css              # Styles for the MCP status card
    └── index.css            # Base styles (dark mode, font stack)
```

## Scripts

| Command           | Description                                                       |
|-------------------|-------------------------------------------------------------------|
| `npm run dev`     | Start Vite dev server on `http://127.0.0.1:1420`.                 |
| `npm run build`   | Type-check + bundle to `dist/`. Outputs `index.html` + 2 assets.  |
| `npm run preview` | Preview the production build locally.                             |
| `npm run lint`    | Run ESLint.                                                       |

## What the shell shows

A single page with:

- `<h1>Palmier Pro — Windows</h1>` heading.
- Subtitle: "Tauri v2 + Rust shell — MCP server status".
- MCP status card that fetches
  `http://127.0.0.1:19789/.well-known/oauth-protected-resource` on mount
  and displays:
  - 🟡 Connecting (initial probe in flight)
  - 🟢 Connected (with the resource URL)
  - 🔴 Failed (with the error message)
- Auto-retry every 5 seconds while not connected.
- Manual "Retry" button to re-probe on demand.

The fetch target matches the MCP server contract locked by Worker #1
(see `app/src-tauri/src/mcp/server.rs`).

## Tauri integration

`app/src-tauri/tauri.conf.json` is wired so that:

- `devUrl: "http://127.0.0.1:1420"` — Tauri's webview loads from Vite dev.
- `frontendDist: "../frontend/dist"` — Tauri bundles the production build.
- `beforeDevCommand: "npm --prefix ../frontend run dev"` — `cargo tauri dev`
  auto-starts Vite.
- `beforeBuildCommand: "npm --prefix ../frontend run build"` — `cargo tauri
  build` builds the frontend first.

So `cargo tauri dev --features tauri-shell` (from `app/src-tauri/`) is the
one-command way to launch the full app — Tauri will boot Vite, the Rust
shell, and the MCP server together.
