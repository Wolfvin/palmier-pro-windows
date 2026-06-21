//! Tauri commands exposed to the frontend.
//!
//! These commands run in the Rust process (same process as the MCP HTTP
//! server), so they can perform HTTP calls to `127.0.0.1:19789` without
//! cross-origin restrictions. The webview's `fetch()` would be blocked by
//! the browser's same-origin policy in production (where the webview origin
//! is `tauri://localhost` / `https://tauri.localhost` / `http://tauri.localhost`
//! — all cross-origin to `http://127.0.0.1:19789`), because the MCP server
//! contract is locked and intentionally does NOT send CORS headers.

use serde::Serialize;
use std::time::Duration;

use crate::tlog_err;

/// MCP server base URL — matches the contract locked by `mcp::server`.
///
/// Marked `allow(dead_code)` because this is only referenced by
/// `probe_oauth_protected_resource` (which is behind `cfg(tauri-shell)`),
/// so the default headless build sees it as unused.
#[allow(dead_code)]
const MCP_BASE_URL: &str = "http://127.0.0.1:19789";

/// RFC 9728 OAuth protected-resource metadata path.
#[allow(dead_code)]
const OAUTH_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";

/// Result of probing the MCP server's `.well-known/oauth-protected-resource`
/// endpoint from the Rust side.
///
/// Serialized to JSON and returned to the frontend via `invoke()`. The
/// frontend uses `status` / `body` / `error` to update the MCP status card.
#[derive(Debug, Serialize)]
pub struct ProbeResult {
    /// HTTP status code (e.g. 200, 404). `None` if the request never reached
    /// the server (connection refused, timeout, DNS failure, …).
    pub status: Option<u16>,
    /// Response body as a UTF-8 string. `None` if the request failed before
    /// receiving a body, or if the body was not valid UTF-8.
    pub body: Option<String>,
    /// Error message when the probe failed (connection refused, timeout, …).
    /// `None` on a successful HTTP response (even a non-2xx one — the
    /// frontend can inspect `status` for that).
    pub error: Option<String>,
}

/// Core HTTP probe logic — performs a GET to the MCP server's
/// `.well-known/oauth-protected-resource` endpoint using `reqwest`.
///
/// This is the shared implementation used by both the Tauri command
/// ([`probe_oauth_protected_resource`]) and the integration test
/// (`test_probe_against_real_mcp_server`). Extracting it as a standalone
/// async function allows testing the HTTP logic without a `tauri::AppHandle`.
///
/// Marked `allow(dead_code)` because in the default headless build (without
/// `tauri-shell`), the Tauri command is not compiled, so `do_probe` is only
/// referenced by tests. The `tauri-shell` build uses it via the command.
///
/// # Returns
///
/// [`ProbeResult`] with the HTTP status, body, and/or error. Never panics —
/// all error paths are converted to `ProbeResult.error`.
#[allow(dead_code)]
async fn do_probe() -> ProbeResult {
    let url = format!("{MCP_BASE_URL}{OAUTH_RESOURCE_PATH}");

    // Build a reqwest client with a short timeout — the MCP server is
    // local-loopback, so 3 seconds is generous even if the server is still
    // booting. A shorter timeout means the status card updates faster on
    // first launch when the server hasn't started yet.
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            tlog_err!("mcp_probe", "failed to build reqwest client", err);
            return ProbeResult {
                status: None,
                body: None,
                error: Some(format!("failed to build HTTP client: {err}")),
            };
        }
    };

    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(err) => {
            // Connection refused / timeout / etc. — this is the expected
            // state when the MCP server hasn't started yet. Log at debug
            // level to avoid spamming logs on every retry tick.
            tracing::debug!(
                "[mcp_probe] failed to reach MCP server at {}: {}",
                url,
                err
            );
            return ProbeResult {
                status: None,
                body: None,
                error: Some(format!("{err}")),
            };
        }
    };

    let status = response.status().as_u16();

    // Read the body as text. If the body is not valid UTF-8, surface the
    // lossy conversion so the frontend still has something to display.
    let body = match response.text().await {
        Ok(text) => Some(text),
        Err(err) => {
            tlog_err!("mcp_probe", "failed to read response body", err);
            None
        }
    };

    ProbeResult {
        status: Some(status),
        body,
        error: None,
    }
}

// ─── Tauri command (only compiled with --features tauri-shell) ──────────────

/// Probes the MCP server's `.well-known/oauth-protected-resource` endpoint.
///
/// This is a Tauri command (not a regular function) — it is registered in
/// `tauri::generate_handler!` and invoked from the frontend via
/// `@tauri-apps/api/core invoke('probe_oauth_protected_resource', …)`.
///
/// The fetch executes in the Rust process (same process as the MCP server),
/// so there is no cross-origin restriction. This closes issue #20: in
/// production, the Tauri webview origin (`tauri://localhost` /
/// `https://tauri.localhost` / `http://tauri.localhost`) is cross-origin to
/// `http://127.0.0.1:19789`, and the MCP server contract does NOT send CORS
/// headers — so a browser `fetch()` would be blocked. By routing through
/// this Rust command, the fetch is same-process and bypasses the browser's
/// same-origin policy entirely.
///
/// # Arguments
///
/// * `_app` — Tauri app handle (unused, but required by the command signature).
///
/// # Returns
///
/// [`ProbeResult`] with the HTTP status, body, and/or error. Never panics —
/// all error paths are converted to `ProbeResult.error`.
#[cfg(feature = "tauri-shell")]
#[tauri::command]
pub async fn probe_oauth_protected_resource(
    _app: tauri::AppHandle,
) -> Result<ProbeResult, ()> {
    Ok(do_probe().await)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test: starts a real MCP server on port 19789, then calls
    /// `do_probe()` (the shared HTTP logic the Tauri command delegates to).
    ///
    /// This verifies the probe correctly:
    /// 1. Connects to a running MCP server
    /// 2. Receives HTTP 200
    /// 3. The response body contains a `resource` field (RFC 9728)
    ///
    /// Note: The probe hardcodes `MCP_BASE_URL = "http://127.0.0.1:19789"`,
    /// so this test starts the server on that port. If port 19789 is already
    /// in use (e.g. another test or a running Palmier Pro instance), the
    /// test skips gracefully.
    #[tokio::test]
    async fn test_probe_against_real_mcp_server() {
        // Start the MCP server on the default port (19789). If the port is
        // already in use, skip the test.
        let server = match crate::mcp::McpServer::start(crate::mcp::McpServerConfig::default())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[test] skipping probe test — MCP server port 19789 unavailable: {e}");
                return;
            }
        };

        // Give the server a moment to start accepting connections.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let result = do_probe().await;

        assert_eq!(result.status, Some(200), "expected HTTP 200; got result: {result:?}");
        assert!(result.error.is_none(), "expected no error; got: {:?}", result.error);
        assert!(result.body.is_some(), "expected body to be present");

        let body = result.body.unwrap();
        assert!(
            body.contains("resource"),
            "response body should contain 'resource' field; got: {body}"
        );

        // Parse as JSON to verify the shape.
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("response should be valid JSON");
        assert!(
            parsed.get("resource").is_some(),
            "response JSON should have a 'resource' key; got: {parsed}"
        );

        server.shutdown().await;
    }

    /// Unit test: `ProbeResult` serializes correctly with all fields present.
    #[test]
    fn test_probe_result_serialize_full() {
        let result = ProbeResult {
            status: Some(200),
            body: Some(r#"{"resource":"http://127.0.0.1:19789"}"#.to_string()),
            error: None,
        };
        let json = serde_json::to_string(&result).expect("failed to serialize");
        assert!(json.contains("\"status\":200"));
        assert!(json.contains("\"body\":"));
        assert!(json.contains("\"error\":null"));
    }

    /// Unit test: `ProbeResult` serializes correctly when the probe failed
    /// (no status, no body, error message present).
    #[test]
    fn test_probe_result_serialize_error() {
        let result = ProbeResult {
            status: None,
            body: None,
            error: Some("connection refused".to_string()),
        };
        let json = serde_json::to_string(&result).expect("failed to serialize");
        assert!(json.contains("\"status\":null"));
        assert!(json.contains("\"body\":null"));
        assert!(json.contains("\"error\":\"connection refused\""));
    }
}
