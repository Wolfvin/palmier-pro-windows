//! MCP HTTP server module.
//!
//! Port of `Sources/PalmierPro/Agent/MCP/MCPHTTPServer.swift` and
//! `MCPService.swift`. Exposes an idiomatic Rust API around an
//! `axum` + `tokio` HTTP server that listens on `127.0.0.1:19789/mcp` and
//! speaks the MCP Streamable HTTP transport (JSON-RPC 2.0 over HTTP POST,
//! SSE keep-alive on GET).
//!
//! The server is intentionally split into small submodules so that future
//! workers can extend it without re-reading the whole file:
//!
//! - [`protocol`] — JSON-RPC 2.0 envelope + MCP method result types.
//! - [`tools`] — Tool list and input schemas, ported from
//!   `ToolDefinitions.swift`.
//! - [`instructions`] — The `instructions` string returned by
//!   `initialize`, ported from `AgentInstructions.swift`.
//! - [`resources`] — Static `resources/list` + `resources/read` handlers
//!   (video / image model catalogs). For now these return an empty catalog
//!   with the correct shape — the actual model catalog will land in a
//!   follow-up PR once the generation backend is ported.
//! - [`server`] — The `McpServer` type that owns the listener task and
//!   wires the router together.

pub mod instructions;
pub mod protocol;
pub mod resources;
pub mod server;
pub mod tools;

pub use server::{McpServer, McpServerConfig};
