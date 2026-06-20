# Palmier Pro — Windows Port (Tauri v2 + Rust)

This folder contains the Windows port of [palmier-io/palmier-pro](https://github.com/palmier-io/palmier-pro),
a macOS Swift video editor for AI. The port targets **Tauri v2 + Rust** on
Windows first, with macOS / Linux to follow once the editor surface is
ported.

## Layout

```
app/
├── frontend/                 # Web frontend (Tauri webview). Empty placeholder —
│                             # the React/TS UI lands in a follow-up PR.
└── src-tauri/                # Rust backend + Tauri shell
    ├── Cargo.toml            # Crate manifest. `tauri-shell` feature flag
    │                         # gates the GUI build so `cargo build` works on
    │                         # headless machines without webkit2gtk.
    ├── build.rs              # Tauri build script (no-op without the feature)
    ├── tauri.conf.json       # Tauri v2 app config (window size, bundle, etc.)
    ├── capabilities/         # Tauri v2 capability files (default.json)
    └── src/
        ├── lib.rs            # Library entry (exports `mcp` module)
        ├── logging.rs        # `tlog_err!` / `tlog_info!` macros + tracing init
        ├── main.rs           # `palmier-pro-windows` binary — Tauri GUI shell
        ├── bin/
        │   └── mcp_server.rs # `mcp-server` binary — standalone MCP HTTP server
        └── mcp/
            ├── mod.rs        # MCP module entry
            ├── protocol.rs   # JSON-RPC 2.0 envelope + MCP result types
            ├── instructions.rs # `initialize` instructions string (verbatim port)
            ├── tools.rs      # Tool list & input schemas (port of ToolDefinitions.swift)
            ├── resources.rs  # `resources/list` + `resources/read` (static stubs)
            └── server.rs     # `McpServer` — axum HTTP server on 127.0.0.1:19789
```

## Build

```sh
# Default build — MCP server only (no system webview required).
cargo build

# Full Tauri shell — requires webkit2gtk-4.1 (Linux), WebView2 (Windows),
# or WebKit (macOS).
cargo build --features tauri-shell
cargo tauri dev   # opens the empty Tauri window
```

## Run

```sh
# Standalone MCP HTTP server (for headless testing).
cargo run --bin mcp-server
# -> listens on http://127.0.0.1:19789/mcp

# Full Tauri app.
cargo run --features tauri-shell --bin palmier-pro-windows
```

## MCP endpoint contract

Locked to match the macOS Swift build (see
`Sources/PalmierPro/Agent/MCP/MCPHTTPServer.swift`):

| Method | Path                                       | Behavior                                            |
|--------|--------------------------------------------|-----------------------------------------------------|
| GET    | `/.well-known/oauth-protected-resource`    | RFC 9728 metadata blob                              |
| GET    | `/mcp` or `/`                              | SSE keep-alive stream (initial `: connected`)       |
| POST   | `/mcp` or `/`                              | JSON-RPC 2.0 request/response                       |
| *      | other paths                                | 404 Not Found                                       |

Server info: `{"name": "palmier-pro", "version": "1.0.0"}`.

JSON-RPC methods supported:

- `initialize` → returns protocol version `2025-06-18`, server capabilities,
  and the verbatim `instructions` string from `AgentInstructions.swift`.
- `notifications/initialized` → acknowledged with `202 Accepted`, no body.
- `ping` → returns `{}`.
- `tools/list` → 31 tools matching `ToolDefinitions.swift` exactly.
- `tools/call` → dispatches to `tools::dispatch_call`. Until the editor
  backend is ported, every call returns a tool-level `isError: true` result
  with an "editor backend not yet wired up" message — this matches the
  Swift fallback `ToolResult.error("Editor not available")`.
- `resources/list` → 2 resources (`palmier://models/video`,
  `palmier://models/image`).
- `resources/read` → returns `[]` for known URIs (catalog content lands in a
  follow-up PR), or an "Unknown resource" text block otherwise.

## Manual test

```sh
# Start the server in one terminal:
cargo run --bin mcp-server

# In another terminal:
curl -s http://127.0.0.1:19789/.well-known/oauth-protected-resource
# -> {"resource":"http://127.0.0.1:19789"}

curl -s -X POST http://127.0.0.1:19789/mcp \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | jq .

curl -s -X POST http://127.0.0.1:19789/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | jq '.result.tools | length'
# -> 31
```

## Port status

- [x] Tauri v2 project scaffold (`app/src-tauri/`)
- [x] MCP HTTP server (`mcp::server`) — port of `MCPHTTPServer.swift` +
      `MCPService.swift`
- [x] Tool list & schemas (`mcp::tools`) — port of `ToolDefinitions.swift`
- [x] Server instructions (`mcp::instructions`) — verbatim port of
      `AgentInstructions.serverInstructions`
- [x] Static resources (`mcp::resources`) — URIs match the Swift build;
      catalog content returns `[]` until the model catalog is ported
- [ ] Tool execution (`mcp::tools::dispatch_call`) — currently returns an
      "editor backend not yet wired up" error. A follow-up PR will port
      `ToolExecutor.swift` once the timeline editor lands.
- [ ] Tauri GUI shell — structure is in place but the actual webview needs
      `--features tauri-shell` and a system webview (webkit2gtk-4.1 on Linux).
- [ ] Frontend (`app/frontend/`) — empty placeholder, will host the React/TS
      timeline editor in a follow-up PR.
