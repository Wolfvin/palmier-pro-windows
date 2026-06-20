//! Standalone MCP HTTP server binary.
//!
//! Runs [`McpServer`] without the Tauri GUI shell so the MCP endpoint
//! contract can be tested in headless environments (CI containers, dev
//! shells without webkit2gtk). Usage:
//!
//! ```sh
//! cargo run --bin mcp-server
//! # -> listens on http://127.0.0.1:19789/mcp
//! ```
//!
//! Then verify with curl:
//!
//! ```sh
//! curl -s http://127.0.0.1:19789/.well-known/oauth-protected-resource
//! curl -s -X POST http://127.0.0.1:19789/mcp \
//!   -H 'content-type: application/json' \
//!   -H 'accept: application/json, text/event-stream' \
//!   -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
//! ```

use palmier_pro_windows_lib::logging;
use palmier_pro_windows_lib::mcp::{McpServer, McpServerConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    logging::init_tracing();

    let server = McpServer::start(McpServerConfig::default()).await?;
    eprintln!("[mcp-server] listening on http://127.0.0.1:{}/mcp", server.local_addr.port());
    eprintln!("[mcp-server] press Ctrl-C to stop");

    // Wait for Ctrl-C, then shut down gracefully.
    match tokio::signal::ctrl_c().await {
        Ok(()) => eprintln!("[mcp-server] received SIGINT, shutting down"),
        Err(err) => eprintln!("[mcp-server] signal handler error: {err}"),
    }
    server.shutdown().await;
    Ok(())
}
