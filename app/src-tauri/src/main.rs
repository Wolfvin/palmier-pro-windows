//! Palmier Pro — Windows port entry point.
//!
//! When the `tauri-shell` feature is enabled, this binary launches the Tauri
//! v2 GUI shell (an empty window per the DoD). When the feature is disabled
//! (the default, used by headless CI / containers without a system webview),
//! the binary prints a helpful message and exits — use the `mcp-server`
//! binary in that case to run the MCP HTTP server standalone.

#[cfg(feature = "tauri-shell")]
fn main() {
    use palmier_pro_windows_lib::commands::probe_oauth_protected_resource;
    use palmier_pro_windows_lib::logging;
    use palmier_pro_windows_lib::mcp::{McpServer, McpServerConfig};

    logging::init_tracing();

    // Bootstrap the MCP HTTP server alongside the GUI. The Swift version
    // starts the server when `MCPService.start()` is called from the
    // AppDelegate; on the Windows port we start it eagerly from main() for
    // the initial shell so that Claude / Cursor can connect as soon as the
    // window opens. A follow-up PR will gate this behind the same
    // "MCP enabled" preference as the macOS build.
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![probe_oauth_protected_resource])
        .setup(|app| {
            let rt = tokio::runtime::Runtime::new().map_err(|err| {
                eprintln!("[app] failed to create tokio runtime: {err}");
                err
            })?;
            rt.block_on(async move {
                let _server = McpServer::start(McpServerConfig::default()).await;
                // Hold the runtime alive for the lifetime of the app by parking it.
                // The Tauri event loop will exit when the user closes the window.
                std::future::pending::<()>().await;
            });
            // The runtime is leaked intentionally — the app lives until exit.
            // (Returning here is unreachable due to `pending`, but Tauri's
            //  `setup` signature requires the closure to return.)
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(not(feature = "tauri-shell"))]
fn main() {
    use palmier_pro_windows_lib::logging;
    logging::init_tracing();

    eprintln!("[app] palmier-pro-windows: tauri-shell feature not enabled");
    eprintln!("[app] build with --features tauri-shell to launch the GUI (requires");
    eprintln!("[app] webkit2gtk-4.1 on Linux / WebView2 on Windows / WebKit on macOS).");
    eprintln!("[app] ");
    eprintln!("[app] To run the MCP HTTP server standalone for testing:");
    eprintln!("[app]   cargo run --bin mcp-server");
    eprintln!("[app] ");
    eprintln!("[app] Then point your MCP client at http://127.0.0.1:19789/mcp");
}
