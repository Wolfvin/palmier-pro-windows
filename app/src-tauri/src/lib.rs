//! Palmier Pro — Windows port (Tauri v2 + Rust).
//!
//! This crate exposes the MCP HTTP server as a library so that both the Tauri
//! GUI shell (`palmier-pro-windows` binary) and the standalone test binary
//! (`mcp-server`) can share the same implementation.
//!
//! The MCP server is a port of the Swift `MCPHTTPServer.swift` +
//! `MCPService.swift` pair found under `Sources/PalmierPro/Agent/MCP/`. The
//! HTTP endpoint contract is locked and matches the macOS version exactly:
//!
//! - `127.0.0.1:19789` (IPv4 loopback only — never reachable from the LAN)
//! - `POST /mcp` — JSON-RPC 2.0 request/response (Streamable HTTP transport)
//! - `GET /mcp` — Server-Sent Events keep-alive stream (initial `: connected`)
//! - `GET /.well-known/oauth-protected-resource` — RFC 9728 metadata blob
//! - `GET /` — alias of `/mcp`
//!
//! The tool list & schemas exposed via `tools/list` are ported 1:1 from
//! `ToolDefinitions.swift` so that Claude / Codex / Cursor clients already
//! configured against the macOS build keep working when pointed at the
//! Windows port.

pub mod commands;
pub mod editor;
pub mod generation;
pub mod logging;
pub mod mcp;

pub use mcp::{McpServer, McpServerConfig};
